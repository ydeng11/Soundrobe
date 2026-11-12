"""Read-only re-audit of witnessed mock completion and all 216 native slots."""
import hashlib
import json
import pathlib
import subprocess
import sys

ROOT = pathlib.Path.cwd()


def load(file):
    return json.loads(file.read_text())


def digest(file):
    return hashlib.sha256(file.read_bytes()).hexdigest()


def audit(run):
    run = run.resolve()
    assert run.is_relative_to(ROOT / '.amp/in/artifacts')
    config = load(run / 'native-supervision.json')
    outcome = load(run / 'receiver-outcome.json')
    launcher = load(run / 'launcher-outcome.json')
    identity = load(run / 'group-identity.json')
    supervisor = load(run / 'supervisor-identity.json')
    assert launcher['state'] == outcome['state'] == 'complete'
    assert launcher['actualReceiverExitObserved'] and launcher['receiverExit'] == 0
    assert launcher['receiverSignal'] is None
    assert launcher['receiverOutcomeSha256'] == digest(run / 'receiver-outcome.json')
    assert launcher['authority'] == outcome['authority'] == 'native_witness_and_timely_supervisor_exit'
    assert launcher['generation'] == outcome['generation'] == config['generation']
    assert launcher['receiverPid'] == outcome['receiverPid'] == config['receiverPid']
    assert not outcome['controlOnly'] and not config['control']
    assert outcome['requestedFieldSlots'] == launcher['requestedFieldSlots'] == 216
    assert outcome['liveAuthority'] is launcher['liveAuthority'] is False
    assert outcome['receiverGroup'] == config['receiverGroup']
    assert outcome['supervisorGroup'] == supervisor['group'] == supervisor['pid'] == identity['supervisorPid']
    assert outcome['executionGroup'] == identity['group'] == identity['anchorPid']
    assert len({outcome[k] for k in ['receiverGroup', 'supervisorGroup', 'executionGroup']}) == 3
    assert outcome['supervisorExitObservedAt'] <= outcome['acceptedAt'] < outcome['deadline']
    assert outcome['evidenceSha256'] == digest(run / 'authority/provisional.json')
    assert outcome['resultSha256'] == digest(run / 'results.json')
    phase = load(run / 'execution-scope.json')
    assert config['preparationMs'] == 300000 and config['executionMs'] == phase['executionMs'] == 600000
    assert phase['deadline'] == outcome['deadline']
    preparation_ms = phase['deadline'] - phase['executionMs'] - config['start']
    assert 0 < preparation_ms < config['preparationMs']
    assert load(run / 'authority/provisional.json')['bindings']['supervision'] == {
        **config, 'executionGroup': identity['group']}

    # Bind scorer inputs and published results to the actual native JSONL whose
    # digest the broker retained, rather than accepting two unrelated exports.
    requested = load(run / 'requested-membership.json')
    executions = []
    for expected in requested['evaluations']:
        directory = run / expected['name']
        rows = [json.loads(line) for line in (directory / 'cold/cold.jsonl').read_text().splitlines() if line.strip()]
        assert load(directory / 'cold/records.json') == rows
        execution = load(directory / 'result.provisional.json')
        assert execution['score'] == load(directory / 'score.provisional.json')
        executions.append(execution)
    assert load(run / 'results.json')['executions'] == executions

    # Preserve the sealed auditors and their field/version/credit/highlight and
    # wire checks. Only lifecycle names and the executable's retained location
    # differ. These substitutions do not establish authority; the gate above does.
    original = ROOT / '.planning/design/live-run-contract-20261111/durable-mock-evidence-20261112/audit-durable.py'
    source = original.read_text()
    old_status = "assert load(run / 'selection-status.json')['executionStatus'] == 'complete'"
    new_status = "assert load(run / 'selection-staged.json')['executionStatus'] == 'provisional_complete'"
    replacements = [
        ('source = original.read_text()',
         f'source = original.read_text()\nassert source.count({old_status!r}) == 1\nsource = source.replace({old_status!r}, {new_status!r})'),
        ("run / 'authority-receipt.json'", "run / 'authority/provisional.json'"),
        ("run / 'authority/final.json'", "run / 'authority-provisional.json'"),
        ("receipt['state'] == 'complete'", "receipt['state'] == 'provisional_complete'"),
        ("digest(run / 'native-evaluator')", "digest(root / receipt['bindings']['sourcesAndTools'][0]['path'])"),
        ("receipt='complete'", "receipt='provisional_complete'"),
    ]
    for old, new in replacements:
        assert source.count(old) == 1, old
        source = source.replace(old, new)
    checked = json.loads(subprocess.check_output([sys.executable, '-B', '-c', source, str(run)], text=True))
    return dict(supervision=dict(authority=outcome['authority'], generation=outcome['generation'],
        supervisorExitObservedAt=outcome['supervisorExitObservedAt'], acceptedAt=outcome['acceptedAt'],
        deadline=outcome['deadline'], preparationMs=preparation_ms, requestedFieldSlots=216),
        native=checked['native'], durable=checked['durable'],
        auditBindings=dict(historicalDurableAuditor=digest(original),
            adaptedDurableAuditor=hashlib.sha256(source.encode()).hexdigest(), native=checked['auditor']),
        liveAuthority=False, powerLossProved=False, groundedAssistanceAccuracyCredit=0)


if __name__ == '__main__':
    print(json.dumps(audit(pathlib.Path(sys.argv[1])), indent=2))
