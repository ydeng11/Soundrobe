"""Independent fixed-denominator audit for run-paired.cjs; no network or media writes."""
import hashlib
import importlib.util
import json
import re
import sys
from pathlib import Path

spec = importlib.util.spec_from_file_location('titles', Path(__file__).with_name('score-best-aligned.py'))
titles = importlib.util.module_from_spec(spec)
spec.loader.exec_module(titles)
load = lambda file: json.loads(Path(file).read_text())
sha = lambda file: hashlib.sha256(Path(file).read_bytes()).hexdigest()


def audit_phase(context, records, case, truth, review, expected, require_no_write=False):
    row = dict(**context, recovery='unavailable', readbackFailures=[], safetyFailures=[],
               missingFixtures=0, processFailure=False, skipped=False, unavailable=False,
               requiredNoWrite=require_no_write)
    if len(records) != 1 or any(records[0].get(k) != v for k, v in context.items()):
        return row | dict(unavailable=True, reason='missing, duplicate or misbound native phase')
    record = records[0]
    native = record.get('native') or {}
    row.update(outcome=record['outcome'], classification=record.get('classification'),
               written=native.get('written', 0), elapsedMs=record.get('elapsedMs'))
    if require_no_write and native.get('written'):
        row['safetyFailures'].append('required no-write control wrote metadata')
        row['recovery'] = 'wrong_match'
    if record.get('error') or record.get('serviceFailureObserved') or record['outcome'] not in ['applied', 'needs_review']:
        row.update(unavailable=True, reason=record.get('error') or 'incomplete provider/resolver work')
    if record.get('providerMode') != 'offline_fixtures' or record.get('mediaMode') != 'synthetic_flac':
        row['safetyFailures'].append('not offline synthetic replay')
    if not record.get('readback'):
        row['safetyFailures'].append('native write/readback mismatch')
    if not record.get('payloadUnchanged'):
        row['safetyFailures'].append('native payload mismatch')
    baseline, after = record.get('profileBaseline', {}), record.get('after', {})
    expected_files = [t['relativePath'] for t in case.get('tracks', [])]
    track_exports = {}
    for name in ['beforeTracks', 'readbackTracks']:
        exported = record.get(name, [])
        if not isinstance(exported, list) or not all(isinstance(t, dict) and isinstance(t.get('path'), str) for t in exported):
            exported = []
        track_exports[name] = exported
        if len(exported) != len(expected_files) or any(
                Path(track['path']).parts[-len(Path(file).parts):] != Path(file).parts
                for track, file in zip(exported, expected_files)):
            row['safetyFailures'].append(f'{name}: incomplete or misbound physical files')
            row['unavailable'] = True
            if name == 'readbackTracks':
                row['readbackFailures'].append('unavailable independent physical readback')
    for name in ['profileBaseline', 'after', 'copiedBeforeProfile']:
        if set(record.get(name, {})) != set(expected_files):
            row['safetyFailures'].append(f'{name}: incomplete or misbound physical hashes')
            row['unavailable'] = True
    if not baseline or baseline.keys() != after.keys():
        row['safetyFailures'].append('missing physical hash exports')
    for file, before in baseline.items():
        current = after.get(file)
        copied = record.get('copiedBeforeProfile', {}).get(file)
        if not all(isinstance(x, dict) and x.get('payloadSha256') for x in [before, current, copied]):
            row['safetyFailures'].append(f'{file}: unavailable independent payload hash')
        elif not before['payloadSha256'] == current['payloadSha256'] == copied['payloadSha256']:
            row['safetyFailures'].append(f'{file}: changed payload')
    tracks = track_exports['readbackTracks']
    changed_files = {file for file in expected_files
                     if isinstance(baseline.get(file), dict) and isinstance(after.get(file), dict)
                     and baseline[file].get('fullSha256') and after[file].get('fullSha256')
                     and baseline[file]['fullSha256'] != after[file]['fullSha256']}
    if not native.get('written') and (after != baseline or tracks != record.get('beforeTracks')):
        row['safetyFailures'].append('unconfirmed write changed media or metadata')
        row['unavailable'] = True
    if not native.get('written') and not changed_files:
        if row['recovery'] != 'wrong_match':
            row['recovery'] = 'abstention' if not row['unavailable'] else 'unavailable'
        return row
    if changed_files and require_no_write and not native.get('written'):
        row['safetyFailures'].append('required no-write control changed media')
    mappings = {m['localTrack']: m for m in truth['mapping']}
    content_failures = []
    if native.get('written') and (len(tracks) != len(case['tracks']) or native['written'] != len(tracks)):
        row['readbackFailures'].append('incomplete physical-file write count')
        row['unavailable'] = True
    for index, track in enumerate(tracks[:len(case['tracks'])], 1):
        file = expected_files[index - 1]
        if (Path(track['path']).parts[-len(Path(file).parts):] != Path(file).parts
                or (not native.get('written') and file not in changed_files)):
            continue
        mapping = mappings[index]
        title = titles.title_outcome(track.get('title'), mapping, case['artist'], context['caseId'])
        if title['status'] != 'correct' or title['prefix']:
            content_failures.append(f'{index}: title {track.get("title")!r}')
        credits = expected['credits'][index - 1]
        if ([titles.key(n) for n in track.get('artists', [])] != [titles.key(n) for n in credits]
                or any(titles.key(n) not in titles.key(track.get('artist') or '') for n in credits)):
            content_failures.append(f'{index}: ordered performer credits')
        if titles.key(track.get('album') or '') != titles.key(expected['album']):
            content_failures.append(f'{index}: album identity')
        if titles.key(track.get('albumArtist') or '') != titles.key(case['artist']):
            content_failures.append(f'{index}: album artist')
        if any(track.get(key) is not None and str(track[key]) in map(str, review.get('hardNegativeIds', []))
               for key in ['musicbrainzAlbumId', 'discogsReleaseId']):
            content_failures.append(f'{index}: reviewed hard-negative readback identity')
        original = record.get('beforeTracks', [])
        before = original[index - 1] if len(original) == len(case['tracks']) else {}
        positions = {name: before.get(name) or fallback for name, fallback in
                     [('trackNumber', index), ('trackTotal', len(case['tracks']))]}
        if any(track.get(k) != v for k, v in positions.items()):
            row['safetyFailures'].append(f'{index}: preserved/fallback numbering')
        if before and any(before.get(k) and track.get(k) != before[k] for k in ['discNumber', 'discTotal']):
            row['safetyFailures'].append(f'{index}: changed disc positions')
    candidate = native.get('candidate') or {}
    identity = candidate.get('musicbrainz_albumid') or candidate.get('discogs_release_id')
    row['selectedIdentity'] = identity
    row['edition'] = 'reviewed_representative' if str(identity) == str(review['releaseId']) else 'unresolved'
    row['reviewedLayoutMatches'] = len(tracks) == len(mappings) and all(
        track.get('trackNumber') == int(mapping['providerTrack'].split('-')[-1])
        and (len(mapping['providerTrack'].split('-')) == 1
             or (track.get('discNumber') or 1) == int(mapping['providerTrack'].split('-')[0]))
        for track, mapping in zip(tracks, truth['mapping']))
    if str(identity) in map(str, review.get('hardNegativeIds', [])):
        content_failures.append('reviewed hard-negative identity')
    if native.get('completionBasis') == 'album_content' and any(
            track.get(k) for track in tracks for k in ['musicbrainzAlbumId', 'musicbrainzTrackId', 'discogsReleaseId']):
        row['safetyFailures'].append('content-only output asserted edition/recording ID')
    row['readbackFailures'].extend(content_failures)
    row['recovery'] = ('wrong_match' if content_failures or (require_no_write and (native.get('written') or changed_files))
                       else 'correct' if native.get('written') and not row['unavailable'] else 'unavailable')
    return row


def summarize(before, after):
    assert [(r['caseId'], r['profile'], r['phase']) for r in before] == [
        (r['caseId'], r['profile'], r['phase']) for r in after], 'different requested membership'
    def counts(rows):
        return dict(correctRecoveries=sum(r['recovery'] == 'correct' and not r['unavailable'] for r in rows),
                    wrongMatches=sum(r['recovery'] == 'wrong_match' for r in rows),
                    abstentions=sum(r['recovery'] == 'abstention' for r in rows),
                    unavailable=sum(r['unavailable'] for r in rows),
                    missingFixtures=sum(bool(r['missingFixtures']) for r in rows),
                    processFailures=sum(r['processFailure'] for r in rows),
                    skipped=sum(r['skipped'] for r in rows),
                    readbackFailures=sum(bool(r['readbackFailures']) for r in rows),
                    safetyFailures=sum(bool(r['safetyFailures']) for r in rows),
                    requiredNoWriteControls=sum(bool(r.get('requiredNoWrite')) for r in rows),
                    passedNoWriteControls=sum(bool(r.get('requiredNoWrite')) and r['recovery'] == 'abstention'
                        and not r['unavailable'] and not r['safetyFailures'] and not r['readbackFailures'] for r in rows),
                    reviewedRepresentatives=sum(r.get('edition') == 'reviewed_representative' for r in rows),
                    unresolvedEditions=sum(r.get('edition') == 'unresolved' for r in rows),
                    reviewedLayouts=sum(bool(r.get('reviewedLayoutMatches')) for r in rows))
    complete = not any(r['unavailable'] for r in before + after)
    improvements = sum(a['recovery'] != 'correct' and b['recovery'] == 'correct'
                       and not a['unavailable'] and not b['unavailable'] for a, b in zip(before, after))
    regressions = sum((a['recovery'] == 'correct' and b['recovery'] != 'correct')
                      or (a['recovery'] != 'wrong_match' and b['recovery'] == 'wrong_match')
                      for a, b in zip(before, after))
    left, right = counts(before), counts(after)
    return dict(requestedEvaluationsPerImplementation=len(before), baseline=left, candidate=right,
                comparable=complete, improvements=improvements, regressions=regressions,
                clean=complete and regressions == 0 and not any(right[k] for k in
                      ['wrongMatches', 'safetyFailures', 'readbackFailures']),
                groundedModelQualityCredit=0)


def reviewed_fields(review, truth, fixtures):
    source = load(fixtures / review['snapshot'])
    remote = []
    if review['provider'] == 'musicbrainz':
        for medium in source['media']:
            for track in medium.get('tracks', []):
                credits = track.get('artist-credit') or track.get('recording', {}).get('artist-credit') or source['artist-credit']
                names = [c.get('name') or c['artist']['name'] for c in credits]
                remote.append((track.get('title') or track['recording']['title'], names))
        album = source['title']
    else:
        clean = lambda name: re.sub(r' \(\d+\)$', '', name)
        for track in source['tracklist']:
            if track.get('type_', 'track') != 'track':
                continue
            credits = track.get('artists') or source['artists']
            names = [clean(c['name']) for c in credits]
            for guest in track.get('extraartists', []):
                if guest['role'].casefold() in ['featuring', 'guest [with]'] and clean(guest['name']) not in names:
                    names.append(clean(guest['name']))
            remote.append((track['title'], names))
        album = source['title']
    credits = []
    for mapping in truth['mapping']:
        options = {tuple(names) for title, names in remote if titles.key(title) == titles.key(mapping['providerTitle'])}
        if len(options) != 1:
            raise ValueError(f'{review["caseId"]}: ambiguous reviewed credits; not eligible for this checker')
        credits.append(list(options.pop()))
    return dict(album=album, credits=credits)


def audit(root):
    snapshot = load(root / 'comparison-inputs.json')
    rows = {}
    failures = list(snapshot['setupFailures'])
    for file, digest in snapshot.get('locks', {}).items():
        if not Path(file).is_file() or sha(file) != digest:
            failures.append(f'changed/missing input: {file}')
    for arm in ['baseline', 'candidate']:
        result = []
        for job in snapshot['jobs']:
            directory = root / arm / job['profile'] / job['caseId']
            evidence_failures = []
            def export(name):
                try:
                    value = load(directory / name)
                    if not isinstance(value, dict) or not value:
                        raise ValueError('expected nonempty object')
                    if name == 'inventory.json':
                        if (not isinstance(value.get('misses'), list) or not isinstance(value.get('modelRequests'), list)
                                or any(not isinstance(m, dict) or type(m.get('count')) is not int or m['count'] < 0
                                       for m in value['misses'])):
                            raise ValueError('invalid miss counts or model request array')
                    elif name == 'execution.json':
                        if ('exitCode' not in value or (value['exitCode'] is not None and type(value['exitCode']) is not int)
                                or 'signal' not in value or (value['signal'] is not None and not isinstance(value['signal'], str))
                                or ('skipped' in value and type(value['skipped']) is not bool)):
                            raise ValueError('invalid exit code, signal or skipped status')
                    else:
                        files = value.get(job['caseId'])
                        if (set(value) != {job['caseId']} or not isinstance(files, dict)
                                or set(files) != {t['relativePath'] for t in job['case']['tracks']}
                                or any(not isinstance(h, dict) or type(h.get('bytes')) is not int or h['bytes'] < 0
                                       or not isinstance(h.get('fullSha256'), str)
                                       or not re.fullmatch(r'[a-f0-9]{64}', h['fullSha256']) for h in files.values())):
                            raise ValueError('invalid source file membership, size or SHA-256')
                    return value
                except (OSError, ValueError) as error:
                    evidence_failures.append(f'{name}: unavailable export ({error})')
                    return {}
            execution = export('execution.json')
            inventory = export('inventory.json')
            source_before = export('source-hashes-before.json')
            source_after = export('source-hashes-after.json')
            phases = {}
            for phase in ['cold', 'warm']:
                file = directory / f'{phase}.jsonl'
                try:
                    phases[phase] = [json.loads(line) for line in file.read_text().splitlines() if line.strip()] if file.exists() else []
                except (ValueError, OSError):
                    phases[phase] = []
                context = dict(caseId=job['caseId'], profile=job['profile'], phase=phase)
                try:
                    row = audit_phase(context, phases[phase], job['case'], job['truth'], job['review'], job['expected'],
                                      require_no_write=job.get('requireNoWrite', False))
                except (KeyError, TypeError, ValueError, IndexError) as error:
                    row = audit_phase(context, [], {}, {}, {}, {}, job.get('requireNoWrite', False)) | dict(
                        reason=f'invalid phase export: {error}')
                row['processFailure'] = (bool(execution) and (execution.get('exitCode') != 0 or execution.get('signal') is not None)
                                         or any(f.startswith('execution.json:') for f in evidence_failures))
                row['skipped'] = not execution or execution.get('skipped', False)
                row['missingFixtures'] = sum(m.get('count', 1) for m in inventory.get('misses', []))
                row['safetyFailures'].extend(evidence_failures)
                if evidence_failures or row['processFailure'] or row['skipped'] or row['missingFixtures'] or not inventory or inventory.get('modelRequests'):
                    row['unavailable'] = True
                    if row['recovery'] != 'wrong_match':
                        row['recovery'] = 'unavailable'
                result.append(row)
            if all(len(phases[p]) == 1 for p in phases):
                cold, warm = [phases[p][0] for p in ['cold', 'warm']]
                if (any(cold.get('cacheBefore', {}).values()) or warm.get('cacheBefore') != cold.get('cacheAfter')
                        or cold.get('profileBaseline') != warm.get('profileBaseline')):
                    for row in result[-2:]:
                        row['safetyFailures'].append('cold/warm cache or initial media isolation')
                def portable(record):
                    tracks = record.get('readbackTracks')
                    if not isinstance(tracks, list) or not all(isinstance(t, dict) for t in tracks):
                        return None
                    return [{k: v for k, v in t.items() if k not in ['path', 'sizeBytes', 'bitrate']} for t in tracks]
                if cold.get('outcome') != warm.get('outcome') or portable(cold) != portable(warm):
                    for row in result[-2:]:
                        row['safetyFailures'].append('cold/warm output drift')
            if execution and (not source_before or not source_after or source_before != source_after):
                for row in result[-2:]:
                    row['safetyFailures'].append('source-media verification missing or changed')
        rows[arm] = result
    report = summarize(rows['baseline'], rows['candidate'])
    report.update(setupFailures=failures, implementations=snapshot['implementations'],
                  budget=snapshot['budget'], inputLockCount=len(snapshot.get('locks', {})), rows=rows,
                  scope='Reviewed metadata content, not acoustic or exact pressing proof; models disabled')
    if failures:
        report.update(clean=False, comparable=False, improvements=0)
    # No credit for an initial-input mismatch between implementations.
    for job in snapshot['jobs']:
        for phase in ['cold', 'warm']:
            files = [root / arm / job['profile'] / job['caseId'] / f'{phase}.jsonl' for arm in rows]
            if all(f.exists() for f in files):
                try:
                    records = [json.loads(f.read_text()) for f in files]
                    if records[0]['profileBaseline'] != records[1]['profileBaseline']:
                        raise ValueError('different profile input bytes')
                except (ValueError, KeyError):
                    failures.append(f'{job["caseId"]}/{job["profile"]}/{phase}: input mismatch or duplicate record')
                    report.update(clean=False, comparable=False, improvements=0)
    (root / 'comparison.json').write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
    lines = ['# Offline paired comparison', '', f'Comparable: {report["comparable"]}; clean candidate gate: {report["clean"]}',
             f'Requested: {len(snapshot["jobs"]) // 3} cases × 3 profiles × 2 phases = {report["requestedEvaluationsPerImplementation"]} per implementation.',
             f'Improvements: {report["improvements"]}; regressions: {report["regressions"]}.', '',
             '| Outcome / check | Baseline | Candidate |', '|---|---:|---:|']
    lines += [f'| {key} | {report["baseline"][key]} | {report["candidate"][key]} |' for key in report['baseline']]
    lines += ['', 'Missing-fixture/process counts above are affected evaluation slots, not distinct routes/processes.', '',
              '## Implementations']
    lines += [f'- {arm}: `{value.get("gitHead", "unavailable")}`, binary `{value.get("binarySha256", "unavailable")}`; dirty source: {value.get("dirtySource", "unknown")}'
              for arm, value in snapshot['implementations'].items()]
    lines += ['', '## Per-profile recoveries', '', '| Profile | Baseline | Candidate |', '|---|---:|---:|']
    for profile in snapshot['profiles']:
        counts = [sum(r['recovery'] == 'correct' and not r['unavailable'] for r in rows[arm] if r['profile'] == profile) for arm in rows]
        lines.append(f'| {profile} | {counts[0]} | {counts[1]} |')
    lines += ['', '## Incomplete setup', *[f'- {failure}' for failure in failures], '', report['scope'],
              'Readback and write-safety failures are separate from content recovery. Abstentions remain in the requested denominator.']
    (root / 'comparison.md').write_text('\n'.join(lines) + '\n')
    print(json.dumps({k: v for k, v in report.items() if k not in ['rows', 'implementations']}, indent=2))
    return 0 if report['clean'] else 1


if __name__ == '__main__':
    if len(sys.argv) == 4 and sys.argv[1] == 'fields':
        job = load(sys.argv[2])
        print(json.dumps(reviewed_fields(job['review'], job['truth'], Path(sys.argv[3]))))
    else:
        sys.exit(audit(Path(sys.argv[1])))
