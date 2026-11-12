"""Recheck retained mock exports without native execution or original-library reads."""
import hashlib
import importlib.util
import json
import pathlib
import sys

root = pathlib.Path.cwd()
run = pathlib.Path(sys.argv[1]).resolve()
assert run.is_relative_to(root / '.amp/in/artifacts')
load = lambda p: json.loads(p.read_text())
module = importlib.util.spec_from_file_location('scorer', root / 'scripts/auto-tag-eval-replay/score-observations.py')
scorer = importlib.util.module_from_spec(module)
module.loader.exec_module(scorer)
requested = load(run / 'requested-membership.json')
assert [requested[k] for k in ['requestedPhysicalOccurrences', 'requestedFieldSlots',
    'predeclaredLabelableSlots', 'uniqueUserAcceptanceLabels', 'uniqueSelectedOccurrenceFieldLabels']] == [72, 216, 30, 11, 15]
assert load(run / 'selection-status.json')['executionStatus'] == 'complete'
results = load(run / 'results.json')
assert len(results['executions']) == 6
pairs, audit = {}, []
for expected in requested['evaluations']:
    name = expected['name']
    directory = run / name
    records = load(directory / 'cold/records.json')
    assert len(records) == 1
    row = records[0]
    prep = load(directory / 'cold/preparation.json')
    contract = load(directory / 'scoring-contract.json')
    score = scorer.project_native(root, contract, records)
    assert score == load(directory / 'score.provisional.json')
    assert score['requestedFieldSlots'] == score['observedFieldSlots'] == len(expected['physicalPaths']) * 3
    assert score['serviceFailureSlots'] == score['groundedAssistanceAccuracyCredit'] == 0
    assert score['writableDelta'] == [] and not score['acceptedCertificate']
    assert row['arm'] == expected['arm'] and row['phase'] == 'cold'
    assert row['fieldObservations']['inputJson'] == prep['fieldObservations']['inputJson']
    assert prep['separateStores'] and all(v == 0 for v in prep['executionCacheBefore'].values())
    assert row['cacheBefore'] == prep['executionCacheBefore']
    assert row['payloadUnchanged']
    assert load(directory / 'cold/source-hashes-before.json') == load(directory / 'cold/source-hashes-after.json')
    assert all(v['payloadSha256'] == row['after'][p]['payloadSha256'] for p, v in row['profileBaseline'].items())
    source_by_path = {s['path']: s for s in prep['physicalSources']}
    for event in row['pairedMock']['observedEvents']:
        for source in event.get('sources', []):
            assert source_by_path[source['path']] == source
        if event['kind'] in ['patch', 'recording_patch']:
            assert source_by_path[event['source']['path']] == event['source']
    inv_file = directory / 'cold/inventory-final.json'
    inventory = load(inv_file)
    assert inv_file.stat().st_size <= 8388608 and inventory['compact']
    assert not inventory['misses'] and not inventory['upstreamForwardingAvailable']
    assert len(inventory['records']) == 1533
    assert all('body' not in r and 'sha256' in r and isinstance(r['bytes'], int) for r in inventory['records'])
    route_keys = {(r['provider'], r['path'], r['query']) for r in inventory['records']}
    for request in inventory['requests']:
        assert (request['provider'], request['path'], json.dumps(request['query'], ensure_ascii=False, separators=(',', ':'))) in route_keys
    assert len(inventory['wireArrivals']) == len(inventory['requests']) + len(inventory['modelRequests']) + sum(
        w['route'].startswith('/__fixtures') for w in inventory['wireArrivals'])
    assert all(w['method'] in ['GET', 'POST'] and w['status'] == 200 for w in inventory['wireArrivals'])
    requests = [e['body'] for e in prep['plan']['events'] if e['kind'] == 'request']
    assert len(requests) == len(inventory['modelRequests']) == (1 if name == 'assistance-crowd' else 0)
    if requests:
        body_bytes = json.dumps(requests[0], ensure_ascii=False, sort_keys=True, separators=(',', ':')).encode()
        assert body_bytes == (directory / 'independent-request.json').read_bytes()
        assert hashlib.sha256(body_bytes).hexdigest() == inventory['modelRequests'][0]['sha256']
        assert inventory['modelRequests'][0]['bytes'] == len(body_bytes)
        assert row['reviewPendingAfterRestart'] and row['actionableReviewAfterRestart']
        assert row['persistedEvidence']['candidate']['assisted_genre'] == row['native']['candidate']['assisted_genre']
    if name.endswith('billie'):
        candidate = row['native']['candidate']
        assert candidate['discogs_release_id'] == '37353822'
        assert candidate['genre'] == 'Electronic, Pop, Indie Pop'
        track = next(t for t in row['fieldObservations']['tracks'] if '&burn' in t['path'])
        assert track['artists'] == ['Billie Eilish', 'Vince Staples']
    if name.endswith('enya'):
        assert prep['plan']['variant'] == 'hold'
        hold = next(e for e in row['pairedMock']['observedEvents'] if e['kind'] == 'hold')
        assert hold['reason'] == 'discovery_budget_limited'
        assert row['profileBaseline'] == row['after']
    if name.endswith('crowd'):
        tracks = row['fieldObservations']['tracks']
        assert next(t for t in tracks if 'Intro.flac' in t['path'])['title'] == 'Intro'
        assert row['beforeTracks'][0]['duration'] == 62.17
        assert next(t for t in tracks if 'Acoustic Ver.' in t['path'])['title'] == '英雄 (動畫電影《諸葛四郎-英雄的英雄》主題曲) [Acoustic Ver.]'
        assert score['userAcceptance']['requestedFieldSlots'] == 11
        assert score['userAcceptance']['agreeingFieldSlots'] == 0
    pairs.setdefault(expected['caseId'], []).append(row)
    audit.append(dict(name=name, variant=prep['plan']['variant'], outcome=row['outcome'],
        providerRequests=len(inventory['requests']), modelRequests=len(inventory['modelRequests']),
        wireArrivals=len(inventory['wireArrivals']), inventoryBytes=inv_file.stat().st_size,
        matchedEvents=row['pairedMock']['matchedEvents'], nativeElapsedMs=row['elapsedMs']))
for rows in pairs.values():
    assert len(rows) == 2 and rows[0]['profileBaseline'] == rows[1]['profileBaseline']
    left = json.loads(rows[0]['fieldObservations']['inputJson'])
    right = json.loads(rows[1]['fieldObservations']['inputJson'])
    assert left == right
print(json.dumps(dict(run=str(run.relative_to(root)), checks='passed', evaluations=audit,
    providerRequests=sum(a['providerRequests'] for a in audit),
    modelRequests=sum(a['modelRequests'] for a in audit),
    wireArrivals=sum(a['wireArrivals'] for a in audit),
    requestedFieldSlots=216, observedFieldSlots=216, labelableSlots=30,
    userAcceptanceRequested=22, userAcceptanceAgreeing=0, groundedAssistanceAccuracyCredit=0), indent=2))
