"""Read-only re-audit of witnessed mock completion and all 216 native slots."""
import hashlib
import json
import pathlib
import subprocess
import sys
import uuid

ROOT = pathlib.Path.cwd()


def load(file):
    return json.loads(file.read_text())


def digest(file):
    return hashlib.sha256(file.read_bytes()).hexdigest()


def storage_audit(run, config, allow_stopped=False):
    """Independent replay of grants, not an inference from final file sizes."""
    mib = 1048576
    with (run / '.storage-ledger').open('rb') as source:
        ledger = source.read(2 * mib + 1)
    assert len(ledger) == 2 * mib
    anchor = ledger[:256]
    number = lambda block, offset: int.from_bytes(block[offset:offset+8], 'little')
    sequence = number(anchor, 81)
    revocation_offset = 2 * mib - 65536
    maximum_records = revocation_offset // 256 - 2
    assert 0 < sequence <= maximum_records
    ordinary = (2 * mib, 4 * mib)
    terminal = False
    owners = list(run.glob('.storage-owner-*'))
    assert 1 <= len(owners) <= 255
    for owner in owners:
        slot = int(owner.name.removeprefix('.storage-owner-'))
        assert owner.name == f'.storage-owner-{slot:03}' and 0 <= slot < 255
        assert owner.is_file() and not owner.is_symlink() and owner.stat().st_size == 0
    for index in range(1, sequence + 1):
        block = ledger[index*256:(index+1)*256]
        assert block[1:17] == uuid.UUID(config['generation']).bytes
        assert block[17:49] == hashlib.sha256(str(run.resolve()).encode()).digest()
        assert block[224:] == hashlib.sha256(block[1:224]).digest()
        assert block[112:128] == b'mock-storage-v1\0'
        assert number(block, 81) == index
        assert (number(block,49), number(block,57)) == (512*mib,1024*mib)
        assert (run / f'.storage-owner-{block[111]:03}').is_file()
        assert int.from_bytes(block[107:111],'little') > 0
        issued = (number(block,91), number(block,99))
        if not terminal and block[89] == 1:
            assert issued == (2*mib,8*mib), 'terminal grant changed'
            terminal = True
        else:
            assert block[89] == int(terminal)
            ordinary = tuple(a+b for a,b in zip(ordinary,issued))
        assert (number(block,65),number(block,73)) == ordinary, 'grant/counter disagreement'
        assert ordinary[0] <= 510*mib and ordinary[1] <= 1016*mib, 'ordinary cap exceeded'
    assert terminal and anchor[1:] == ledger[sequence*256+1:(sequence+1)*256]
    assert not any(ledger[(sequence+1)*256:revocation_offset]), 'ambiguous grant history'
    assert not any(ledger[revocation_offset+1:]), 'unexpected revocation tail'
    assert anchor[0] in (0,1) and ledger[revocation_offset] in (0,1)
    stopped = bool(anchor[0] or ledger[revocation_offset])
    assert allow_stopped or not stopped
    # Two bytes cover initial latch touching and its once-only transition.
    # Successful and worst-case failed writes still fit the original backing.
    bookkeeping = 256 + sequence*512 + 255*513 + 2
    assert bookkeeping <= 4*mib
    return dict(ordinaryGrowthReserved=ordinary[0], ordinaryWriteBytesReserved=ordinary[1],
        terminalGrowthReserved=2*mib, terminalWriteBytesReserved=8*mib,
        totalGrowthReserved=ordinary[0]+2*mib, totalWriteBytesReserved=ordinary[1]+8*mib,
        records=sequence, owners=len(owners), stopped=stopped,
        selfBookkeepingWriteUpperBound=bookkeeping, ledgerSha256=hashlib.sha256(ledger).hexdigest(),
        accounting='nonrefundable_prepaid_application_vfs_not_physical_io')


def incomplete_audit(run):
    # No partial result/JSONL parsing, preparation credit or staging promotion.
    expected = ROOT / 'test/fixtures/tauri/auto-tag-eval/supervised-requested-membership.json'
    membership = load(expected)
    try:
        membership_evidence = 'matched' if load(run / 'requested-membership.json') == membership else 'mismatched'
    except (OSError, ValueError, TypeError):
        membership_evidence = 'unavailable'
    try:
        storage = storage_audit(run, load(run / 'native-supervision.json'), allow_stopped=True)
    except (OSError, ValueError, KeyError, TypeError, AssertionError):
        # Failure projection is not storage acceptance. Missing/torn evidence
        # must not erase requested slots or invent a validated accounting state.
        storage = dict(state='unknown', reason='recording_unavailable_or_invalid')
    slots = [f"{e['arm']}/{e['caseId']}/{e['profile']}/{e['phase']}/{path}/{field}"
             for e in membership['evaluations'] for path in e['physicalPaths'] for field in e['fields']]
    assert len(slots) == len(set(slots)) == membership['requestedFieldSlots'] == 216
    return dict(state='incomplete', requestedFieldSlots=216, validatedFieldSlots=0,
        unavailableFieldSlots=216, slots=[dict(id=slot, observation='unavailable', effects='unknown') for slot in slots],
        storage=storage, membershipEvidence=membership_evidence, requestedMembershipSha256=digest(expected),
        containment='not_inferred_from_files', groundedAssistanceAccuracyCredit=0, liveAuthority=False)


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
    result = dict(supervision=dict(authority=outcome['authority'], generation=outcome['generation'],
        supervisorExitObservedAt=outcome['supervisorExitObservedAt'], acceptedAt=outcome['acceptedAt'],
        deadline=outcome['deadline'], preparationMs=preparation_ms, requestedFieldSlots=216),
        native=checked['native'], durable=checked['durable'],
        auditBindings=dict(historicalDurableAuditor=digest(original),
            adaptedDurableAuditor=hashlib.sha256(source.encode()).hexdigest(), native=checked['auditor']),
        liveAuthority=False, powerLossProved=False, groundedAssistanceAccuracyCredit=0)
    if config.get('storage'):
        result['storage'] = storage_audit(run, config)
    return result


if __name__ == '__main__':
    run = pathlib.Path(sys.argv[1]).resolve()
    print(json.dumps(incomplete_audit(run) if sys.argv[2:] == ['--incomplete'] else audit(run), indent=2))
