"""Corrupt real exports in memory; never modify sealed sources or media."""
import copy
import importlib.util
import json
import pathlib
import sys

root = pathlib.Path.cwd()
spec = importlib.util.spec_from_file_location('scorer', root / 'scripts/auto-tag-eval-replay/score-observations.py')
scorer = importlib.util.module_from_spec(spec)
spec.loader.exec_module(scorer)
run = pathlib.Path(sys.argv[1])
checks = []
for role in ['provider', 'genre', 'unknown-usage', 'cancel', 'hold']:
    directory = run / role
    contract = json.loads((directory / 'scoring-contract.json').read_text())
    records = json.loads((directory / 'bounded/records.json').read_text())
    manifest = {'path': str(directory / 'bounded-manifest.json'),
                'sha256': __import__('hashlib').sha256((directory / 'bounded-manifest.json').read_bytes()).hexdigest()}
    good = scorer.project_native(root, contract, records, manifest)
    assert good['requestedPhysicalOccurrences'] == len(contract['slots']) // 3
    assert good['observedFieldSlots'] == good['requestedFieldSlots']
    checks.append({'role': role, 'control': 'real-export', 'passed': True})
    variants = {'omitted-job': records[:-1], 'duplicate-job': records + [records[0]]}
    for label, key, value in [('corrupted-input', 'inputSha256', '0' * 64),
                              ('changed-request', 'requestJson', '{}')]:
        changed = copy.deepcopy(records)
        changed[0]['fieldObservations'][key] = value
        variants[label] = changed
    changed = copy.deepcopy(records)
    changed[0]['fieldObservations']['tracks'][0]['path'] = '../outside.flac'
    variants['wrong-physical-path'] = changed
    for label, changed in variants.items():
        try:
            scorer.project_native(root, contract, changed, manifest)
        except ValueError:
            checks.append({'role': role, 'control': label, 'passed': True})
        else:
            raise AssertionError((role, label, 'accepted corrupted export'))
    changed_contract = copy.deepcopy(contract)
    changed_contract['spec']['bindings']['labels']['sha256'] = '0' * 64
    try:
        scorer.project_native(root, changed_contract, records, manifest)
    except ValueError:
        checks.append({'role': role, 'control': 'changed-label-binding', 'passed': True})
    else:
        raise AssertionError('accepted changed label binding')
    unavailable = copy.deepcopy(records)
    for row in unavailable:
        row['fieldObservations'].update(readbackAvailable=False, readbackError='injected unavailable readback', tracks=None)
    unknown = scorer.project_native(root, contract, unavailable, manifest)
    assert unknown['requestedFieldSlots'] == good['requestedFieldSlots']
    assert unknown['predeclaredLabelableSlots'] == good['predeclaredLabelableSlots']
    assert unknown['outcomes'] == {'observation_unknown': good['requestedFieldSlots']}
    assert unknown['groundedAssistanceAccuracyCredit'] == 0 and unknown['writableDelta'] == []
    checks.append({'role': role, 'control': 'unavailable-keeps-denominators', 'passed': True})
print(json.dumps({'controls': checks, 'passed': len(checks), 'modifiedSourcesOrMedia': False}, indent=2))
