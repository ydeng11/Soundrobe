"""Recheck actual native exports and durable receipts without executing media or HTTP."""
import hashlib
import json
import pathlib
import subprocess
import sys

root = pathlib.Path.cwd()
run = pathlib.Path(sys.argv[1]).resolve()
assert run.is_relative_to(root / '.amp/in/artifacts')
load = lambda p: json.loads(p.read_text())
digest = lambda p: hashlib.sha256(p.read_bytes()).hexdigest()

# Preserve the sealed historical auditor. Only its preparation location changed.
original = root / '.planning/plans/cold-paired-mock-delivery-20261111/audit-run.py'
source = original.read_text()
old = "directory / 'cold/preparation.json'"
assert source.count(old) == 1
adapted = source.replace(old, "directory / 'preparation/preparation.json'")
native = json.loads(subprocess.check_output([sys.executable, '-B', '-c', adapted, str(run)], text=True))

receipt = load(run / 'authority-receipt.json')
stored = load(run / 'authority/final.json')
frozen = load(run / 'frozen-checkpoint.json')
requested = load(run / 'requested-membership.json')
assert receipt == stored
assert receipt['state'] == 'complete' and receipt['ownerLive'] is False
assert receipt['consumptions'] == 1 and receipt['denied'] == 0 and not receipt['ingressDenials']
assert receipt['membership'] == frozen['membership'] == requested
assert receipt['runId'] == frozen['runId'] and receipt['checkpoint'] == frozen['checkpoint']
assert receipt['bindings'] == frozen['bindings'] and receipt['liveAuthority'] is False
assert len(receipt['sessions']) == 12 and len(frozen['sessions']) == 6
assert frozen['state'] == 'frozen' and frozen['consumptions'] == 0 and frozen['counters']['model'] == 0
checkpoint_input = {k: frozen[v] for k, v in [('runId', 'runId'), ('membership', 'membership'),
    ('bindings', 'bindings'), ('preparations', 'preparations'), ('services', 'sessions'), ('counters', 'counters')]}
assert hashlib.sha256(json.dumps(checkpoint_input, ensure_ascii=False, sort_keys=True,
    separators=(',', ':')).encode()).hexdigest() == frozen['checkpoint']
assert all(receipt['sessions'][k] == s for k, s in frozen['sessions'].items())
assert all(receipt['operations'][k] == o for k, o in frozen['operations'].items())
assert receipt['preparations'] == frozen['preparations']
assert receipt['bindings']['executable'] == digest(run / 'native-evaluator')
for binding in receipt['bindings']['sourcesAndTools']:
    assert digest(root / binding['path']) == binding['sha256'], binding['path']

all_wires = []
provider = model = inventory = 0
inventory_sizes = []
for expected in requested['evaluations']:
    name = expected['name']
    preparation = run / name / 'preparation/preparation.json'
    assert digest(preparation) == receipt['preparations'][name]['preparationSha256']
    for stage, folder in [('preparation', 'preparation'), ('execution', 'cold')]:
        session_id, session = next((k, s) for k, s in receipt['sessions'].items() if s['job'] == name and s['stage'] == stage)
        assert session['attached'] and session['sealed'] and session['finished'] and session['closed']
        assert session['native_live'] is False and session['fixture_live'] is False
        if stage == 'execution':
            assert session['observation_sha256'] == digest(run / name / folder / 'cold.jsonl')
        file = run / name / folder / 'inventory-final.json'
        inv = load(file)
        snapshot = load(run / name / (folder + '-fixture-ingress.json'))
        assert inv == snapshot
        assert inv['compact'] and len(inv['records']) == 1533 and not inv['misses']
        assert file.stat().st_size <= 8388608
        inventory_sizes.append(file.stat().st_size)
        for wire in inv['wireArrivals']:
            operation = receipt['operations'][wire['operation']]
            assert operation['session'] == session_id
            assert operation['dispatched'] and operation['arrived'] and operation['completed']
            assert operation['category'] == ('inventory' if wire['route'] == '/__fixtures/compact'
                else 'model' if wire['method'] == 'POST' else 'provider')
            all_wires.append(wire['operation'])
        provider += len(inv['requests'])
        model += len(inv['modelRequests'])
        inventory += sum(w['route'] == '/__fixtures/compact' for w in inv['wireArrivals'])
assert len(all_wires) == len(set(all_wires)) == len(receipt['operations'])
assert receipt['counters'] == dict(provider=provider, inventory=inventory, model=model,
    reserved=len(all_wires), arrived=len(all_wires), completed=len(all_wires))
assert provider <= 512 and inventory <= 32 and model == 1 and len(all_wires) <= 576
assert load(run / 'wire-reconciliation.json')['counts'] == receipt['counters']
print(json.dumps(dict(native=native, durable=dict(runId=receipt['runId'], checkpoint=receipt['checkpoint'],
    counts=receipt['counters'], inventoryBytesMin=min(inventory_sizes), inventoryBytesMax=max(inventory_sizes),
    sessions=12, consumptions=1, receipt='complete', actualWireArrivals=len(all_wires)),
    auditor=dict(historicalSourceSha256=digest(original), adaptedSourceSha256=hashlib.sha256(adapted.encode()).hexdigest(),
        adaptation='preparation/preparation.json instead of cold/preparation.json'),
    liveAuthority=False, powerLossProved=False, groundedAssistanceAccuracyCredit=0), indent=2))
