"""Corrupt copies of retained receipts; never change the frozen run or execute media."""
import copy
import json
import pathlib
import shutil
import subprocess
import sys
import tempfile

root = pathlib.Path.cwd()
run = pathlib.Path(sys.argv[1]).resolve()
assert run.is_relative_to(root / '.amp/in/artifacts')
auditor = pathlib.Path(__file__).with_name('audit-durable.py')
load = lambda p: json.loads(p.read_text())
receipt = load(run / 'authority-receipt.json')
job = receipt['membership']['evaluations'][0]['name']
execution = next(k for k, s in receipt['sessions'].items() if s['stage'] == 'execution')
operation = next(iter(receipt['operations']))

def change_receipt(field, value):
    changed = copy.deepcopy(receipt)
    target = changed
    for key in field[:-1]:
        target = target[key]
    target[field[-1]] = value
    return {'authority-receipt.json': changed, 'authority/final.json': changed}

controls = {
    'terminal-receipt': change_receipt(['state'], 'terminal'),
    'checkpoint-mismatch': change_receipt(['checkpoint'], '0' * 64),
    'unattached-execution': change_receipt(['sessions', execution, 'attached'], False),
    'wrong-observation-digest': change_receipt(['sessions', execution, 'observation_sha256'], '0' * 64),
    'wrong-executable': change_receipt(['bindings', 'executable'], '0' * 64),
    'unsettled-operation': change_receipt(['operations', operation, 'completed'], False),
    'reset-model-counter': change_receipt(['counters', 'model'], 0),
}
inventory_path = f'{job}/cold/inventory-final.json'
inventory = load(run / inventory_path)
inventory['wireArrivals'].append(copy.deepcopy(inventory['wireArrivals'][0]))
controls['duplicate-actual-wire'] = {
    inventory_path: inventory, f'{job}/cold-fixture-ingress.json': inventory,
}

def check(directory):
    return subprocess.run([sys.executable, '-B', str(auditor), str(directory)], capture_output=True, text=True)

assert check(run).returncode == 0, 'uncorrupted run must pass first'
passed = []
for name, replacements in controls.items():
    directory = pathlib.Path(tempfile.mkdtemp(prefix='durable-audit-control-', dir=root / '.amp/in/artifacts'))
    try:
        # Only metadata is read. Links avoid copying generated media and the binary.
        for original in run.rglob('*'):
            if original.is_file():
                target = directory / original.relative_to(run)
                target.parent.mkdir(parents=True, exist_ok=True)
                target.symlink_to(original)
        for relative, value in replacements.items():
            target = directory / relative
            target.unlink()
            target.write_text(json.dumps(value))
        result = check(directory)
        assert result.returncode != 0 and 'AssertionError' in result.stderr, (name, result.stdout, result.stderr)
        passed.append(name)
    finally:
        shutil.rmtree(directory)
assert check(run).returncode == 0, 'original run must remain valid'
print(json.dumps({'controlsPassed': len(passed), 'controls': passed,
    'scope': 'altered metadata copies; no native workflow, HTTP or media mutation'}, indent=2))
