"""Read-only re-audit of witnessed mock completion and all 216 native slots."""
import hashlib
import json
import pathlib
import re
import sqlite3
import stat
import subprocess
import sys
import uuid

ROOT = pathlib.Path.cwd()


def load(file):
    return json.loads(file.read_text())


def digest(file):
    return hashlib.sha256(file.read_bytes()).hexdigest()


def model_rehearsal_audit(run, receipt, requested):
    """Join rehearsal evidence only; never establish completion or live authority."""
    policy = 'offline-model-rehearsal-v1;separate-verifier;fake-credential;loopback-only;no-live;no-retry;no-repair'
    disabled_policy = 'disabled-hosted-boundary-v1;native-file;native-response-observation;no-live;no-retry;no-repair'
    canonical = lambda value: json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(',', ':')).encode()
    compact = lambda value: json.dumps(value, ensure_ascii=False, separators=(',', ':')).encode()
    sha = lambda value: hashlib.sha256(value).hexdigest()
    bindings = receipt['bindings']
    selection = load(run / 'selection.json')
    descriptors = [(expected['name'], stage, folder,
        load(run / expected['name'] / folder / 'session.json'))
        for expected in requested['evaluations']
        for stage, folder in [('preparation', 'preparation'), ('execution', 'cold')]]
    contextual = ('modelRehearsalPlan' in bindings or 'modelRehearsal' in bindings
        or 'modelRehearsalPlan' in selection or receipt.get('testPolicy') in (policy, disabled_policy)
        or any('model_rehearsal' in descriptor for _, _, _, descriptor in descriptors))
    if not contextual:
        return None
    assert 'modelRehearsalPlan' in bindings, 'model rehearsal context missing immutable plan'
    plan = bindings['modelRehearsalPlan']
    hosted = 'offlineHosted' in plan
    assert set(plan) == {'model', 'observedTarget', 'route', 'provider', 'input'} | ({'offlineHosted'} if hosted else set()), 'model rehearsal plan fields'
    if hosted:
        policy = disabled_policy
        assert bindings['modelRehearsal']['offlineFixture'] is True
    assert set(plan['input']) == {'artist', 'album', 'tracks'}, 'model rehearsal input fields'
    locked_plans = []
    for source in bindings['sourcesAndTools']:
        file = ROOT / source['path']
        if file.suffix == '.json' and load(file) == plan:
            assert file.resolve().is_relative_to(ROOT) and not pathlib.Path(source['path']).is_absolute()
            assert digest(file) == source['sha256'], 'model rehearsal public plan hash drift'
            locked_plans.append(source['path'])
    assert len(locked_plans) == 1, 'model rehearsal public plan must be source-locked exactly once'
    if hosted:
        # This deliberately public file is the sole credential input we read.
        # Never inspect development .env.local or resolve an arbitrary source as a credential.
        credential = ROOT / pathlib.Path(locked_plans[0]).parent / 'credential-fixture/.env.local'
        assert credential.resolve().is_relative_to(ROOT) and not credential.resolve().is_relative_to(run.resolve())
        assert not credential.is_symlink() and not credential.parent.is_symlink()
        assert stat.S_ISREG(credential.stat().st_mode) and stat.S_IMODE(credential.stat().st_mode) == 0o600
        credential_bytes = f"LLM_API_KEY=MOCK_ONLY_SECRET_CANARY\nLLM_MODEL={plan['model']}\n".encode()
        assert credential.read_bytes() == credential_bytes, 'public fake credential bytes drift'
        locked_credentials = [s for s in bindings['sourcesAndTools'] if (ROOT / s['path']).resolve() == credential.resolve()]
        assert len(locked_credentials) == 1, 'public fake credential must be source-locked exactly once'
        assert not pathlib.Path(locked_credentials[0]['path']).is_absolute()
        assert locked_credentials[0]['sha256'] == sha(credential_bytes), 'public fake credential hash drift'
    for key in ['model', 'observedTarget']:
        assert isinstance(plan[key], str) and re.fullmatch(r'~?deepseek/[a-zA-Z0-9_.:/-]+', plan[key])
        assert len(plan[key].encode()) <= 160
    assert plan['model'] != plan['observedTarget'], 'request alias and response target must remain separate'
    assert re.fullmatch(r'[a-z0-9_/-]{1,100}', plan['route']) and all(plan['route'].split('/'))
    assert re.fullmatch(r'[a-zA-Z0-9 _/-]{1,100}', plan['provider'])
    payload = plan['input']
    assert isinstance(payload['tracks'], list) and len(payload['tracks']) <= 100
    for value in [payload['artist'], payload['album']]:
        assert value is None or isinstance(value, str)
    for value in payload['tracks']:
        assert isinstance(value, str)
    for value in [payload['artist'], payload['album'], *payload['tracks']]:
        assert value is None or (len(value.encode()) <= 2048 and not any(ord(c) < 32 or 127 <= ord(c) <= 159 for c in value))
    body = dict(model=plan['model'], temperature=0.2, max_tokens=1024,
        reasoning=dict(enabled=False), stream=False,
        provider=dict(allow_fallbacks=False, require_parameters=True, only=[plan['route']], order=[plan['route']]),
        messages=[dict(role='system', content='Infer a conservative music genre from the supplied artist, album, and track titles. '
            'Return a concise Discogs-style comma-separated genre and confidence. Use low confidence when the evidence is insufficient.'),
            dict(role='user', content=canonical(payload).decode())],
        response_format=dict(type='json_schema', json_schema=dict(name='GenreFillResponse', schema=dict(
            type='object', properties=dict(genre=dict(type='string'), confidence=dict(type='number')),
            required=['genre', 'confidence']))))
    request_bytes = canonical(body)
    assert len(request_bytes) <= 32768
    request_sha = sha(request_bytes)
    # Public seed43 hosted / seed42 fixture verifiers are independently fixed,
    # never supplied by JSON; neither establishes real user approval.
    verifier = bytes.fromhex('4508a07aa941707f3eb2db94c8897a80b2c1197476b6de213ac273df7d86c4ff' if hosted
        else '197f6b23e16c8532c6abc838facd5ea789be0c76b2920334039bfa8b3d368d61')
    binding = dict(policy=policy, model=plan['model'], observedTarget=plan['observedTarget'],
        providerClaim=plan['provider'], route=plan['route'], requestSha256=request_sha,
        verifierSha256=sha(verifier), liveAuthority=False)
    if hosted:
        binding['offlineFixture'] = True
        pricing = plan['offlineHosted']
        assert isinstance(pricing, dict) and set(pricing) == {'promptPicoUsd', 'completionPicoUsd', 'catalogSha256', 'termsSha256'}
        for key in ['promptPicoUsd', 'completionPicoUsd']:
            assert type(pricing[key]) is int and 0 <= pricing[key] <= 2**64 - 1
        for key in ['catalogSha256', 'termsSha256']:
            assert isinstance(pricing[key], str) and re.fullmatch('[a-f0-9]{64}', pricing[key])
        estimate = pricing['promptPicoUsd'] * 8192 + pricing['completionPicoUsd'] * 1024
        assert estimate <= 5_000_000_000_000
        settings = dict(model=plan['model'], endpoint='https://openrouter.ai/api/v1', provider='openrouter', source='env_local_only')
        binding['boundary'] = dict(endpoint='https://openrouter.ai/api/v1/chat/completions',
            settingsSha256=sha(compact(settings)), pricingEvidence=pricing, textEstimatePicoUsd=estimate,
            allowanceUsdCents=500, inputTokenIllustration=8192, outputTokens=1024, hardBillingCap=False,
            billingRiskAccepted=False, liveDispatchAvailable=False, readyForExecution=False,
            retention=dict(remote='documented_provider_defaults_for_public_metadata', local='until_result_review'))
    assert canonical(bindings['modelRehearsal']) == canonical(binding), 'model rehearsal immutable binding drift'
    assert bindings['modelRehearsal']['liveAuthority'] is False
    assert receipt['testPolicy'] == policy
    assert selection['modelRehearsalPlan'] == plan and selection['model'] == plan['model']
    names = [e['name'] for e in requested['evaluations']]
    assert len(names) == len(set(names)) == 6 and 'assistance-crowd' in names
    assert set(receipt['preparations']) == set(names)
    for name, stage, folder, descriptor in descriptors:
        assert descriptor['model_rehearsal'] == plan, f'{name}/{folder}: rehearsal descriptor drift'
        assert descriptor['job'] == name and descriptor['stage'] == stage
    answer = dict(genre='MOCK_ACCEPTED_GENRE_CANARY', confidence=0.8)
    usage = dict(prompt_tokens=7, completion_tokens=3, total_tokens=10)
    response_encoder = canonical if hosted else compact
    response_bytes = response_encoder(dict(model=plan['observedTarget'], provider=plan['provider'],
        choices=[dict(finish_reason='stop', message=dict(content=response_encoder(answer).decode()))],
        usage=usage, diagnostic='MOCK_RAW_DIAGNOSTIC_CANARY'))
    response_sha = sha(response_bytes)
    model_operations = {k: v for k, v in receipt['operations'].items() if v['category'] == 'model'}
    assert len(model_operations) == receipt['counters']['model'] == receipt['credentialLookups'] == 1
    operation_id, operation = next(iter(model_operations.items()))
    assert all(operation[k] is True for k in ['dispatched', 'completed']) and operation['arrived'] is (not hosted)
    assert operation['request_sha256'] == sha(compact(['POST', '/model/chat/completions', request_sha]))
    response = dict(sha256=response_sha, bytes=len(response_bytes), status=200,
        identityClaimsVerified=False, pricingVerified=False, usageClaimsVerified=False)
    if hosted:
        response.update(observation='offline_injected_native_response', remoteArrivalVerified=False)
    assert canonical(operation['response']) == canonical(response)
    assert all(operation['response'][k] is False for k in ['identityClaimsVerified', 'pricingVerified', 'usageClaimsVerified'])
    session = receipt['sessions'][operation['session']]
    assert session['job'] == 'assistance-crowd' and session['stage'] == 'execution'
    for name in names:
        directory = run / name
        active = name == 'assistance-crowd'
        preparation = load(directory / 'preparation/preparation.json')
        prepared = receipt['preparations'][name]
        assert prepared['preparationSha256'] == digest(directory / 'preparation/preparation.json'), name
        assert prepared['requestSha256'] == (request_sha if active else None), name
        requests = [e['body'] for e in preparation['plan']['events'] if e['kind'] == 'request']
        assert requests == ([body] if active else []), f'{name}: prepared request drift'
        service = load(directory / 'service.json')
        if active:
            assert (directory / 'independent-request.json').read_bytes() == request_bytes
        if active and not hosted:
            scripted = service['scriptedModel']
            assert {k: scripted[k] for k in ['requestModel', 'responseModel', 'provider', 'maxRequests',
                'bodySha256', 'payload', 'response', 'usage', 'privacyCanaries']} == dict(
                    requestModel=plan['model'], responseModel=plan['observedTarget'], provider=plan['provider'],
                    maxRequests=1, bodySha256=request_sha, payload=payload, response=answer, usage=usage, privacyCanaries=True)
            assert scripted.get('status', 200) == 200
        else:
            assert 'scriptedModel' not in service, name
        if hosted:
            # Independently select from the Journal, then compare the driver's
            # projection; the export is never accepted as operation authority.
            selected = {k: v for k, v in model_operations.items()
                if receipt['sessions'][v['session']]['job'] == name
                and receipt['sessions'][v['session']]['stage'] == 'execution'}
            assert canonical(load(directory / 'native-model-observations.json')) == canonical(selected), f'{name}: native model projection drift'
        for stage, folder in [('preparation', 'preparation'), ('execution', 'cold')]:
            inventory = load(directory / folder / 'inventory-final.json')
            expected_request = [dict(status=200, sha256=request_sha, bytes=len(request_bytes),
                responseSha256=response_sha, responseBytes=len(response_bytes), mockCredentialPresented=True)] if active and not hosted and stage == 'execution' else []
            assert inventory['modelRequests'] == expected_request, f'{name}/{folder}: model inventory drift'
            posts = [w for w in inventory['wireArrivals'] if w['method'] == 'POST']
            assert len(posts) == len(expected_request)
            if posts:
                assert posts[0] == dict(operation=operation_id, method='POST', route='/model/chat/completions', status=200, state='responded')
                session = receipt['sessions'][operation['session']]
                assert session['job'] == name and session['stage'] == stage
        if hosted:
            execution = load(directory / 'result.provisional.json')
            assert execution['modelRequests'] == len(selected), f'{name}: native observation summary drift'
            assert execution['wireArrivals'] == len(inventory['wireArrivals']), f'{name}: invented summary arrival'
        rows = load(directory / 'cold/records.json')
        assert len(rows) == 1
        row = rows[0]
        candidate = row['native'].get('candidate') or {}
        assisted = dict(model=plan['model'], input_sha256=sha(canonical(payload)), value=answer['genre']) if active else None
        assert candidate.get('assisted_genre') == row['persistedEvidence']['assistedGenre'] == assisted, name
        if active:
            assert candidate['genre'] == answer['genre']
            assert row['fieldObservations']['tracks'] and all(t['genre'] == answer['genre'] for t in row['fieldObservations']['tracks'])
        assert load(directory / 'scoring-contract.json')['spec']['mode'] == 'mock', name
        score = load(directory / 'score.provisional.json')
        assert score['groundedAssistanceAccuracyCredit'] == 0 and score['writableDelta'] == [] and not score['acceptedCertificate']
    return dict(policy=policy, requestSha256=request_sha, responseSha256=response_sha,
        model=plan['model'], observedTarget=plan['observedTarget'], providerClaim=plan['provider'],
        verifierSha256=sha(verifier), preparedJobs=6, modelRequests=1,
        identityClaimsVerified=False, groundedAssistanceAccuracyCredit=0, liveAuthority=False)


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


def retained_auditor_source(hosted=False):
    """Count every adaptation; never modify the sealed historical sources."""
    original = ROOT / '.planning/design/live-run-contract-20261111/durable-mock-evidence-20261112/audit-durable.py'
    source = original.read_text()
    native_replacements = [
        ("assert load(run / 'selection-status.json')['executionStatus'] == 'complete'",
         "assert load(run / 'selection-staged.json')['executionStatus'] == 'provisional_complete'"),
        ("assert row['persistedEvidence']['candidate']['assisted_genre'] == row['native']['candidate']['assisted_genre']",
         "assert row['persistedEvidence']['assistedGenre'] == row['native']['candidate']['assisted_genre']"),
    ]
    if hosted:
        native_replacements += [
            ("    assert len(requests) == len(inventory['modelRequests']) == (1 if name == 'assistance-crowd' else 0)",
             "    native_observations = load(directory / 'native-model-observations.json')\n"
             "    assert not inventory['modelRequests']\n"
             "    assert len(requests) == len(native_observations) == (1 if name == 'assistance-crowd' else 0)"),
            ("        assert hashlib.sha256(body_bytes).hexdigest() == inventory['modelRequests'][0]['sha256']\n"
             "        assert inventory['modelRequests'][0]['bytes'] == len(body_bytes)",
             "        operation = next(iter(native_observations.values()))\n"
             "        assert operation['request_sha256'] == hashlib.sha256(json.dumps(['POST', '/model/chat/completions', "
             "hashlib.sha256(body_bytes).hexdigest()], separators=(',', ':')).encode()).hexdigest()"),
            ("providerRequests=len(inventory['requests']), modelRequests=len(inventory['modelRequests']),",
             "providerRequests=len(inventory['requests']), modelRequests=len(native_observations),"),
        ]
    nested = 'source = original.read_text()'
    for old, new in native_replacements:
        nested += f'\nassert source.count({old!r}) == 1\nsource = source.replace({old!r}, {new!r})'
    replacements = [
        ('source = original.read_text()', nested),
        ('assert receipt == stored',
         "assert stored.pop('actualFakeLookups') == receipt['credentialLookups'] == receipt['counters']['model'] == 1\nassert receipt == stored"),
        ("run / 'authority-receipt.json'", "run / 'authority/provisional.json'"),
        ("run / 'authority/final.json'", "run / 'authority-provisional.json'"),
        ("receipt['state'] == 'complete'", "receipt['state'] == 'provisional_complete'"),
        ("digest(run / 'native-evaluator')", "digest(root / receipt['bindings']['sourcesAndTools'][0]['path'])"),
        ("receipt='complete'", "receipt='provisional_complete'"),
    ]
    if hosted:
        replacements += [
            ('all_wires = []', 'all_wires = []\nnative_observations = {}'),
            ("    for stage, folder in [('preparation', 'preparation'), ('execution', 'cold')]:",
             "    selected = {k: v for k, v in receipt['operations'].items() if v['category'] == 'model'\n"
             "        and receipt['sessions'][v['session']]['job'] == name\n"
             "        and receipt['sessions'][v['session']]['stage'] == 'execution'}\n"
             "    assert json.dumps(load(run / name / 'native-model-observations.json'), sort_keys=True) == json.dumps(selected, sort_keys=True)\n"
             "    assert len(selected) == (1 if name == 'assistance-crowd' else 0)\n"
             "    for operation in selected.values():\n"
             "        assert operation['dispatched'] is True and operation['completed'] is True and operation['arrived'] is False\n"
             "        assert operation['response']['observation'] == 'offline_injected_native_response'\n"
             "        assert operation['response']['remoteArrivalVerified'] is False\n"
             "    native_observations.update(selected)\n"
             "    model += len(selected)\n"
             "    for stage, folder in [('preparation', 'preparation'), ('execution', 'cold')]:"),
            ("        model += len(inv['modelRequests'])",
             "        assert inv['modelRequests'] == [] and all(w['method'] == 'GET' for w in inv['wireArrivals'])"),
            ("assert len(all_wires) == len(set(all_wires)) == len(receipt['operations'])",
             "assert len(all_wires) == len(set(all_wires)) == 496\n"
             "assert not set(all_wires) & set(native_observations)\n"
             "assert set(all_wires) | set(native_observations) == set(receipt['operations'])\n"
             "assert len(receipt['operations']) == 497 and (provider, inventory, model) == (472, 24, 1)"),
            ('reserved=len(all_wires), arrived=len(all_wires), completed=len(all_wires))',
             'reserved=len(receipt[\'operations\']), arrived=len(all_wires), completed=len(receipt[\'operations\']))'),
            ("assert load(run / 'wire-reconciliation.json')['counts'] == receipt['counters']",
             "wire = load(run / 'wire-reconciliation.json')\n"
             "assert wire['counts'] == receipt['counters']\n"
             "assert wire['nativeModelObservations'] == len(native_observations) == 1\n"
             "assert wire['actualWireArrivals'] == len(all_wires) == 496\n"
             "assert wire['uniqueOperations'] == len(receipt['operations']) == 497"),
        ]
    for old, new in replacements:
        assert source.count(old) == 1, old
        source = source.replace(old, new)
    return original, source


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
    rehearsal = model_rehearsal_audit(run, load(run / 'authority/provisional.json'), requested)
    executions = []
    for expected in requested['evaluations']:
        directory = run / expected['name']
        rows = [json.loads(line) for line in (directory / 'cold/cold.jsonl').read_text().splitlines() if line.strip()]
        assert load(directory / 'cold/records.json') == rows
        assert len(rows) == 1
        native = rows[0]['native']
        projection = dict(outcome=native['outcome'], authority=native.get('authority'),
            coreReceipts=native.get('coreReceipts', []), completionBasis=native.get('completionBasis'),
            written=native['written'], assistedGenre=(native.get('candidate') or {}).get('assisted_genre'))
        if native.get('proofVersion') is not None:
            projection['proofVersion'] = native['proofVersion']
        assert rows[0]['persistedEvidence'] == projection
        worker = directory / 'retained-native/execution-stores/worker/worker.db'
        db = sqlite3.connect(worker.as_uri() + '?mode=ro', uri=True)
        try:
            evidence = [json.loads(row[0]) for row in db.execute('SELECT evidence FROM worker_evidence')]
        finally:
            db.close()
        assert evidence == [projection], expected['name']
        execution = load(directory / 'result.provisional.json')
        assert execution['score'] == load(directory / 'score.provisional.json')
        executions.append(execution)
    assert load(run / 'results.json')['executions'] == executions

    # All scoring/source/payload/worker checks remain; hosted mode distinguishes
    # a Journal observation from fixture wires without manufacturing an arrival.
    hosted = rehearsal is not None and 'offlineHosted' in load(run / 'authority/provisional.json')['bindings']['modelRehearsalPlan']
    original, source = retained_auditor_source(hosted)
    checked = json.loads(subprocess.check_output([sys.executable, '-B', '-c', source, str(run)], text=True))
    wire = load(run / 'wire-reconciliation.json')
    assert wire['actualFakeLookups'] == wire['durableLookupAdmissions'] == 1
    result = dict(actualFakeLookups=wire['actualFakeLookups'], durableLookupAdmissions=wire['durableLookupAdmissions'],
        workerProjection='exact_declared_fields_and_actual_worker_rows', supervision=dict(authority=outcome['authority'], generation=outcome['generation'],
        supervisorExitObservedAt=outcome['supervisorExitObservedAt'], acceptedAt=outcome['acceptedAt'],
        deadline=outcome['deadline'], preparationMs=preparation_ms, requestedFieldSlots=216),
        native=checked['native'], durable=checked['durable'],
        auditBindings=dict(historicalDurableAuditor=digest(original),
            adaptedDurableAuditor=hashlib.sha256(source.encode()).hexdigest(), native=checked['auditor']),
        liveAuthority=False, powerLossProved=False, groundedAssistanceAccuracyCredit=0)
    if rehearsal is not None:
        result['modelRehearsal'] = rehearsal
    if config.get('storage'):
        result['storage'] = storage_audit(run, config)
    return result


if __name__ == '__main__':
    run = pathlib.Path(sys.argv[1]).resolve()
    print(json.dumps(incomplete_audit(run) if sys.argv[2:] == ['--incomplete'] else audit(run), indent=2))
