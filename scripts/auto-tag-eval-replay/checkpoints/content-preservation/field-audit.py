"""Additional frozen field contract. Does not import application matching code."""
import collections
import hashlib
import importlib.util
import json
import pathlib
import re
import sys
import unicodedata

ROOT = pathlib.Path(__file__).resolve().parent
FIXTURES = pathlib.Path('test/fixtures/tauri/auto-tag-eval')
load = lambda path: json.loads(pathlib.Path(path).read_text())
digest = lambda path: hashlib.sha256(pathlib.Path(path).read_bytes()).hexdigest()


def key(value):
    return ''.join(c for c in unicodedata.normalize('NFC', value or '').casefold() if c.isalnum())


def seconds(value):
    if isinstance(value, (int, float)):
        return value if value > 0 else None
    try:
        total = 0
        for part in value.split(':'):
            total = total * 60 + float(part)
        return total if total > 0 else None
    except (AttributeError, ValueError):
        return None


def names(credits):
    return [re.sub(r' \(\d+\)$', '', credit.get('name') or credit.get('artist', {}).get('name', ''))
            for credit in credits if isinstance(credit, dict)]


def source_tracks(provider, source):
    if isinstance(source.get('tracks'), list):
        return [dict(title=t.get('title'), duration=t.get('length'), artists=t.get('artists') or [t.get('artist')])
                for t in source['tracks']]
    tracks = []
    if provider == 'discogs':
        for track in source.get('tracklist', []):
            if track.get('type_', 'track') != 'track':
                continue
            artists = names(track.get('artists') or source.get('artists', []))
            for extra in track.get('extraartists', []):
                if extra.get('role', '').casefold() in ['featuring', 'guest [with]']:
                    artists.extend(name for name in names([extra]) if name not in artists)
            tracks.append(dict(title=track.get('title'), duration=seconds(track.get('duration')), artists=artists))
    else:
        for medium in source.get('media', []):
            for track in medium.get('tracks', []):
                recording = track.get('recording', {})
                duration = track.get('length') or recording.get('length')
                tracks.append(dict(title=track.get('title') or recording.get('title'),
                                   duration=duration / 1000 if duration else None,
                                   artists=names(track.get('artist-credit') or recording.get('artist-credit') or source.get('artist-credit', []))))
    return tracks


def source_supported(reference, provider, source):
    artist = source.get('artist') or ' & '.join(names(source.get('artists') or source.get('artist-credit', [])))
    if key(artist) not in {key(name) for name in [reference['artist'], *reference['aliases']]}:
        return False
    if key(source.get('title') or source.get('album')) != key(reference['album']):
        return False
    tracks = source_tracks(provider, source)
    if len(tracks) != len(reference['mapping']):
        return False
    used = set()
    for mapping, duration in zip(reference['mapping'], reference['durations']):
        accepted = {key(mapping['providerTitle']), key(mapping['localTitle'])}
        possible = [index for index, track in enumerate(tracks)
                    if index not in used and key(track['title']) in accepted and track['duration']
                    and abs(track['duration'] - duration) <= max(5, duration * .03)]
        if len(possible) != 1:
            return False
        used.add(possible[0])
    return True


def source_genre(source):
    if isinstance(source.get('genre'), str):
        return source['genre'] or None
    return ', '.join(dict.fromkeys(value.strip() for name in ['genres', 'styles']
                                   for value in source.get(name, []) if isinstance(value, str) and value.strip())) or None


def receipts(diagnostics):
    return [receipt for item in diagnostics for receipt in [item, *item.get('fieldEvidence', [])]
            if receipt.get('stage') in ['provider_field', 'provider_track_credits']]


def audit_record(record, sources):
    codes, ledger = [], []
    def require(ok, code):
        if not ok:
            codes.append(code)
    native = record.get('native') or {}
    candidate = native.get('candidate') or {}
    before, after = record.get('beforeTracks', []), record.get('readbackTracks', [])
    require(record.get('assistanceMode') == 'disabled' and not candidate.get('assisted_genre'), 'assistance_boundary')
    require(record.get('payloadUnchanged') and record.get('readback'), 'integrity')
    require(len(before) == len(after), 'binding')
    if not native.get('written'):
        require(record['after'] == record['profileBaseline'], 'held_bytes')
        require(before == after, 'held_tags')
        return codes, ledger
    evidence = receipts(native.get('diagnostics', []))
    persisted = record.get('persistedEvidence') or {}
    require(candidate == persisted.get('candidate'), 'persisted_candidate')
    stored = receipts(persisted.get('diagnostics', []))
    for receipt in evidence:
        require(receipt in stored, 'persisted_receipt')
        require((receipt.get('provider'), receipt.get('releaseId')) in sources, 'source_eligibility')
    genre = next((r for r in evidence if r.get('field') == 'genre'), None)
    if candidate.get('genre'):
        require(bool(genre), 'missing_genre_receipt')
    if genre:
        source = sources.get((genre['provider'], genre['releaseId']))
        if source:
            require(genre['value'] == source_genre(source), 'provider_genre')
    content = candidate.get('verification') == 'album_content' or native.get('completionBasis') == 'album_content'
    require(record.get('reviewPendingAfterRestart') == (content or bool(candidate.get('assisted_genre'))), 'review_state')
    selected_id = candidate.get('musicbrainz_albumid') or candidate.get('discogs_release_id')
    selected_source = sources.get((candidate.get('source'), selected_id))
    if selected_source and source_genre(selected_source):
        require(bool(genre) and genre['provider'] == candidate['source'] and genre['releaseId'] == selected_id,
                'selected_genre_authority')
    for index, (original, actual) in enumerate(zip(before, after)):
        require(pathlib.Path(original['path']).name == pathlib.Path(actual['path']).name, 'binding')
        local = (original.get('genre') or '').strip()
        if local:
            require(original['genre'] == actual.get('genre'), 'local_genre')
        elif genre:
            require(actual.get('genre') == genre['value'], 'provider_genre')
        elif actual.get('genre'):
            require(False, 'unsupported_genre')
        if content:
            require(not any(actual.get(field) for field in ['musicbrainzAlbumId', 'musicbrainzTrackId', 'discogsReleaseId']), 'content_ids')
        else:
            field = 'musicbrainzAlbumId' if candidate.get('source') == 'musicbrainz' else 'discogsReleaseId'
            require(not selected_id or actual.get(field) == selected_id, 'selected_identity')
        for receipt in evidence:
            if receipt.get('stage') != 'provider_track_credits' or receipt.get('localIndex') != index:
                continue
            source = sources.get((receipt['provider'], receipt['releaseId']))
            require(actual.get('artists') == receipt.get('artists'), 'performer_readback')
            if source:
                require(any(key(track['title']) == key(actual.get('title')) and track['artists'] == actual.get('artists')
                            for track in source_tracks(receipt['provider'], source)), 'performer_source')
        ledger.append(dict(file=pathlib.Path(actual['path']).name,
                           original=original, readback=actual,
                           genreAuthority='retained_local' if local else 'provider' if genre else 'absent_assistance_disabled',
                           genreReceipt=genre if not local else None, receipts=evidence,
                           contentOnly=content, reviewPending=record.get('reviewPendingAfterRestart')))
    return codes, ledger


def prepare():
    spec = importlib.util.spec_from_file_location('review', 'scripts/review_auto_tag_subset.py')
    review = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(review)
    batch = load(FIXTURES / 'batch-suite.json')
    manifest = load(ROOT / 'preflight-providers.json')
    detail = [row for row in manifest['records'] if re.fullmatch(r'/release[s]?/[^/]+', row['path']) and row.get('status', 200) == 200]
    certificates = {}
    for item in [c for c in batch['cases'] if c['scored']]:
        corpus = load(pathlib.Path('test/fixtures/tauri') / item['corpus'])
        case = next(c for c in corpus['cases'] if c['caseId'] == item['caseId'])
        rows = load(pathlib.Path('test/fixtures/tauri') / item['expectationSource'])
        row = next(c for c in rows['cases'] if c['caseId'] == item['caseId'])
        expected = review.compare_case(case, row, FIXTURES)
        primary = load(FIXTURES / row['snapshot'])
        reference = dict(artist=case['artist'], album=primary.get('title') or primary.get('album'),
                         aliases=row.get('artistAliases', []), mapping=expected['mapping'],
                         durations=[track['duration'] for track in case['tracks']])
        certified = []
        for source in detail:
            payload = load(ROOT / source['bodyFile'])
            if source_supported(reference, source['provider'], payload):
                certified.append(dict(provider=source['provider'], releaseId=source['path'].rsplit('/', 1)[1],
                                      bodyFile=source['bodyFile'], sha256=source['sha256'],
                                      genre=source_genre(payload)))
        certificates[item['caseId']] = dict(reference=reference, reviewedSource=row, eligibleSources=certified)
    path = ROOT / 'source-certificates.json'
    with path.open('x') as file:
        json.dump(certificates, file, indent=2, ensure_ascii=False)
    print('Pre-output source review:', len(certificates), 'cases,', sum(len(c['eligibleSources']) for c in certificates.values()), 'certificates')


def main():
    selection, certificates = load(ROOT / 'selection.json'), load(ROOT / 'source-certificates.json')
    manifest = load(ROOT / 'providers/manifest.json')
    findings, rows, ledgers = [], [], []
    for case in selection['cases']:
        sources = {(row['provider'], row['releaseId']): load(ROOT / row['bodyFile'])
                   for row in certificates[case['caseId']]['eligibleSources']}
        for profile in selection['profiles']:
            directory = ROOT / profile / case['caseId']
            inventory = load(directory / 'provider-inventory-final.json')
            for phase in ['cold', 'warm']:
                record = load(directory / (phase + '.jsonl'))
                context = dict(caseId=case['caseId'], profile=profile, phase=phase)
                codes, ledger = audit_record(record, sources)
                native = record.get('native') or {}
                evidence = receipts(native.get('diagnostics', []))
                for receipt in evidence:
                    endpoint = ('/releases/' if receipt['provider'] == 'discogs' else '/release/') + receipt['releaseId']
                    if not any(r['provider'] == receipt['provider'] and r['path'] == endpoint and r['status'] == 200 for r in inventory['requests']):
                        codes.append('source_not_fetched')
                for code in sorted(set(codes)):
                    findings.append(dict(**context, category=code))
                rows.append(dict(**context, written=native.get('written', 0), outcome=record['outcome'],
                                 reasonCode=native.get('reasonCode'), error=record.get('error'),
                                 findings=sorted(set(codes)), genreMissing=sum(not t.get('genre') for t in record['readbackTracks']),
                                 genreReceipts=[r for r in evidence if r.get('field') == 'genre'],
                                 cancelled=record.get('cancellationRequested', False)))
                ledgers.append(dict(**context, fields=ledger))
    summary = dict(contract=load(ROOT / 'field-contract.json')['revision'], clean=not findings,
                   evaluations=len(rows), uniquePhysicalFiles=sum(c['tracks'] for c in selection['cases']),
                   physicalFileEvaluations=sum(c['tracks'] * 6 for c in selection['cases']),
                   findings=findings, totals=dict(collections.Counter(f['category'] for f in findings)),
                   groundedAssistanceAccuracyCredit=0, assistanceMode='disabled', cases=rows)
    (ROOT / 'field-verification.json').write_text(json.dumps(summary, indent=2, ensure_ascii=False) + '\n')
    (ROOT / 'field-ledger.json').write_text(json.dumps(ledgers, indent=2, ensure_ascii=False) + '\n')
    print(json.dumps({k:v for k,v in summary.items() if k not in ['cases', 'findings']}, indent=2))
    return 0 if summary['clean'] else 1


if __name__ == '__main__':
    if '--prepare' in sys.argv:
        prepare()
    else:
        sys.exit(main())
