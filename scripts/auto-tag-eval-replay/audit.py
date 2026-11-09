"""Audit all requested evaluations without hiding abstentions or fixture gaps."""
import collections
import hashlib
import importlib.util
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent
REPO = Path.cwd()
partial = '--partial' in sys.argv
load = lambda path: json.loads(Path(path).read_text())
sha = lambda path: hashlib.sha256(Path(path).read_bytes()).hexdigest()
selection = load(ROOT / 'selection.json')
assert len(selection['cases']) == 51
assert sum(case['tracks'] for case in selection['cases']) == 539
assert sha(selection['binary']) == selection['binaryHash']
for file, expected in selection['locks'].items():
    assert sha(REPO / file) == expected, file
manifest = load(ROOT / 'providers/manifest.json')
for response in manifest['records']:
    assert sha(ROOT / 'providers' / response['bodyFile']) == response['sha256']
spec = importlib.util.spec_from_file_location('titles', ROOT.parent / 'album-content-comparison-20261107/score-best-aligned.py')
rubric = importlib.util.module_from_spec(spec)
spec.loader.exec_module(rubric)
rubric.self_test()
fixtures = REPO / 'test/fixtures/tauri/auto-tag-eval'
review_rows = {case['caseId']: case for file in [fixtures / 'reviewed-subset.json', *sorted(fixtures.glob('reviewed-*-20261108.json'))]
               for case in load(file)['cases'] if case['status'] == 'verified_match'}
assert {case['caseId'] for case in selection['cases']} == set(review_rows)


def route(record):
    query = record.get('query', {})
    return record['provider'], record['path'], tuple(sorted(query.items() if isinstance(query, dict) else map(tuple, query)))


def provider_rows(provider, release):
    rows = []
    # The original reviewed Relapse snapshot is a frozen normalized candidate,
    # not a raw Discogs response. Match its documented receipt schema only.
    if isinstance(release.get('tracks'), list):
        return [(track['title'], track['artist'], tuple(track['artists']))
                for track in release['tracks']]
    if provider == 'musicbrainz':
        for medium in release['media']:
            for track in medium.get('tracks', []):
                credits = track.get('artist-credit') or track.get('recording', {}).get('artist-credit') or release['artist-credit']
                names = [credit.get('name') or credit['artist']['name'] for credit in credits]
                display = ''.join(name + credit.get('joinphrase', '') for name, credit in zip(names, credits))
                for title in {track.get('title'), track.get('recording', {}).get('title')} - {None}:
                    rows.append((title, display, tuple(names)))
    else:
        clean = lambda name: re.sub(r' \(\d+\)$', '', name)
        for track in release['tracklist']:
            if track.get('type_', 'track') != 'track':
                continue
            credits = track.get('artists') or release['artists']
            names = [clean(credit['name']) for credit in credits]
            # Guest [With] explicitly bills a collaborating performer, not a producer.
            for extra in track.get('extraartists', []):
                if extra['role'].casefold() in {'featuring', 'guest [with]'} and clean(extra['name']) not in names:
                    names.append(clean(extra['name']))
            rows.append((track['title'], ' & '.join(names), tuple(names)))
    return rows


rows, missing, integrity, credit_failures, title_failures = [], [], [], [], []
missing_reviewed_collaborators = []
cache_differences, output_differences, classification_differences, redactions = [], [], [], []
fields = ['title', 'artist', 'artists', 'album', 'albumArtist', 'albumArtists', 'year', 'genre',
          'trackNumber', 'trackTotal', 'discNumber', 'discTotal', 'musicbrainzAlbumId',
          'musicbrainzArtistId', 'musicbrainzTrackId', 'discogsReleaseId', 'discogsArtistId']
for job in selection['cases']:
    case = next(case for case in load(job['corpusFile'])['cases'] if case['caseId'] == job['caseId'])
    oracle = next(case for case in load(job['expectations'])['cases'] if case['caseId'] == job['caseId'])
    review = review_rows[job['caseId']]
    reviewed_credits = provider_rows(review['provider'], load(fixtures / review['snapshot']))
    aliases = {rubric.key(name) for name in [case['artist'], *review.get('artistAliases', [])]}
    for profile in selection['profiles']:
        directory = ROOT / profile / job['caseId']
        context = {'caseId': job['caseId'], 'profile': profile}
        if partial and not (directory / 'execution.json').exists():
            continue
        execution = load(directory / 'execution.json')
        assert execution['exitCode'] == 0 and execution['signal'] is None, context
        inventory = load(directory / 'provider-inventory-final.json')
        missing.extend({**context, **entry} for entry in inventory['misses'])
        assert load(directory / 'source-hashes-before.json') == load(directory / 'source-hashes-after.json'), context
        records = {phase: load(directory / (phase + '.jsonl')) for phase in ['cold', 'warm']}
        cold, warm = records['cold'], records['warm']
        assert cold['profileBaseline'] == warm['profileBaseline'], context
        if any(cold['cacheBefore'].values()) or warm['cacheBefore'] != cold['cacheAfter']:
            cache_differences.append(context)
        if cold['classification'] != warm['classification']:
            classification_differences.append({**context, 'cold': cold['classification'], 'warm': warm['classification']})
        if cold['outcome'] != warm['outcome'] or [
            {field: track.get(field) for field in fields} for track in cold['readbackTracks']
        ] != [{field: track.get(field) for field in fields} for track in warm['readbackTracks']]:
            output_differences.append(context)
        for phase, record in records.items():
            context = {'caseId': job['caseId'], 'profile': profile, 'phase': phase}
            assert record['providerMode'] == 'offline_fixtures' and record['mediaMode'] == 'synthetic_flac'
            if not record['readback'] or not record['payloadUnchanged']:
                integrity.append(context)
            for name, after in record['after'].items():
                if after == '[redacted]':
                    redactions.append({**context, 'relativePath': name})
                else:
                    assert after['payloadSha256'] == record['profileBaseline'][name]['payloadSha256'] == record['copiedBeforeProfile'][name]['payloadSha256'], (context, name)
            native = record.get('native') or {}
            tracks = record['readbackTracks']
            row = {**context, 'albumFolder': job['sourceRelativeFolder'], 'outcome': record['outcome'],
                   'classification': record['classification'], 'reasonCode': native.get('reasonCode'),
                   'error': record['error'], 'processExit': execution['exitCode'],
                   'missingRoutesInColdWarmJob': len(inventory['misses']), 'written': native.get('written', 0),
                   'readback': record['readback'], 'payloadUnchanged': record['payloadUnchanged'],
                   'elapsedMs': record['elapsedMs'], 'titleChecks': [], 'creditChecks': []}
            if record['outcome'] == 'applied':
                assert len(tracks) == len(oracle['mapping']) == job['tracks'] == native['written'], context
                row['localOrderNumbering'] = [track['trackNumber'] for track in tracks] == list(range(1, len(tracks) + 1))
                row['localTrackTotal'] = all(track['trackTotal'] == len(tracks) for track in tracks)
                row['varyingAlbumFields'] = [field for field in ['album', 'albumArtist', 'albumArtists', 'year', 'genre', 'trackTotal']
                    if len({json.dumps(track[field], sort_keys=True) for track in tracks}) != 1]
                row['discNumbers'] = list(dict.fromkeys(track['discNumber'] for track in tracks))
                candidate = native['candidate']
                basic = next((entry for entry in native['diagnostics'] if entry.get('stage') == 'album_content'), {})
                provider = candidate['source']
                release_id = basic.get('representativeReleaseId') or candidate.get('musicbrainz_albumid') or candidate.get('discogs_release_id')
                row['representative'] = [provider, release_id]
                endpoint = ('/release/' if provider == 'musicbrainz' else '/releases/') + str(release_id)
                responses = [response for response in manifest['records'] if response['provider'] == provider
                    and response['path'] == endpoint and response.get('status', 200) == 200]
                reached = [response for response in responses if any(route(request) == route(response) and request['status'] == 200 for request in inventory['requests'])]
                assert len(reached) <= 1, context
                release = load(ROOT / 'providers' / reached[0]['bodyFile']) if reached else None
                frozen = provider_rows(provider, release) if release else []
                album_credits = release.get('artist-credit', []) if release else []
                artist_ids = {credit['artist']['id'] for credit in album_credits if credit.get('artist', {}).get('id')}
                album_names = tuple(credit.get('name') or credit['artist']['name'] for credit in album_credits)
                for index, (track, source, mapping) in enumerate(zip(tracks, case['tracks'], oracle['mapping']), 1):
                    assert mapping['localTrack'] == index
                    assert Path(track['path']).name == Path(source['relativePath']).name, context
                    title = rubric.title_outcome(track['title'], mapping, case['artist'], job['caseId'])
                    row['titleChecks'].append({'localTrack': index, 'title': track['title'], **title})
                    if title['status'] != 'correct' or title['prefix']:
                        title_failures.append({**context, **row['titleChecks'][-1], 'expected': mapping})
                    compatible = [(display, names) for title, display, names in frozen
                        if rubric.title_outcome(title, mapping, case['artist'], job['caseId'])['status'] == 'correct']
                    options = set(compatible)
                    credit_scope = 'reviewed_title'
                    if not options:
                        all_credits = {(display, names) for _, display, names in frozen}
                        if len(all_credits) == 1:
                            options = all_credits
                            credit_scope = 'uniform_provider_track_credits'
                    actual = (track['artist'], tuple(track['artists']))
                    status = 'exact' if len(options) == 1 and actual in options else 'unverified'
                    if status == 'unverified' and len(options) == 1:
                        display, names = next(iter(options))
                        if (provider == 'musicbrainz' and len(names) == len(track['artists']) == 1
                            and names == album_names and candidate.get('musicbrainz_artistid') in artist_ids
                            and rubric.key(track['artist'] or '') in aliases and track['artists'] == [track['artist']]):
                            status = 'reviewed_solo_alias_same_provider_identity'
                    credit = {'localTrack': index, 'title': track['title'], 'actualArtist': track['artist'],
                              'actualArtists': track['artists'], 'status': status, 'creditScope': credit_scope,
                              'frozenOptions': sorted(options)}
                    row['creditChecks'].append(credit)
                    if status == 'unverified':
                        credit_failures.append({**context, **credit})
                    reviewed_options = {names for title, _, names in reviewed_credits
                        if rubric.key(title) == rubric.key(mapping['providerTitle'])}
                    if len(reviewed_options) == 1:
                        expected_names = next(iter(reviewed_options))
                        absent = [name for name in expected_names
                            if rubric.key(name) not in {rubric.key(name) for name in track['artists']}]
                        if len(expected_names) > 1 and absent:
                            missing_reviewed_collaborators.append({**context, 'localTrack': index,
                                'title': track['title'], 'missingArtists': absent, 'actualArtists': track['artists'],
                                'selectedRepresentative': row['representative'],
                                'reviewedProvider': review['provider'], 'reviewedReleaseId': review['releaseId'],
                                'reviewedSnapshot': review['snapshot'], 'selectedProviderCreditStatus': status})
                if native.get('completionBasis') == 'album_content':
                    assert all(track['musicbrainzAlbumId'] is None and track['musicbrainzTrackId'] is None and track['discogsReleaseId'] is None for track in tracks), context
            rows.append(row)

if not partial:
    assert len(rows) == 306
    assert len(load(ROOT / 'executions.json')) == 153
totals = {profile: {phase: {
    'evaluations': len(subset := [row for row in rows if row['profile'] == profile and row['phase'] == phase]),
    'outcomes': dict(collections.Counter(row['outcome'] for row in subset)),
    'classifications': dict(collections.Counter(row['classification'] for row in subset)),
    'appliedWithMissingRoutes': sum(row['outcome'] == 'applied' and row['missingRoutesInColdWarmJob'] > 0 for row in subset),
    'correctTitles': sum(check['status'] == 'correct' and not check['prefix'] for row in subset for check in row['titleChecks']),
    'supportedCredits': sum(check['status'] != 'unverified' for row in subset for check in row['creditChecks']),
    'writtenTracks': sum(row['written'] for row in subset),
    'wrongNumberingCases': sum(not row['localOrderNumbering'] or not row['localTrackTotal'] for row in subset if row['outcome'] == 'applied'),
    'nonUniformAlbumCases': sum(bool(row['varyingAlbumFields']) for row in subset if row['outcome'] == 'applied'),
} for phase in ['cold', 'warm']} for profile in selection['profiles']}
result = {
    'scope': 'All 51 reviewed metadata-content positives, three profiles, cold and warm, native resolver and WriteQueue, silent FLACs and frozen HTTP responses; not acoustic identity proof',
    'partialAudit': partial, 'requestedEvaluations': 306,
    'binarySha256': selection['binaryHash'], 'auditSha256': sha(__file__), 'rubricSha256': sha(rubric.__file__),
    'evaluations': len(rows), 'appliedEvaluations': sum(row['outcome'] == 'applied' for row in rows),
    'sourceDummiesUnchanged': 539, 'frozenInputLocks': len(selection['locks']),
    'frozenProviderResponses': len(manifest['records']), 'totals': totals,
    'missingRoutes': missing, 'integrityFailures': integrity, 'titleFailures': title_failures,
    'unsupportedCreditEvidence': credit_failures, 'missingReviewedCollaborators': missing_reviewed_collaborators,
    'coldWarmOutputDifferences': output_differences,
    'coldWarmClassificationDifferences': classification_differences, 'cacheIsolationFailures': cache_differences,
    'redactedIndependentPayloadExports': redactions, 'cases': rows,
}
(ROOT / ('verification-partial.json' if partial else 'verification.json')).write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n')
print(json.dumps({key: value for key, value in result.items() if key not in {'cases', 'missingRoutes', 'unsupportedCreditEvidence', 'titleFailures', 'redactedIndependentPayloadExports'}}, indent=2))
print('Missing provider requests:', len(missing), 'title failures:', len(title_failures), 'unverified credits:', len(credit_failures))
