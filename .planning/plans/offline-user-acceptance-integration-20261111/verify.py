"""Audit two fresh maintained mock runs, not a second evaluator. No audio reads."""
import hashlib
import importlib.util
import json
from pathlib import Path
import sys

ROOT = Path.cwd()
plain, labelled = map(Path, sys.argv[1:])
read = lambda p: json.loads(p.read_text())
digest = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()
module = importlib.util.spec_from_file_location('scorer', ROOT / 'scripts/auto-tag-eval-replay/score-observations.py')
scorer = importlib.util.module_from_spec(module)
module.loader.exec_module(scorer)
labels = read(ROOT / '.planning/plans/crowd-lu-user-acceptance-v1-20261111/labels.json')
source = read(ROOT / 'test/fixtures/tauri/auto-tag-eval/provider-snapshots/review-expansion-2-20261108/musicbrainz-release-7935e252-463d-4207-bce9-594d0a84e63a.json')
payload = {'artist': '盧廣仲', 'album': source['title'],
           'tracks': [t['title'] for medium in source['media'] for t in medium['tracks']]}
checks, jobs, redacted_hash_observations = [], [], []
for run in [plain, labelled]:
    assert read(run / 'selection-status.json')['executionStatus'] == 'complete'
    result = read(run / 'results.json')
    assert result['requestedFieldSlots'] == 372
    assert result['requestedPhysicalOccurrences'] == 124
    assert result['groundedAssistanceAccuracyCredit'] == 0
    assert result['acceptedCertificate'] is False and result['writableDelta'] == []
    checks.append(str(run) + ':complete-membership-zero-credit')
for job in ['provider', 'genre', 'unknown-usage', 'cancel', 'hold']:
    bodies = []
    for run in [plain, labelled]:
        directory = run / job
        inventory = read(directory / 'bounded/inventory-final.json')
        records = read(directory / 'bounded/records.json')
        assert not inventory['misses']
        bodies.append([request['body'] for request in inventory['modelRequests']])
        for request in inventory['modelRequests']:
            body = request['body']
            assert set(body) == {'model', 'temperature', 'max_tokens', 'messages', 'response_format'}
            assert json.loads(body['messages'][1]['content']) == payload
            text = json.dumps(body, ensure_ascii=False)
            assert all(value not in text for value in labels['albumLevel']['acceptedGenreValues'])
            assert all(value not in text for value in ['acceptedGenreValues', 'originalAnnotation',
                'userAcceptance', 'annotations-rev63', 'crowd-lu-user-acceptance'])
        for row in records:
            assert row['payloadUnchanged']
            assert set(row['after']) == set(row['profileBaseline'])
            for path, after in row['after'].items():
                if after == '[redacted]':
                    assert job == 'provider' and path == '02. Cookie Jar.flac'
                    assert row['profileBaseline'][path] == '[redacted]'
                    redacted_hash_observations.append({'run': run.name, 'job': job,
                        'phase': row['phase'], 'path': path, 'independentPostwriteHashCheck': 'unavailable'})
                    continue
                assert after['payloadSha256'] == row['profileBaseline'][path]['payloadSha256']
                if job in ['unknown-usage', 'cancel', 'hold']:
                    assert after['fullSha256'] == row['profileBaseline'][path]['fullSha256']
            if job in ['genre', 'unknown-usage', 'cancel']:
                before = {t['path']: t for t in json.loads(row['fieldObservations']['inputJson'])['tracks']}
                after = {t['path']: t for t in row['fieldObservations']['tracks']}
                assert set(before) == set(after)
                for path in before:
                    assert before[path]['title'] == after[path]['title']
                    assert before[path]['artists'] == after[path]['artists']
            if job in ['unknown-usage', 'cancel']:
                assert row['serviceFailureObserved']
        assert len(records) == 2
        if job in ['unknown-usage', 'cancel']:
            assert records[1]['outcome'] == 'admission_denied'
        assert read(directory / 'bounded/source-hashes-before.json') == read(directory / 'bounded/source-hashes-after.json')
        contract = read(directory / 'scoring-contract.json')
        manifest = directory / 'bounded-manifest.json'
        score = scorer.project_native(ROOT, contract, records,
            {'path': str(manifest), 'sha256': digest(manifest)})
        assert score == read(directory / 'score.provisional.json')
        checks.append(f'{run.name}:{job}:projection-payload-source-protection')
    assert bodies[0] == bodies[1]
    assert len(bodies[0]) == (2 if job == 'genre' else 1 if job in ['unknown-usage', 'cancel'] else 0)
    checks.append(job + ':identical-serialized-requests-with-and-without-labels')
    jobs.append({'name': job, 'scriptedModelRequestsPerRun': len(bodies[0]),
        'outcomes': read(labelled / job / 'score.provisional.json')['outcomes']})
base, added = read(plain / 'results.json'), read(labelled / 'results.json')
assert base['predeclaredLabelableSlots'] == 16
assert added['predeclaredLabelableSlots'] == 82
assert added['uniqueUserAcceptanceLabels'] == 11
assert added['uniqueSelectedOccurrenceFieldLabels'] == 15
assert added['userAcceptance'] == {'requestedFieldSlots': 66, 'observedFieldSlots': 66,
    'agreeingFieldSlots': 0, 'serviceFailureSlots': 44}
print(json.dumps({'checksPassed': len(checks), 'checks': checks, 'jobs': jobs,
    'redactedHashObservations': redacted_hash_observations,
    'userAcceptance': added['userAcceptance'], 'groundedAssistanceAccuracyCredit': 0,
    'scope': 'scripted wiring/preservation evidence, not musical truth or causal AI improvement',
    'runs': [{'path': str(run), 'resultsSha256': digest(run / 'results.json')} for run in [plain, labelled]]},
    ensure_ascii=False, indent=2))
