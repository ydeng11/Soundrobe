"""Audit retained mock wiring and readbacks, not musical correctness."""
import collections
import hashlib
import json
import pathlib
import sys

root = pathlib.Path.cwd()
run = pathlib.Path(sys.argv[1])
load = lambda p: json.loads(p.read_text())
status = load(run / 'selection-status.json')
assert status['executionStatus'] == 'complete' and status['scoringStatus'] == 'insufficient'
summary = load(run / 'results.json')
requested = load(run / 'requested-membership.json')
assert (summary['requestedPhysicalOccurrences'], summary['requestedFieldSlots'],
        summary['predeclaredLabelableSlots'], summary['uniqueSelectedOccurrenceFieldLabels']) == (124, 372, 16, 4)
assert len(requested['evaluations']) == 10
assert summary['groundedAssistanceAccuracyCredit'] == 0 and not summary['acceptedCertificate'] and summary['writableDelta'] == []
for lock_file in ['source-and-tool-locks.json', 'fixture-source-locks.json']:
    for binding in load(run / lock_file):
        assert hashlib.sha256((root / binding['path']).read_bytes()).hexdigest() == binding['sha256'], binding['path']
outcomes = collections.Counter()
rows = []
for job in summary['executions']:
    role = job['name']
    directory = run / role
    assert job['projectionStatus'] == 'provisional' and job['exitCode'] == 0 and job['signal'] is None and job['misses'] == 0
    assert job['score']['observedFieldSlots'] == job['score']['requestedFieldSlots']
    outcomes.update(job['score']['outcomes'])
    exported = load(directory / 'bounded/records.json')
    assert [r['phase'] for r in exported] == ['cold', 'warm']
    for phase in ['preparation-disabled', 'bounded']:
        assert load(directory / phase / 'source-hashes-before.json') == load(directory / phase / 'source-hashes-after.json')
    for record in exported:
        assert record['payloadUnchanged'] and record['readback'], (role, record['phase'])
        before = {t['path']: t for t in json.loads(record['fieldObservations']['inputJson'])['tracks']}
        after = {t['path']: t for t in record['fieldObservations']['tracks']}
        assert before.keys() == after.keys()
        if role == 'genre':
            assert record['outcome'] == 'needs_review'
            assert record['reviewPendingAfterRestart'] and record['actionableReviewAfterRestart']
            assert record['persistedEvidence']['aiStatus'] == 'genre_inferred'
            for path, track in before.items():
                assert track['genre'] is None and after[path]['genre'] == 'Scripted diagnostic genre'
                assert (track['title'], track['artists']) == (after[path]['title'], after[path]['artists'])
        elif role in ['cancel', 'unknown-usage']:
            assert record['outcome'] == ('resolver_error' if record['phase'] == 'cold' else 'admission_denied')
            assert record['serviceFailureObserved'] and record['after'] == record['profileBaseline']
        elif role == 'hold':
            assert record['persistedEvidence']['written'] == 0
            assert record['persistedEvidence']['reasonCode'] == 'discovery_budget_limited'
            assert record['after'] == record['profileBaseline']
        rows.append({'role': role, 'phase': record['phase'], 'outcome': record['outcome'],
                     'files': len(before), 'payloadUnchanged': True,
                     'reviewPending': record['reviewPendingAfterRestart'],
                     'actionableReview': record['actionableReviewAfterRestart']})
assert outcomes == {'unscorable': 356, 'unchanged_unknown': 16}
provider = sum(j['providerRequests'] for j in summary['executions'])
model = sum(j['modelRequests'] for j in summary['executions'])
tunnels = sum(j['deniedTunnels'] for j in summary['executions'])
arrivals = sum(j['wireArrivals'] for j in summary['executions'])
assert (provider, model, tunnels, arrivals) == (209, 4, 16, 249)
print(json.dumps({'executionStatus': 'complete', 'scoringStatus': 'insufficient',
    'requestedPhysicalOccurrences': 124, 'requestedFieldSlots': 372, 'observedFieldSlots': 372,
    'replicatedLabelableSlots': 16, 'uniqueSelectedLabels': 4, 'outcomes': outcomes,
    'providerFixtureRequests': provider, 'scriptedModelRequests': model, 'deniedTunnels': tunnels,
    'parsedArrivalsIncludingInventory': arrivals, 'groundedAccuracyCredit': 0,
    'rows': rows, 'sourceAndToolLocksVerified': True}, ensure_ascii=False, indent=2))
