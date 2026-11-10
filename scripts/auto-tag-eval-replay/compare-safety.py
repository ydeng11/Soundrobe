"""Compare executed before/after conditions under a common provider-first oracle.

Run from the repository root with BEFORE AFTER. Neither oracle is rewritten.
"""
import collections
import json
import pathlib
import re
import sys

before, after = map(pathlib.Path, sys.argv[1:3])
load = lambda path: json.loads(path.read_text())
left, right = [load(root / 'snapshot.json') for root in [before, after]]
assert left['locks'] == right['locks'], 'changed inputs, reviewed expectations or model controls'
assert left['budget'] == right['budget'], 'changed request allowances'
assert left['selectionSha256'] == right['selectionSha256'], 'changed safety selection'
assert left['binarySha256'] != right['binarySha256'], 'same executable'
assert left['harnessLocks'] == right['harnessLocks'], 'before/after harness differs'
assert left.get('jobFilter') == right.get('jobFilter'), 'different evaluated jobs'
changed = sorted(file for file in left['sourceLocks'].keys() | right['sourceLocks'].keys()
                 if right['sourceLocks'].get(file) != left['sourceLocks'].get(file))
assert set(changed) <= {'scripts/auto-tag-eval-replay/audit-safety.py',
                        'scripts/auto-tag-eval-replay/run-safety.cjs',
                        'scripts/auto-tag-eval-replay/compare-safety.py',
                        'scripts/auto-tag-eval-replay/test_safety_audit.py',
                        'src-tauri/src/commands/auto_tag.rs',
                        'src-tauri/src/commands/auto_tag_collections_tests.rs',
                        'src-tauri/src/commands/track_matcher.rs'}, changed
audits = [load(root / 'audit.json') for root in [before, after]]
revisions = {audit['oracleRevision'] for audit in audits}
assert len(revisions) == 1 and revisions <= {'provider-first-v2', 'provider-first-v3'}
assert audits[0]['checkerSha256'] == audits[1]['checkerSha256'], 'different independent checkers'
assert all(audit['evaluations'] == snapshot['requestedEvaluations']
           and audit['physicalFileEvaluations'] == snapshot['requestedPhysicalFiles']
           for audit, snapshot in zip(audits, [left, right]))
assert left['requestedEvaluations'] == right['requestedEvaluations']
selection = load(before / 'selection.json')
exchanges, outcomes, fields, model_deltas, violations = [], [], [], [], []
semantic = lambda track: {key: value for key, value in track.items() if key not in ['path', 'sizeBytes', 'bitrate']}
def model_exchanges(inventory):
    normalized = json.loads(json.dumps(inventory['modelRequests']))
    for request in normalized:
        request.pop('sha256')  # Raw hashes remain in each immutable inventory.
        for message in request['body']['messages']:
            if message['role'] != 'user':
                continue
            payload = json.loads(message['content'])
            if 'full_path' in payload:
                payload['full_path'] = re.sub(
                    r'^/private/tmp/soundrobe-auto-tag-eval-[0-9a-f-]{36}/media/(cold|warm)/',
                    '$EVAL_MEDIA/', payload['full_path'])
            message['content'] = payload
    return normalized

for job in selection['jobs']:
    if left.get('jobFilter') and job['name'] not in left['jobFilter']:
        continue
    for profile in selection['profiles']:
        name = f"{job['name']}-{profile}"
        assert load(before / name / 'providers.json') == load(after / name / 'providers.json'), name
        assert load(before / name / 'input.json') == load(after / name / 'input.json'), name
        inventories = [load(root / name / 'inventory.json') for root in [before, after]]
        observations = [collections.Counter(json.dumps(row, sort_keys=True) for row in inventory['requests'])
                        for inventory in inventories]
        delta = dict(added=[json.loads(row) for row in (observations[1] - observations[0]).elements()],
                     removed=[json.loads(row) for row in (observations[0] - observations[1]).elements()])
        if any(delta.values()):
            assert job['name'] == 'collaborator-disagreement', f'{name}: unrelated provider exchanges changed'
            exchanges.append(dict(job=name, **delta))
        models = [model_exchanges(inventory) for inventory in inventories]
        if models[0] != models[1]:
            if not (job.get('providerGenre') and profile == 'folder_filename'
                    and len(models[0]) == 2 and models[1] == []):
                violations.append(f'{name}: unexpected model exchanges changed')
            model_deltas.append(dict(job=name, before=models[0], after=models[1]))
        for phase in ['cold', 'warm']:
            records = [load(root / name / f'{phase}.jsonl') for root in [before, after]]
            assert all(record['payloadUnchanged'] and record['readback'] for record in records)
            assert len(records[0]['readbackTracks']) == len(records[1]['readbackTracks'])
            for index, (original, corrected) in enumerate(zip(*[r['readbackTracks'] for r in records])):
                old, new = semantic(original), semantic(corrected)
                differences = {key: dict(before=old.get(key), after=new.get(key))
                               for key in old.keys() | new.keys() if old.get(key) != new.get(key)}
                if not differences:
                    continue
                assert job['name'] in ['unproven-remaster', 'collaborator-disagreement'], f'{name}: unrelated fields changed'
                if job['name'] == 'collaborator-disagreement':
                    allowed = {'artist', 'artists'} if profile == 'tagged_recovery' else {'album', 'discogsReleaseId', 'artist', 'artists'}
                    if profile == 'folder_filename':
                        allowed.add('genre')  # Explicit user-authorized v2 contract, not a credit/version relaxation.
                    if not set(differences) <= allowed:
                        violations.append(f'{name}/{phase}: unexpected fields {sorted(set(differences) - allowed)}')
                    assert not records[1]['native']['candidate'].get('assisted_genre')
                fields.append(dict(job=name, phase=phase, fileIndex=index, changes=differences))
            outcomes.append(dict(job=name, phase=phase,
                                 beforeWritten=(records[0].get('native') or {}).get('written', 0),
                                 afterWritten=(records[1].get('native') or {}).get('written', 0),
                                 beforeOutcome=records[0]['outcome'], afterOutcome=records[1]['outcome']))
summary = dict(comparableInputs=True, safetyClean=audits[1]['clean'], violations=violations,
               checkerSha256=audits[1]['checkerSha256'],
               frozenInputs=len(left['locks']), changedSources=changed,
               findings=[len(audit['findings']) for audit in audits],
               evaluationScopedFindings=[len({(f['job'], f['phase']) for f in audit['findings'] if 'phase' in f}) for audit in audits],
               jobScopedFindings=[sum('phase' not in f for f in audit['findings']) for audit in audits],
               oracleRevision=selection['oracleRevision'], evaluationsPerCondition=left['requestedEvaluations'],
               physicalFileEvaluationsPerCondition=left['requestedPhysicalFiles'], uniquePhysicalFiles=left['uniquePhysicalFiles'],
               providerRequests=[audit['providerRequests'] for audit in audits],
               scriptedModelRequests=[audit['modelRequests'] for audit in audits],
               groundedAssistedAccuracyCredit=0, exchanges=exchanges, modelDeltas=model_deltas,
               fieldChanges=fields, outcomes=outcomes)
(after / 'comparison.json').write_text(json.dumps(summary, indent=2) + '\n')
print(json.dumps({key: value for key, value in summary.items() if key not in ['exchanges', 'modelDeltas', 'fieldChanges', 'outcomes']}, indent=2))
assert audits[1]['clean'] and not audits[1]['findings'], 'independent safety gate failed'
assert not violations, violations
