"""Independent paired replay comparison; does not import the evidence exporter."""
import json
import pathlib
import re

HERE = pathlib.Path(__file__).resolve().parent
load = lambda p: json.loads(p.read_text())


def stable(value, path=''):
    if isinstance(value, dict):
        return {k: stable(v, f'{path}/{k}') for k, v in value.items() if path or k != 'elapsedMs'}
    if isinstance(value, list):
        return [stable(v, f'{path}/{i}') for i, v in enumerate(value)]
    if isinstance(value, str):
        return re.sub(r'/private/tmp/soundrobe-auto-tag-eval-[0-9a-f-]{36}', '$DISPOSABLE_ROOT', value)
    return value


def differences(a, b, path=''):
    if type(a) != type(b):
        return [dict(path=path, before=a, after=b)]
    if isinstance(a, dict):
        rows = []
        for key in sorted(a.keys() | b.keys()):
            if key not in a or key not in b:
                rows.append(dict(path=f'{path}/{key}', beforePresent=key in a, afterPresent=key in b))
            else:
                rows.extend(differences(a[key], b[key], f'{path}/{key}'))
        return rows
    if isinstance(a, list):
        if len(a) != len(b):
            return [dict(path=path, beforeLength=len(a), afterLength=len(b))]
        return [row for i, (x, y) in enumerate(zip(a, b)) for row in differences(x, y, f'{path}/{i}')]
    return [] if a == b else [dict(path=path, before=a, after=b)]


def disposable_runtime_fact(row, before, after):
    match = re.fullmatch(r'(/(?:native|persistedEvidence)/diagnostics/\d+/diagnostics/observations/\d+/observation/source)/(?:observedAt|heldFileIdentity/(?:inode|ctimeSeconds|ctimeNanoseconds|mtimeSeconds|mtimeNanoseconds))', row['path'])
    if not match:
        return False
    def get(value, path):
        for part in path.strip('/').split('/'):
            value = value[int(part)] if isinstance(value, list) else value[part]
        return value
    a, b = get(before, match[1]), get(after, match[1])
    diagnostic_path = match[1].split('/diagnostics/observations/')[0]
    batch_path = match[1].removesuffix('/observation/source')
    for record in [before, after]:
        if get(record, diagnostic_path)['stage'] != 'exact_audio_groups':
            return False
        batch = get(record, batch_path)
        observation = batch['observation']
        if (batch['sameInodeAlias'] is not False or observation['status'] != 'complete'
                or observation['error'] is not None or observation['source']['finalizationError'] is not None):
            return False
        source = observation['source']
        if source['observedAt'] < max(source['heldFileIdentity']['ctimeSeconds'], source['heldFileIdentity']['mtimeSeconds']):
            return False
    for record in [before, after]:
        assert record['native']['diagnostics'] == record['persistedEvidence']['diagnostics']
    # These are separately allocated disposable copies. Retain every raw
    # difference, and exempt only allocation/clock facts after byte identity
    # and within-observation stability are established independently.
    return (a['path'] != b['path'] and stable(a['path']) == stable(b['path'])
            and stable(a['path']).startswith('$DISPOSABLE_ROOT/')
            and a['stable'] is True and b['stable'] is True
            and len({a['expectedSha256'], a['beforeSha256'], a['afterSha256'],
                     b['expectedSha256'], b['beforeSha256'], b['afterSha256']}) == 1
            and re.fullmatch('[a-f0-9]{64}', a['expectedSha256']) is not None
            and a['bytes'] == b['bytes']
            and a['heldFileIdentity']['device'] == b['heldFileIdentity']['device']
            and all(type(row[k]) is int and row[k] >= 0 for k in ['before', 'after']))


rows, changes, runtime_facts = [], [], []
for job in load(HERE / 'before/selection.json')['jobs']:
    relative = pathlib.Path(job['profile']) / job['caseId']
    before, after = HERE / 'before' / relative, HERE / 'after' / relative
    for phase in ['cold', 'warm']:
        a = load(before / f'{phase}.jsonl')
        b = load(after / f'{phase}.jsonl')
        diff = differences(stable(a), stable(b))
        for row in diff:
            target = runtime_facts if disposable_runtime_fact(row, a, b) else changes
            target.append(dict(caseId=job['caseId'], profile=job['profile'], phase=phase, **row))
        assert a['payloadUnchanged'] and b['payloadUnchanged'] and a['readback'] and b['readback']
        rows.append(dict(caseId=job['caseId'], profile=job['profile'], phase=phase,
                         tracks=len(a['beforeTracks']), written=(a.get('native') or {}).get('written', 0),
                         rawDifferences=len(diff), unexpectedDifferences=sum(not disposable_runtime_fact(r, a, b) for r in diff),
                         payloadUnchanged=True, readback=True))
        for stem in ['donors', 'requests']:
            filename = f'{stem}-{phase}-{job["caseId"]}.json'
            diff = differences(stable(load(before / filename)), stable(load(after / filename)))
            changes.extend(dict(artifact=filename, profile=job['profile'], **r) for r in diff)
    for filename in ['inventory.json', 'execution.json']:
        diff = differences(stable(load(before / filename)), stable(load(after / filename)))
        changes.extend(dict(artifact=filename, profile=job['profile'], caseId=job['caseId'], **r) for r in diff)
result = dict(rows=rows, differences=changes, expectedDisposableRuntimeFacts=runtime_facts,
              normalization=['disposable UUID prefix', 'elapsedMs measurement only'],
              runtimeFactBoundary='only separately allocated disposable inode/ctime/mtime and observation clock; same source hash, bytes, device, relative path and stable held-file observations required',
              originalMediaAccessed=False, sourceMediaWrites=0,
              groundedAssistanceAccuracyCredit=0,
              scope='paired native outputs and all observed provider/model exchanges; not a clean tagging-safety verdict')
(HERE / 'semantic-comparison.json').write_text(json.dumps(result, indent=2) + '\n')
print(json.dumps(dict(evaluations=len(rows), physicalInputEvaluations=sum(r['tracks'] for r in rows),
                     writtenTrackReadbacks=sum(r['written'] for r in rows), expectedRuntimeFacts=len(runtime_facts), differences=len(changes))))
raise SystemExit(bool(changes))
