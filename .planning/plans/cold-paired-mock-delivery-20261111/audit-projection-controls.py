"""Adversarial projection controls against actual disposable native exports."""
import copy
import importlib.util
import json
import pathlib
import sys

root = pathlib.Path.cwd()
run = pathlib.Path(sys.argv[1]).resolve()
assert run.is_relative_to(root / '.amp/in/artifacts')
module = importlib.util.spec_from_file_location('scorer', root / 'scripts/auto-tag-eval-replay/score-observations.py')
scorer = importlib.util.module_from_spec(module)
module.loader.exec_module(scorer)
checks = []
for directory in sorted(run.glob('*/scoring-contract.json')):
    contract = json.loads(directory.read_text())
    rows = json.loads((directory.parent / 'cold/records.json').read_text())
    slots = len(contract['slots'])
    valid = scorer.project_native(root, contract, rows)
    assert valid['observedFieldSlots'] == slots
    controls = [('missing-proof', lambda r: r.pop('pairedMock')),
        ('stopped', lambda r: r['pairedMock'].update(stopped=True)),
        ('false-cursor', lambda r: r['pairedMock'].update(matchedEvents=0)),
        ('copied-plan-mismatch', lambda r: r['pairedMock'].update(events=[])),
        ('missing-actual-events', lambda r: r['pairedMock'].pop('observedEvents')),
        ('contradictory-actual-events', lambda r: r['pairedMock'].update(observedEvents=[
            {'kind': 'patch', 'path': 'not-approved.flac', 'fields': {'title': 'Unapproved'}}]))]
    for name, mutate in controls:
        changed = copy.deepcopy(rows)
        mutate(changed[0])
        score = scorer.project_native(root, contract, changed)
        assert score['observedFieldSlots'] == 0 and score['serviceFailureSlots'] == slots
        assert score['outcomes'] == {'observation_unknown': slots}
        checks.append(directory.parent.name + '/' + name)
    missing = scorer.project_native(root, contract, [])
    assert missing['requestedFieldSlots'] == missing['serviceFailureSlots'] == slots
    checks.append(directory.parent.name + '/missing-record')
    for name, change in [('wrong-arm', {'arm': 'disabled' if rows[0]['arm'] == 'assistance' else 'assistance'}),
                         ('warm-injection', {'phase': 'warm'})]:
        try:
            scorer.project_native(root, contract, [dict(rows[0], **change)])
        except ValueError:
            checks.append(directory.parent.name + '/' + name)
        else:
            raise AssertionError(name)
    try:
        scorer.project_native(root, contract, rows * 2)
    except ValueError:
        checks.append(directory.parent.name + '/duplicate')
    else:
        raise AssertionError('duplicate')
    failed = copy.deepcopy(rows)
    failed[0]['serviceFailureObserved'] = True
    score = scorer.project_native(root, contract, failed)
    assert score['serviceFailureSlots'] == slots and score['observedFieldSlots'] == slots
    assert score['outcomes'] == valid['outcomes']
    checks.append(directory.parent.name + '/failed-agreement-preserved')
    if directory.parent.name == 'assistance-crowd':
        for name in ['revert-assisted-candidate', 'omit-inferred-patches', 'omit-required-review', 'wrong-provenance-model']:
            changed = copy.deepcopy(rows)
            events = changed[0]['pairedMock']['observedEvents']
            for index, event in enumerate(events):
                if name == 'revert-assisted-candidate' and event['kind'] == 'candidate' and 'assisted_genre' in event['candidate']:
                    events[index] = copy.deepcopy(contract['pairedPlans'][contract['jobs'][0]['id']]['events'][index])
                if name == 'omit-inferred-patches' and event['kind'] == 'patch':
                    event['fields'].pop('genre', None)
                if name == 'omit-required-review' and event['kind'] == 'review':
                    event['required'] = False
                if name == 'wrong-provenance-model' and event['kind'] == 'candidate' and 'assisted_genre' in event['candidate']:
                    event['candidate']['assisted_genre']['model'] = 'not-the-requested-model'
            assert changed != rows
            score = scorer.project_native(root, contract, changed)
            assert score['observedFieldSlots'] == 0 and score['serviceFailureSlots'] == slots
            checks.append(directory.parent.name + '/' + name)
assert len(checks) == 70
print(json.dumps({'controlsPassed': len(checks), 'controls': checks,
    'scope': 'in-memory corruptions of actual exports, not native admission or live authority'}, indent=2))
