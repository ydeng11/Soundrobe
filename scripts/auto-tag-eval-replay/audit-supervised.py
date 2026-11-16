"""Read-only re-audit of witnessed mock completion and all 216 native slots."""
import hashlib
import json
import math
import os
import pathlib
import re
import sqlite3
import stat
import subprocess
import sys
import tempfile
import uuid

ROOT = pathlib.Path.cwd()


def load(file):
    return json.loads(file.read_text())


def digest(file):
    return hashlib.sha256(file.read_bytes()).hexdigest()


def owner_execution_intent_audit(proposal, intent, public_key, expected_verifier_sha256, verified_at):
    """Verify only the public mock_run_owner v1 proposal/intent signature scope.

    Supply parsed JSON objects with duplicate keys already rejected, an
    independently trusted raw 32-byte Ed25519 key and lowercase SHA-256 pin,
    and integer Unix seconds. Invalid evidence or unavailable local OpenSSL
    raises AssertionError, including under optimized Python. No source paths
    are read: signed digest declarations do not verify actual source bytes,
    native construction, runtime custody, nonce consumption, execution, human
    presence, dispatch, billing or quality. Temporary verification files are
    public only; no credentials, private keys or network are consulted.
    """
    if not __debug__:
        raise AssertionError('owner intent audit requires enabled assertions')
    sha = lambda data: hashlib.sha256(data).hexdigest()
    is_digest = lambda value: type(value) is str and re.fullmatch('[a-f0-9]{64}', value) is not None
    is_u64 = lambda value: type(value) is int and 0 <= value < 2**64

    def is_uuid4(value):
        if type(value) is not str or len(value) != 36:
            return False
        try:
            identity = uuid.UUID(value)
            return identity.version == 4 and str(identity) == value
        except ValueError:
            return False

    def canonical(value, limit):
        # Exact shallow schemas are checked before encoding; no arbitrary JSON
        # trees or float representations enter this independent contract.
        try:
            encoded = json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(',', ':'), allow_nan=False).encode('utf-8')
        except (ValueError, UnicodeError, RecursionError):
            raise AssertionError('owner record encoding') from None
        assert len(encoded) <= limit, 'owner record byte bound'
        return encoded

    assert type(public_key) is bytes and len(public_key) == 32, 'public key'
    assert public_key != bytes(32) and public_key.hex() not in (
        'd75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a',
        '197f6b23e16c8532c6abc838facd5ea789be0c76b2920334039bfa8b3d368d61',
        '4508a07aa941707f3eb2db94c8897a80b2c1197476b6de213ac273df7d86c4ff'), 'fixture verifier'
    assert is_digest(expected_verifier_sha256) and sha(public_key) == expected_verifier_sha256, 'independent verifier digest'
    assert is_u64(verified_at), 'verification time'
    digest_fields = {'executableSha256', 'membershipSha256', 'nativePlanSha256', 'requestSha256',
        'verifierSha256', 'boundaryPricingSha256', 'contextSha256'}
    assert type(proposal) is dict and set(proposal) == digest_fields | {
        'version', 'proposalId', 'executionKind', 'sourcesAndTools'}, 'proposal fields'
    assert type(proposal['version']) is int and proposal['version'] == 1, 'proposal version'
    assert is_uuid4(proposal['proposalId']), 'proposal UUIDv4'
    kind = proposal['executionKind']
    assert type(kind) is str and kind in ('offlineRehearsal', 'hostedSingleRequest'), 'execution kind'
    assert all(is_digest(proposal[field]) for field in digest_fields), 'proposal digests'
    assert proposal['verifierSha256'] == expected_verifier_sha256, 'proposal verifier'
    sources = proposal['sourcesAndTools']
    assert type(sources) is list and 0 < len(sources) <= 4096, 'source list'
    paths = set()
    runtime_count = 0
    for source in sources:
        assert type(source) is dict and set(source) in ({'path', 'sha256'}, {'path', 'runtime'}), 'source fields'
        path = source['path']
        assert type(path) is str and 0 < len(path) <= 4096, 'source path'
        try:
            assert len(path.encode('utf-8')) <= 4096, 'source path byte bound'
        except UnicodeError:
            raise AssertionError('source path encoding') from None
        # Match Unix Path::components: inner dots/repeated separators collapse,
        # but a leading CurDir, root or any ParentDir is not Normal.
        parts = pathlib.PurePosixPath(path).parts
        assert not any(ord(char) < 32 or 127 <= ord(char) <= 159 for char in path) \
            and not path.startswith(('/', './')) and path != '.' \
            and parts and all(part != '..' for part in parts), 'relative source path'
        assert path not in paths, 'duplicate source path'
        paths.add(path)
        if 'runtime' in source:
            assert source['runtime'] == 'nativeSupervision' and parts[-1] == 'native-supervision.json', 'runtime marker'
            runtime_count += 1
        else:
            assert parts[-1] != 'native-supervision.json' and is_digest(source['sha256']), 'static source digest'
    assert runtime_count <= 1, 'runtime marker count'
    proposal_sha256 = sha(canonical(proposal, 1048576))
    claims = {'version', 'proposalId', 'proposalSha256', 'executionKind', 'nonce',
        'issuedUnixSeconds', 'expiresUnixSeconds', 'hostedEstimateRiskAccepted'}
    assert type(intent) is dict and set(intent) == claims | {'signature'}, 'intent fields'
    assert type(intent['version']) is int and intent['version'] == 1, 'intent version'
    assert type(intent['proposalId']) is str and intent['proposalId'] == proposal['proposalId'] \
        and is_digest(intent['proposalSha256']) and intent['proposalSha256'] == proposal_sha256 \
        and type(intent['executionKind']) is str and intent['executionKind'] == kind, 'intent proposal binding'
    assert is_uuid4(intent['nonce']), 'intent nonce UUIDv4'
    issued, expires = intent['issuedUnixSeconds'], intent['expiresUnixSeconds']
    assert is_u64(issued) and is_u64(expires) and 0 < issued <= verified_at < expires \
        and expires - issued <= 1800, 'original intent lifetime'
    assert type(intent['hostedEstimateRiskAccepted']) is bool \
        and intent['hostedEstimateRiskAccepted'] == (kind == 'hostedSingleRequest'), 'kind risk agreement'
    signature = intent['signature']
    assert type(signature) is list and len(signature) == 64 \
        and all(type(byte) is int and 0 <= byte <= 255 for byte in signature), 'signature byte array'
    intent_sha256 = sha(canonical(intent, 8192))
    payload = b'soundrobe/owner-execution-intent/v1\0' + canonical({field: intent[field] for field in claims}, 8192)
    try:
        with tempfile.TemporaryDirectory(prefix='soundrobe-intent-verification-') as directory:
            directory = pathlib.Path(directory)
            (directory / 'public.der').write_bytes(bytes.fromhex('302a300506032b6570032100') + public_key)
            (directory / 'payload').write_bytes(payload)
            (directory / 'signature').write_bytes(bytes(signature))
            verified = subprocess.run(['openssl', 'pkeyutl', '-verify', '-pubin', '-keyform', 'DER',
                '-inkey', str(directory / 'public.der'), '-rawin', '-in', str(directory / 'payload'),
                '-sigfile', str(directory / 'signature')], timeout=5, stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                env=dict(PATH=os.environ.get('PATH', os.defpath), OPENSSL_CONF=os.devnull))
            assert verified.returncode == 0, 'owner intent signature'
    except (OSError, subprocess.SubprocessError):
        raise AssertionError('local Ed25519 verification unavailable') from None
    return dict(version=1, signatureVerified=True, verifierSha256=expected_verifier_sha256,
        proposalId=proposal['proposalId'], proposalSha256=proposal_sha256, intentSha256=intent_sha256,
        executionKind=kind, nonce=intent['nonce'], issuedUnixSeconds=issued, expiresUnixSeconds=expires,
        hostedEstimateRiskAccepted=intent['hostedEstimateRiskAccepted'], sourceBytesVerified=False,
        executionVerified=False, dispatchVerified=False, liveAuthority=False, userPresenceVerified=False, qualityCredit=0)


def owner_approval_audit(report, approval, public_key, expected_verifier_sha256, verified_at):
    """Verify a frozen native snapshot, not execution, user presence or quality.

    Trust/time must be supplied independently: raw 32-byte Ed25519 public key,
    its expected lowercase SHA-256, and integer Unix seconds. Inputs are parsed
    JSON objects (duplicate keys must already have been rejected by the caller).
    Only bounded, integer-only serde_json-compatible scope values are accepted.
    Invalid evidence or unavailable local OpenSSL raises AssertionError. No
    report paths, private keys, credentials, network or historical auditors are
    consulted; temporary files contain public verification material only.
    """
    if not __debug__:
        raise AssertionError('owner approval audit requires enabled assertions')
    policy = 'disabled-hosted-boundary-v1;native-file;native-response-observation;no-live;no-retry;no-repair'
    sha = lambda data: hashlib.sha256(data).hexdigest()
    is_digest = lambda value: type(value) is str and re.fullmatch('[a-f0-9]{64}', value) is not None
    is_u64 = lambda value: type(value) is int and 0 <= value < 2**64

    def canonical(value, limit=2 * 1048576):
        remaining = 100000

        def validate(item, depth=0):
            nonlocal remaining
            remaining -= 1
            assert remaining >= 0 and depth <= 64, 'scope complexity'
            if item is None or type(item) is bool:
                return
            if type(item) is int:
                assert -2**63 <= item < 2**64, 'scope integer range'
            elif type(item) is str:
                assert len(item) <= limit, 'scope string bound'
            elif type(item) is list:
                assert len(item) <= remaining, 'scope list bound'
                for child in item:
                    validate(child, depth + 1)
            elif type(item) is dict:
                assert len(item) <= remaining, 'scope object bound'
                for key, child in item.items():
                    assert type(key) is str, 'scope object key'
                    validate(key, depth + 1)
                    validate(child, depth + 1)
            else:
                raise AssertionError('scope JSON type; floats unsupported')

        validate(value)
        try:
            encoded = json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(',', ':'), allow_nan=False).encode('utf-8')
        except (ValueError, UnicodeError, RecursionError):
            raise AssertionError('scope JSON encoding') from None
        assert len(encoded) <= limit, 'scope byte bound'
        return encoded

    assert type(public_key) is bytes and len(public_key) == 32, 'public key'
    assert public_key.hex() not in (
        'd75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a',
        '197f6b23e16c8532c6abc838facd5ea789be0c76b2920334039bfa8b3d368d61',
        '4508a07aa941707f3eb2db94c8897a80b2c1197476b6de213ac273df7d86c4ff'), 'fixture verifier'
    assert is_digest(expected_verifier_sha256) and sha(public_key) == expected_verifier_sha256, 'independent verifier digest'
    assert is_u64(verified_at), 'verification time'
    required = {'runId', 'state', 'membership', 'bindings', 'checkpoint', 'preparations', 'sessions', 'counters',
        'operations', 'approvalSha256', 'approvalExpires', 'credentialLookups', 'testPolicy', 'consumptions',
        'mutationAdmissions', 'resultSha256', 'liveAuthority'}
    assert type(report) is dict and required <= set(report), 'report fields'
    assert report['state'] == 'frozen' and report['testPolicy'] == policy, 'frozen disabled snapshot'
    assert type(report['runId']) is str and 0 < len(report['runId']) <= 128, 'run identifier'
    assert report['liveAuthority'] is False, 'report live authority'
    assert report.get('ownerLive') is True, 'live owner lease'
    for key in ('consumptions', 'credentialLookups'):
        assert type(report[key]) is int and report[key] == 0, 'unconsumed preparation'
    # Native preparation mutates disposable profile baselines before freeze.
    # This global counter is not signed scope or proof of approved execution.
    assert type(report['mutationAdmissions']) is int and 0 <= report['mutationAdmissions'] <= 4096, 'preparation mutation bound'
    for key in ('approvalSha256', 'approvalExpires', 'resultSha256'):
        assert report[key] is None, 'already approved or published'
    if 'actualFakeLookups' in report:
        assert type(report['actualFakeLookups']) is int and report['actualFakeLookups'] == 0, 'credential lookup'
    for key in ('membership', 'bindings', 'preparations', 'sessions', 'counters', 'operations'):
        assert type(report[key]) is dict, 'report object'
    checkpoint_bytes = canonical(dict(runId=report['runId'], membership=report['membership'], bindings=report['bindings'],
        preparations=report['preparations'], services=report['sessions'], counters=report['counters']))
    checkpoint = sha(checkpoint_bytes)
    assert is_digest(report['checkpoint']) and report['checkpoint'] == checkpoint, 'checkpoint reconstruction'

    bindings = report['bindings']
    binding = bindings.get('modelRehearsal')
    binding_fields = {'policy', 'model', 'observedTarget', 'providerClaim', 'route', 'requestSha256',
        'verifierSha256', 'liveAuthority', 'boundary'}
    assert type(binding) is dict and set(binding) == binding_fields, 'native nonfixture binding fields'
    assert binding['policy'] == policy and binding['liveAuthority'] is False, 'disabled binding'
    assert binding['verifierSha256'] == expected_verifier_sha256, 'binding verifier'
    assert is_digest(binding['requestSha256']), 'prepared request digest'
    for key in ('model', 'observedTarget'):
        assert type(binding[key]) is str and len(binding[key].encode()) <= 160 \
            and re.fullmatch(r'~?deepseek/[a-zA-Z0-9_.:/-]+', binding[key]), 'model binding'
    assert binding['model'] != binding['observedTarget'], 'alias and target separation'
    assert type(binding['route']) is str and re.fullmatch(r'[a-z0-9_/-]{1,100}', binding['route']) \
        and all(binding['route'].split('/')), 'route binding'
    assert type(binding['providerClaim']) is str and re.fullmatch(r'[a-zA-Z0-9 _/-]{1,100}', binding['providerClaim']), 'provider claim'
    if 'modelRehearsalPlan' in bindings:
        plan = bindings['modelRehearsalPlan']
        assert type(plan) is dict and set(plan) == {'nativePrepared', 'model', 'observedTarget', 'route', 'provider', 'input'}, 'native plan fields'
        assert plan['nativePrepared'] is True, 'native plan marker'
        assert all(plan[key] == binding[key] for key in ('model', 'observedTarget', 'route')) \
            and plan['provider'] == binding['providerClaim'], 'native plan binding'
        source = plan['input']
        assert type(source) is dict and set(source) == {'artist', 'album', 'tracks'}, 'native input fields'
        assert type(source['tracks']) is list and len(source['tracks']) <= 100 \
            and all(type(track) is str for track in source['tracks']), 'native input tracks'
        assert all(source[key] is None or type(source[key]) is str for key in ('artist', 'album')), 'native input credits'
        for text in (source['artist'], source['album'], *source['tracks']):
            assert text is None or (len(text.encode()) <= 2048 \
                and not any(ord(char) < 32 or 127 <= ord(char) <= 159 for char in text)), 'native input text'
    boundary = binding['boundary']
    assert type(boundary) is dict, 'disabled boundary'
    pricing = boundary.get('pricingEvidence')
    assert type(pricing) is dict and set(pricing) == {'promptPicoUsd', 'completionPicoUsd', 'catalogSha256', 'termsSha256'}, 'pricing fields'
    assert all(is_u64(pricing[key]) for key in ('promptPicoUsd', 'completionPicoUsd')), 'pricing integers'
    assert all(is_digest(pricing[key]) for key in ('catalogSha256', 'termsSha256')), 'pricing evidence digests'
    estimate = pricing['promptPicoUsd'] * 8192 + pricing['completionPicoUsd'] * 1024
    assert estimate <= 5_000_000_000_000, 'five dollar text allowance'
    # This native Settings struct has declared order; it is not a Value map.
    settings = dict(model=binding['model'], endpoint='https://openrouter.ai/api/v1', provider='openrouter', source='env_local_only')
    expected_boundary = dict(endpoint='https://openrouter.ai/api/v1/chat/completions',
        settingsSha256=sha(json.dumps(settings, ensure_ascii=False, separators=(',', ':')).encode()),
        pricingEvidence=pricing, textEstimatePicoUsd=estimate, allowanceUsdCents=500, inputTokenIllustration=8192,
        outputTokens=1024, hardBillingCap=False, billingRiskAccepted=False, liveDispatchAvailable=False, readyForExecution=False,
        retention=dict(remote='documented_provider_defaults_for_public_metadata', local='until_result_review'))
    assert canonical(boundary) == canonical(expected_boundary), 'disabled boundary drift'

    names = {'disabled-crowd', 'disabled-billie', 'disabled-enya', 'assistance-billie', 'assistance-enya', 'assistance-crowd'}
    assert set(report['preparations']) == names and len(report['sessions']) == 6, 'six preparations'
    jobs = []
    for session_id, session in report['sessions'].items():
        fields = {'job', 'stage', 'endpoint', 'manifest_sha256', 'client', 'service', 'closed', 'finished',
            'attached', 'sealed', 'native_live', 'fixture_live', 'observation_sha256'}
        assert type(session) is dict and set(session) == fields, 'preparation session fields'
        assert type(session['job']) is str and session['job'] in names and session['stage'] == 'preparation', 'preparation session'
        assert session['client'] == session_id and is_digest(session['manifest_sha256']), 'session binding'
        for key in ('endpoint', 'service'):
            assert type(session[key]) is str and 0 < len(session[key]) <= 2048, 'session string'
        assert all(session[key] is True for key in ('closed', 'finished', 'attached', 'sealed')), 'finished preparation'
        assert session['native_live'] is False and session['fixture_live'] is False \
            and session['observation_sha256'] is None, 'closed preparation leases'
        jobs.append(session['job'])
    assert set(jobs) == names, 'unique preparation jobs'
    for name, prepared in report['preparations'].items():
        assert type(prepared) is dict and set(prepared) == {'preparationSha256', 'requestSha256'}, 'preparation fields'
        assert is_digest(prepared['preparationSha256']), 'preparation digest'
        assert prepared['requestSha256'] == (binding['requestSha256'] if name == 'assistance-crowd' else None), 'prepared request binding'
    counters = dict(provider=0, inventory=0, model=0, reserved=0, arrived=0, completed=0)
    assert len(report['operations']) <= 100000, 'operation bound'
    for operation in report['operations'].values():
        fields = {'session', 'category', 'request_sha256', 'dispatched', 'arrived', 'completed'}
        assert type(operation) is dict and set(operation) == fields, 'preparation operation fields'
        assert type(operation['session']) is str and operation['session'] in report['sessions'], 'operation session'
        assert type(operation['category']) is str and operation['category'] in ('provider', 'inventory'), 'no model operation'
        assert is_digest(operation['request_sha256']), 'operation digest'
        assert all(operation[key] is True for key in ('dispatched', 'arrived', 'completed')), 'settled preparation operation'
        counters[operation['category']] += 1
        for key in ('reserved', 'arrived', 'completed'):
            counters[key] += 1
    assert canonical(report['counters']) == canonical(counters), 'reconstructed preparation counters'
    scope = dict(runId=report['runId'], checkpoint=checkpoint, bindings=bindings, membership=report['membership'],
        preparations=report['preparations'], policy=policy)
    scope_sha256 = sha(canonical(scope))
    assert type(approval) is dict and set(approval) == {'payload', 'signature'}, 'approval fields'
    payload, signature = approval['payload'], approval['signature']
    fields = {'version', 'runId', 'checkpoint', 'scopeSha256', 'nonce', 'expiresUnixSeconds'}
    assert type(payload) is dict and set(payload) == fields, 'payload fields'
    assert type(payload['version']) is int and payload['version'] == 1, 'payload version'
    assert payload['runId'] == report['runId'] and payload['checkpoint'] == checkpoint \
        and payload['scopeSha256'] == scope_sha256, 'signed snapshot binding'
    assert type(payload['nonce']) is str and len(payload['nonce']) <= 64, 'nonce type'
    try:
        uuid.UUID(payload['nonce'])
    except ValueError:
        raise AssertionError('nonce UUID') from None
    expiry = payload['expiresUnixSeconds']
    assert is_u64(expiry) and verified_at < expiry <= verified_at + 1800, 'approval expiry'
    assert type(signature) is list and len(signature) == 64 \
        and all(type(byte) is int and 0 <= byte <= 255 for byte in signature), 'signature byte array'
    payload_bytes = canonical(payload, 4096)
    approval_sha256 = sha(canonical(approval, 8192))
    try:
        with tempfile.TemporaryDirectory(prefix='soundrobe-owner-verification-') as directory:
            directory = pathlib.Path(directory)
            # RFC 8410 SubjectPublicKeyInfo prefix followed by the raw public key.
            (directory / 'public.der').write_bytes(bytes.fromhex('302a300506032b6570032100') + public_key)
            (directory / 'payload').write_bytes(payload_bytes)
            (directory / 'signature').write_bytes(bytes(signature))
            verified = subprocess.run(['openssl', 'pkeyutl', '-verify', '-pubin', '-keyform', 'DER',
                '-inkey', str(directory / 'public.der'), '-rawin', '-in', str(directory / 'payload'),
                '-sigfile', str(directory / 'signature')], timeout=5, stdin=subprocess.DEVNULL,
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL,
                env=dict(PATH=os.environ.get('PATH', os.defpath), OPENSSL_CONF=os.devnull))
            assert verified.returncode == 0, 'owner approval signature'
    except (OSError, subprocess.SubprocessError):
        raise AssertionError('local Ed25519 verification unavailable') from None
    return dict(version=1, signatureVerified=True, verifierSha256=expected_verifier_sha256,
        approvalSha256=approval_sha256, checkpoint=checkpoint, scopeSha256=scope_sha256, expiresUnixSeconds=expiry,
        liveAuthority=False, userPresenceVerified=False, qualityCredit=0)


def http_response_audit(request_bytes, response_bytes, receipt, expected_model, expected_provider, transport):
    """Rebuild fully received HTTP claims, never delivery, identity or authority.

    Invalid response content requires a rejected/null projection. Invalid byte
    bounds or receipt evidence raises AssertionError; partial responses belong
    to the transport and cannot be audited here. No filesystem or network I/O.
    """
    assert type(request_bytes) is bytes and len(request_bytes) <= 32768, 'request bytes'
    assert type(response_bytes) is bytes and len(response_bytes) <= 65536, 'response bytes'
    assert type(transport) is str and transport in ('offline_local_http', 'hosted_https'), 'transport'
    assert type(expected_model) is str and type(expected_provider) is str, 'expected claims'
    fields = {'sha256', 'bytes', 'status', 'identityClaimsVerified', 'pricingVerified',
        'usageClaimsVerified', 'observation', 'remoteArrivalVerified', 'http'}
    assert type(receipt) is dict and set(receipt) == fields, 'receipt fields'
    assert type(receipt['bytes']) is int and receipt['bytes'] == len(response_bytes) \
        and receipt['sha256'] == hashlib.sha256(response_bytes).hexdigest(), 'response binding'
    assert type(receipt['status']) is int and 100 <= receipt['status'] <= 599, 'status'
    assert all(receipt[key] is False for key in ('identityClaimsVerified', 'pricingVerified',
        'usageClaimsVerified', 'remoteArrivalVerified')), 'verification flags'
    assert receipt['observation'] == 'native_http_response', 'observation'

    def invalid_json_number(_):
        raise ValueError('non-JSON number')

    def json_float(raw):
        value = float(raw)
        if not math.isfinite(value):
            raise ValueError('out-of-range JSON number')
        return value

    def json_integer(raw):
        # Native serde_json represents lexical -0 as a float, not u64 evidence.
        json_float(raw)
        return -0.0 if raw == '-0' else int(raw)

    def unique_answer(pairs):
        value = dict(pairs)
        assert len(value) == len(pairs), 'duplicate answer field'
        return value

    def derive_projection():
        assert receipt['status'] == 200
        value = json.loads(response_bytes.decode('utf-8'), parse_constant=invalid_json_number,
            parse_float=json_float, parse_int=json_integer)
        assert type(value) is dict
        assert type(value['model']) is str and value['model'] == expected_model
        assert type(value['provider']) is str and value['provider'] == expected_provider
        choices = value['choices']
        assert type(choices) is list and len(choices) == 1
        choice = choices[0]
        assert type(choice) is dict and choice['finish_reason'] == 'stop'
        message = choice['message']
        assert type(message) is dict
        assert message.get('refusal') is None and message.get('function_call') is None
        tools = message.get('tool_calls')
        assert tools is None or (type(tools) is list and not tools)
        for key in ('reasoning', 'reasoning_content'):
            assert message.get(key) is None or (type(message[key]) is str and message[key] == '')
        content = message['content']
        assert type(content) is str and len(content.encode('utf-8')) <= 4096
        answer = json.loads(content, parse_constant=invalid_json_number,
            parse_float=json_float, parse_int=json_integer, object_pairs_hook=unique_answer)
        assert type(answer) is dict and set(answer) == {'genre', 'confidence'}
        genre, confidence = answer['genre'], answer['confidence']
        assert type(genre) is str and len(genre.encode('utf-8')) <= 256
        assert not any(ord(c) < 32 or 127 <= ord(c) <= 159 for c in genre)
        assert type(confidence) in (int, float) and 0 <= confidence <= 1 and math.isfinite(confidence)
        usage = value['usage']
        assert type(usage) is dict
        prompt, completion, total = (usage[k] for k in ('prompt_tokens', 'completion_tokens', 'total_tokens'))
        assert all(type(n) is int for n in (prompt, completion, total))
        assert 0 <= prompt <= 8192 and 0 <= completion <= 1024 and total == prompt + completion
        if 'completion_tokens_details' in usage:
            details = usage['completion_tokens_details']
            assert type(details) is dict
            reasoning = details.get('reasoning_tokens', 0)
            assert type(reasoning) is int and reasoning == 0
        return dict(model=value['model'], provider=value['provider'],
            genre=genre if confidence >= 0.6 else '', confidence=confidence,
            usage=dict(promptTokens=prompt, completionTokens=completion, totalTokens=total, reasoningTokens=0))

    try:
        projection = derive_projection()
    except (AssertionError, KeyError, TypeError, ValueError, UnicodeError, RecursionError):
        projection = None
    validation = 'rejected' if projection is None else ('accepted' if projection['genre'] else 'withheld')
    expected = dict(version=1, transport=transport,
        requestSha256=hashlib.sha256(request_bytes).hexdigest(), validation=validation, projection=projection)

    def matches(actual, wanted, confidence=False):
        # Python's ordinary equality confuses False with 0 and True with 1.
        # Only confidence is a general number; other evidence numbers are ints.
        if confidence:
            return type(actual) in (int, float) and actual == wanted
        if type(actual) is not type(wanted):
            return False
        if type(wanted) is dict:
            return set(actual) == set(wanted) and all(matches(actual[k], v, k == 'confidence')
                for k, v in wanted.items())
        return actual == wanted

    assert matches(receipt['http'], expected), 'http receipt'
    return expected


def owner_offline_response_bytes(run, plan, sources):
    """Read only an exact public sibling response lock, never remote evidence.

    Callers establish signed source order separately. No lock retains the
    historical default response; a nearby file alone cannot select bytes.
    """
    response_sources = [s for s in sources if pathlib.Path(s['path']).name == 'response-fixture.json']
    if not response_sources:
        return None
    canonical = lambda value: json.dumps(value, ensure_ascii=False, sort_keys=True,
        separators=(',', ':'), allow_nan=False).encode()
    plans = []
    for source in sources:
        file = ROOT / source['path']
        if file.suffix != '.json' or file.name == 'response-fixture.json':
            continue
        if canonical(load(file)) == canonical(plan):
            plans.append(source)
    assert len(plans) == 1, 'owner public plan source must be unique'
    plan_source = plans[0]
    public_plan = ROOT / plan_source['path']
    fixture = public_plan.with_name('response-fixture.json')
    matches = [s for s in sources if (ROOT / s['path']).resolve() == fixture.resolve()]
    if not matches:
        return None
    assert public_plan.name == 'public-native-plan.json', 'owner response public native plan name'
    assert public_plan.resolve().is_relative_to(ROOT / '.amp/in/artifacts') \
        and not public_plan.resolve().is_relative_to(run.resolve()), 'owner response case artifact location'
    for file, lock in ((public_plan, plan_source), (fixture, matches[0])):
        relative = file.relative_to(ROOT)
        assert lock['path'] == relative.as_posix() and '..' not in relative.parts, 'owner response canonical source path'
        assert not any((ROOT / pathlib.Path(*relative.parts[:i])).is_symlink()
            for i in range(1, len(relative.parts) + 1)), 'owner response source symlink'
        info = file.stat()
        assert stat.S_ISREG(info.st_mode) and info.st_nlink == 1, 'owner response regular single-link file'
    assert len(matches) == 1, 'owner response must be source-locked exactly once'
    assert digest(public_plan) == plan_source['sha256'], 'owner public plan hash drift'
    assert fixture.stat().st_size <= 65536, 'owner response bounded file'
    with fixture.open('rb') as source:
        response_bytes = source.read(65537)
    assert len(response_bytes) <= 65536 and hashlib.sha256(response_bytes).hexdigest() == matches[0]['sha256'], 'owner response hash drift'
    return response_bytes


def model_rehearsal_audit(run, receipt, requested, *, _owner_public_key=None):
    """Join rehearsal evidence only; never establish completion or live authority."""
    owner_offline = _owner_public_key is not None
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
    offline_http = 'offlineHttp' in plan
    offline_response = 'offlineResponse' in plan
    assert set(plan) == ({'model', 'observedTarget', 'route', 'provider', 'input'}
        | ({'nativePrepared'} if owner_offline else set())
        | ({'offlineHosted'} if hosted else set()) | ({'offlineHttp'} if offline_http else set())
        | ({'offlineResponse'} if offline_response else set())), 'model rehearsal plan fields'
    if owner_offline:
        assert plan['nativePrepared'] is True and not (hosted or offline_http or offline_response), 'owner nonfixture plan'
        assert type(_owner_public_key) is bytes and len(_owner_public_key) == 32, 'owner verifier'
        assert bindings['transport']['endpoint'] == 'native-owned-loopback-http/api/v1/chat/completions', 'owner offline transport'
    if offline_response:
        assert hosted and plan.get('offlineHttp') is True, 'offline response requires offline HTTP hosted mode'
        metadata = plan['offlineResponse']
        assert type(metadata) is dict and set(metadata) == {'sha256', 'status'}, 'offline response fields'
        assert type(metadata['sha256']) is str and re.fullmatch('[a-f0-9]{64}', metadata['sha256']), 'offline response digest'
        assert type(metadata['status']) is int and 100 <= metadata['status'] <= 599, 'offline response status'
    if offline_http:
        assert hosted and plan['offlineHttp'] is True, 'offline HTTP requires explicit hosted fixture plan'
        assert bindings['modelRehearsal']['offlineHttp'] is True, 'offline HTTP immutable mode binding'
        assert bindings['transport']['endpoint'] == 'native-owned-loopback-http/api/v1/chat/completions', 'offline HTTP transport binding'
    if hosted:
        policy = disabled_policy
        assert bindings['modelRehearsal']['offlineFixture'] is True
    if owner_offline:
        policy = disabled_policy
    assert set(plan['input']) == {'artist', 'album', 'tracks'}, 'model rehearsal input fields'
    locked_plans = []
    for source in bindings['sourcesAndTools']:
        file = ROOT / source['path']
        # Raw response JSON may be malformed; validate its file before reading.
        if file.suffix != '.json' or ((offline_response or owner_offline) and file.name == 'response-fixture.json'):
            continue
        public = load(file)
        if public == plan:
            assert not offline_response or canonical(public) == canonical(plan), 'offline response public plan types'
            assert file.resolve().is_relative_to(ROOT) and not pathlib.Path(source['path']).is_absolute()
            assert not offline_http or public.get('offlineHttp') is True, 'offline HTTP public plan marker'
            assert digest(file) == source['sha256'], 'model rehearsal public plan hash drift'
            locked_plans.append(source['path'])
    assert len(locked_plans) == 1, 'model rehearsal public plan must be source-locked exactly once'
    owner_response = owner_offline_response_bytes(run, plan, bindings['sourcesAndTools']) if owner_offline else None
    if offline_response:
        public_plan = ROOT / locked_plans[0]
        assert not public_plan.is_symlink() and stat.S_ISREG(public_plan.stat().st_mode), 'public plan regular file'
        assert not public_plan.resolve().is_relative_to(run.resolve()), 'public plan inside run root'
        fixture = public_plan.with_name('response-fixture.json')
        assert fixture.resolve().is_relative_to(ROOT) and not fixture.resolve().is_relative_to(run.resolve()), 'offline response location'
        relative = fixture.relative_to(ROOT)
        assert '..' not in relative.parts and not any((ROOT / pathlib.Path(*relative.parts[:i])).is_symlink()
            for i in range(1, len(relative.parts) + 1)), 'offline response symlink'
        assert stat.S_ISREG(fixture.stat().st_mode) and fixture.stat().st_size <= 65536, 'offline response regular bounded file'
        locked_responses = [s for s in bindings['sourcesAndTools'] if (ROOT / s['path']).resolve() == fixture.resolve()]
        assert len(locked_responses) == 1, 'offline response must be source-locked exactly once'
        assert locked_responses[0]['path'] == relative.as_posix(), 'offline response checkout-relative source lock'
        with fixture.open('rb') as source:
            fixture_bytes = source.read(65537)
        assert len(fixture_bytes) <= 65536 and sha(fixture_bytes) == metadata['sha256'] == locked_responses[0]['sha256'], 'offline response hash drift'
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
    verifier = _owner_public_key if owner_offline else bytes.fromhex(
        '4508a07aa941707f3eb2db94c8897a80b2c1197476b6de213ac273df7d86c4ff' if hosted
        else '197f6b23e16c8532c6abc838facd5ea789be0c76b2920334039bfa8b3d368d61')
    binding = dict(policy=policy, model=plan['model'], observedTarget=plan['observedTarget'],
        providerClaim=plan['provider'], route=plan['route'], requestSha256=request_sha,
        verifierSha256=sha(verifier), liveAuthority=False)
    if offline_http:
        binding['offlineHttp'] = True
    if offline_response:
        binding['offlineResponse'] = metadata
    if hosted or owner_offline:
        if hosted:
            binding['offlineFixture'] = True
        pricing = bindings['modelRehearsal']['boundary']['pricingEvidence'] if owner_offline else plan['offlineHosted']
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
    # Only this explicitly selected branch uses the native-owned HTTP fixture
    # without fixture markers in the signed preparation/context.
    if owner_offline:
        hosted = offline_http = True
    assert receipt['testPolicy'] == policy
    assert selection['modelRehearsalPlan'] == plan and selection['model'] == plan['model']
    if offline_response or owner_offline:
        assert canonical(selection['modelRehearsalPlan']) == canonical(plan), 'offline response selection drift'
    assert owner_offline or not offline_http or selection['modelRehearsalPlan'].get('offlineHttp') is True, 'offline HTTP selection marker'
    names = [e['name'] for e in requested['evaluations']]
    assert len(names) == len(set(names)) == 6 and 'assistance-crowd' in names
    assert set(receipt['preparations']) == set(names)
    for name, stage, folder, descriptor in descriptors:
        assert descriptor['model_rehearsal'] == plan, f'{name}/{folder}: rehearsal descriptor drift'
        if offline_response or owner_offline:
            assert canonical(descriptor['model_rehearsal']) == canonical(plan), f'{name}/{folder}: offline response descriptor drift'
        assert owner_offline or not offline_http or descriptor['model_rehearsal'].get('offlineHttp') is True, f'{name}/{folder}: offline HTTP descriptor marker'
        assert descriptor['job'] == name and descriptor['stage'] == stage
    answer = dict(genre='MOCK_ACCEPTED_GENRE_CANARY', confidence=0.8)
    usage = dict(prompt_tokens=7, completion_tokens=3, total_tokens=10)
    response_encoder = canonical if hosted else compact
    response_bytes = response_encoder(dict(model=plan['observedTarget'], provider=plan['provider'],
        choices=[dict(finish_reason='stop', message=dict(content=response_encoder(answer).decode()))],
        usage=usage, diagnostic='MOCK_RAW_DIAGNOSTIC_CANARY'))
    response_status = 200
    if offline_response:
        response_bytes, response_status = fixture_bytes, metadata['status']
    if owner_response is not None:
        response_bytes = owner_response
    response_sha = sha(response_bytes)
    model_operations = {k: v for k, v in receipt['operations'].items() if v['category'] == 'model'}
    assert len(model_operations) == receipt['counters']['model'] == receipt['credentialLookups'] == 1
    if offline_response or owner_offline:
        assert type(receipt['counters']['model']) is int and type(receipt['credentialLookups']) is int, 'model count types'
    operation_id, operation = next(iter(model_operations.items()))
    assert all(operation[k] is True for k in ['dispatched', 'completed']) and operation['arrived'] is (not hosted)
    assert operation['request_sha256'] == sha(compact(['POST', '/model/chat/completions', request_sha]))
    if owner_offline:
        assert operation['send_admitted'] is True, 'owner model send admission'
    response = dict(sha256=response_sha, bytes=len(response_bytes), status=response_status,
        identityClaimsVerified=False, pricingVerified=False, usageClaimsVerified=False)
    if hosted:
        response.update(observation='offline_injected_native_response', remoteArrivalVerified=False)
    if offline_http:
        # Derive from public bytes and the actual Journal receipt, not the
        # fixture's prepared-response claims or the driver's selected export.
        http = http_response_audit(request_bytes, response_bytes, operation['response'],
            plan['observedTarget'], plan['provider'], 'offline_local_http')
        response.update(observation='native_http_response', http=http)
        observation = dict(version=1, transport='offline_local_http', requests=1,
            requestSha256=request_sha, requestBytes=len(request_bytes),
            responseSha256=response_sha, responseBytes=len(response_bytes), status=response_status,
            exactRequest=True, fakeCredentialPresented=True, liveAuthority=False)
        assert canonical(load(run / 'offline-http-observation.json')) == canonical(observation), 'offline HTTP fixture observation drift'
        assert http['validation'] != 'rejected', 'rejected response cannot validate a completed rehearsal'
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
            if offline_response or owner_offline:
                assert type(execution['modelRequests']) is int and type(execution['wireArrivals']) is int, 'summary count types'
        rows = load(directory / 'cold/records.json')
        assert len(rows) == 1
        row = rows[0]
        candidate = row['native'].get('candidate') or {}
        genre = http['projection']['genre'] if offline_response or owner_response is not None else answer['genre']
        assisted = dict(model=plan['model'], input_sha256=sha(canonical(payload)), value=genre) if active and genre else None
        assert candidate.get('assisted_genre') == row['persistedEvidence']['assistedGenre'] == assisted, name
        if active:
            if genre:
                assert candidate['genre'] == genre
                assert row['fieldObservations']['tracks'] and all(t['genre'] == genre for t in row['fieldObservations']['tracks'])
            else:
                assert candidate.get('genre') in (None, ''), 'withheld response invented candidate genre'
                assert row['fieldObservations']['tracks'] and all(t['genre'] in (None, '') for t in row['fieldObservations']['tracks']), 'withheld response invented readback genre'
            if offline_response or owner_offline:
                assert row['serviceFailureObserved'] is False, 'completed rehearsal service failure'
                assert [d for d in row['native']['diagnostics'] if d.get('stage') == 'genre'] == [
                    dict(stage='genre', code='genre_assisted' if genre else 'genre_rejected')], 'genre diagnostic drift'
        assert load(directory / 'scoring-contract.json')['spec']['mode'] == 'mock', name
        score = load(directory / 'score.provisional.json')
        assert score['groundedAssistanceAccuracyCredit'] == 0 and score['writableDelta'] == [] and not score['acceptedCertificate']
        if offline_response or owner_offline:
            assert type(score['groundedAssistanceAccuracyCredit']) is int and score['acceptedCertificate'] is False, 'mock credit types'
    result = dict(policy=policy, requestSha256=request_sha, responseSha256=response_sha,
        model=plan['model'], observedTarget=plan['observedTarget'], providerClaim=plan['provider'],
        verifierSha256=sha(verifier), preparedJobs=6, modelRequests=1,
        identityClaimsVerified=False, groundedAssistanceAccuracyCredit=0, liveAuthority=False)
    if offline_response:
        result['http'] = http
    if owner_offline:
        result.update(http=http, responseEvidence='public_offline_fixture_bytes_not_remote_response_evidence'
            if owner_response is not None else 'deterministic_default_fixture_reconstruction_not_retained_raw_bytes')
    return result


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


def retained_auditor_source(hosted=False, offline_http=False, offline_response=False):
    """Count every adaptation; never modify the sealed historical sources."""
    assert not offline_http or hosted, 'offline HTTP requires hosted fixture adaptations'
    assert not offline_response or (hosted and offline_http), 'offline response requires HTTP adaptations'
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
    if offline_response:
        # No canary/usage constants occur in the sealed auditors themselves.
        # Their implicit fixed-response assumptions are present provenance and
        # zero accepted readbacks. Preserve independent scorer recomputation.
        native_replacements += [
            ("assert row['persistedEvidence']['assistedGenre'] == row['native']['candidate']['assisted_genre']",
             "assert row['persistedEvidence']['assistedGenre'] == row['native']['candidate'].get('assisted_genre')"),
            ("assert score['userAcceptance']['agreeingFieldSlots'] == 0",
             "assert score['userAcceptance']['agreeingFieldSlots'] == sum(f['userAcceptanceAgreement'] is True for f in score['fields'])"),
            ("userAcceptanceRequested=22, userAcceptanceAgreeing=0, groundedAssistanceAccuracyCredit=0",
             "userAcceptanceRequested=22, userAcceptanceAgreeing=sum(load(run / e['name'] / 'score.provisional.json')"
             "['userAcceptance']['agreeingFieldSlots'] for e in requested['evaluations']), groundedAssistanceAccuracyCredit=0"),
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
    if offline_http:
        old = "assert operation['response']['observation'] == 'offline_injected_native_response'"
        new = "assert operation['response']['observation'] == 'native_http_response'"
        assert source.count(old) == 1, old
        source = source.replace(old, new)
    return original, source


def owner_offline_retained_auditor_source(rehearsal, plan):
    """Two native and one durable owner-only adaptations; sealed sources unchanged.

    The caller supplies the independently audited response and bound plan, not
    review/provenance claims from a row. A withheld answer is a completed mock
    observation with no actionable review, not an accepted genre or quality.
    """
    assert rehearsal['http']['validation'] in ('accepted', 'withheld'), 'completed owner response validation'
    genre = rehearsal['http']['projection']['genre']
    assert bool(genre) == (rehearsal['http']['validation'] == 'accepted'), 'owner response genre validation'
    assert rehearsal['model'] == plan['model'], 'owner response model binding'
    assisted = dict(model=rehearsal['model'], input_sha256=hashlib.sha256(json.dumps(
        plan['input'], ensure_ascii=False, sort_keys=True, separators=(',', ':')).encode()).hexdigest(),
        value=genre) if genre else None
    original, source = retained_auditor_source(True, True)
    replacements = [
        ("assert row['reviewPendingAfterRestart'] and row['actionableReviewAfterRestart']",
         f"assert row['reviewPendingAfterRestart'] is {bool(genre)!r} and row['actionableReviewAfterRestart'] is {bool(genre)!r}"),
        ("assert row['persistedEvidence']['assistedGenre'] == row['native']['candidate']['assisted_genre']",
         f"assert row['persistedEvidence']['assistedGenre'] == row['native']['candidate'].get('assisted_genre') == {assisted!r}"),
    ]
    nested = ''
    for old, new in replacements:
        nested += f'assert source.count({old!r}) == 1\nsource = source.replace({old!r}, {new!r})\n'
    insertion = 'old = "directory / \'cold/preparation.json\'"'
    assert source.count(insertion) == 1, 'owner native adaptation insertion'
    source = source.replace(insertion, nested + insertion)
    old = "assert operation['dispatched'] is True and operation['completed'] is True and operation['arrived'] is False"
    assert source.count(old) == 1, 'owner durable admission adaptation'
    source = source.replace(old, old + " and operation['send_admitted'] is True")
    return original, source


def owner_offline_completed_audit(run, *, proposal, intent, approval, public_key,
                                  expected_verifier_sha256, verified_at, trusted_executable):
    """Independently audit a completed OWNER OFFLINE rehearsal, never hosted.

    proposal, intent and approval are caller-supplied public JSON objects with
    duplicate keys rejected before parsing. Trust is raw 32-byte Ed25519 public
    key bytes, a separately pinned lowercase SHA-256 and integer Unix seconds
    at signature verification (not inferred from a run file or current clock).
    trusted_executable is the independently selected canonical evaluator path,
    never a value taken from signed source declarations or file permissions.
    The approval object is supplied from outside the run; no provisioned owner
    directory, signer, private key or credential configuration is discovered.

    Both existing signature auditors run, then actual source bytes, bootstrap,
    consumed coordination and final receipt are joined before the general
    completion/scoring/worker/storage audits. Missing evidence fails closed.
    Locked public fixture bytes are re-read when present; otherwise default
    response bytes are reconstructed. Neither is retained remote evidence.
    Signatures do not prove human presence, nonce-file custody or live authority.
    """
    if not __debug__:
        raise AssertionError('owner completed audit requires enabled assertions')
    run = pathlib.Path(run).resolve()
    assert run.is_relative_to(ROOT / '.amp/in/artifacts') and run != ROOT / '.amp/in/artifacts', 'owner run root'
    canonical = lambda value: json.dumps(value, ensure_ascii=False, sort_keys=True,
        separators=(',', ':'), allow_nan=False).encode()
    sha = lambda value: hashlib.sha256(canonical(value)).hexdigest()

    def public_file(file, limit=8 * 1048576):
        assert file.resolve().is_relative_to(ROOT) and not file.is_symlink() \
            and stat.S_ISREG(file.stat().st_mode) and file.stat().st_size <= limit, 'bounded public evidence file'
        def unique(pairs):
            value = dict(pairs)
            assert len(value) == len(pairs), 'duplicate public evidence field'
            return value
        return json.loads(file.read_bytes(), object_pairs_hook=unique)

    intent_check = owner_execution_intent_audit(proposal, intent, public_key, expected_verifier_sha256, verified_at)
    assert intent_check['executionKind'] == 'offlineRehearsal', 'hosted execution is disabled'
    frozen = public_file(run / 'frozen-checkpoint.json')
    approval_check = owner_approval_audit(frozen, approval, public_key, expected_verifier_sha256, verified_at)
    assert approval['payload']['nonce'] == intent_check['nonce'], 'owner approval nonce differs from signed intent'
    assert approval_check['expiresUnixSeconds'] == intent_check['expiresUnixSeconds'], 'owner approval expiry differs from signed intent'
    bindings = frozen['bindings']
    plan, context = bindings['modelRehearsalPlan'], bindings['modelRehearsal']
    assert plan['nativePrepared'] is True and bindings['disposableRunRoot'] == str(run), 'native owner preparation root'
    claimed = {key: intent_check[key] for key in ('version', 'proposalId', 'proposalSha256',
        'intentSha256', 'executionKind', 'nonce', 'expiresUnixSeconds')}
    assert canonical(bindings['ownerExecution']) == canonical(claimed), 'claimed owner binding'
    packet = public_file(run / 'owner-execution-bootstrap.json', 1048576)
    assert type(packet) is dict and set(packet) == {'version', 'proposal', 'intent', 'ownerExecution', 'offlineCredential'} \
        and type(packet['version']) is int and packet['version'] == 1, 'owner bootstrap fields'
    assert canonical(packet['proposal']) == canonical(proposal) and canonical(packet['intent']) == canonical(intent) \
        and canonical(packet['ownerExecution']) == canonical(claimed), 'public owner bootstrap join'
    requested = public_file(run / 'requested-membership.json')
    assert canonical(requested) == canonical(frozen['membership']) == canonical(public_file(
        ROOT / 'test/fixtures/tauri/auto-tag-eval/supervised-requested-membership.json')), 'full native membership'
    for key, value in dict(executableSha256=bindings['executable'], membershipSha256=sha(requested),
            nativePlanSha256=sha(plan), requestSha256=context['requestSha256'],
            verifierSha256=expected_verifier_sha256, boundaryPricingSha256=sha(context['boundary']),
            contextSha256=sha(context)).items():
        assert proposal[key] == value, 'owner proposal actual scope: ' + key

    runtime = (run / 'native-supervision.json').relative_to(ROOT).as_posix()
    locks = bindings['sourcesAndTools']
    assert type(locks) is list and len(locks) == len(proposal['sourcesAndTools']), 'full owner source list'
    credential = packet['offlineCredential']
    assert type(credential) is dict and set(credential) == {'path', 'sha256'} \
        and sum(canonical(lock) == canonical(credential) for lock in locks) == 1, 'public offline credential lock'
    credential_file = ROOT / credential['path']
    assert credential_file.resolve().is_relative_to(ROOT / '.amp/in/artifacts') \
        and not credential_file.resolve().is_relative_to(run) \
        and credential_file.name == '.env.local', 'only public fake credential fixture'
    expected_credential = f"LLM_API_KEY=MOCK_ONLY_SECRET_CANARY\nLLM_MODEL={plan['model']}\n".encode()
    assert credential['sha256'] == hashlib.sha256(expected_credential).hexdigest(), 'public fake credential expected lock'
    public_plans = [ROOT / lock['path'] for lock in locks
        if pathlib.PurePosixPath(lock['path']).name == 'public-native-plan.json']
    assert len(public_plans) == 1, 'owner public plan source must be unique'
    public_plan = public_plans[0]
    assert public_plan.is_relative_to(ROOT / '.amp/in/artifacts') \
        and not public_plan.is_relative_to(run), 'owner public plan artifact location'
    public_files = {'src-tauri/Cargo.toml', 'src-tauri/Cargo.lock',
        'scripts/auto-tag-eval-replay/run-bounded.cjs', 'scripts/auto-tag-eval-replay/run-supervised.cjs',
        'scripts/auto-tag-eval-replay/mock-authority.cjs', 'scripts/auto-tag-eval-replay/bounded-storage.cjs',
        'scripts/auto-tag-eval-replay/score-observations.py', 'scripts/auto-tag-eval-replay/audit-supervised.py',
        'scripts/mock-provider-service.cjs', 'scripts/bounded_storage.py', 'scripts/review_auto_tag_subset.py'}
    label_roots = ('.planning/plans/crowd-lu-user-acceptance-v1-20261111/',
        '.planning/plans/crowd-lu-annotation-sheet-20261111/',
        '.planning/plans/scorable-assistance-contract-20261111/')
    label_files = {'.planning/research/bounded-assistance-machine-labels-20261111.json',
        '.planning/research/bounded-assistance-label-review-20261111.json',
        '.amp/in/eval-dummies/artist-batch-extension/corpus.json', '.amp/in/eval-dummies/original/corpus.json'}
    # Executable identity is independent audit input, not a signed claim or mode.
    trusted_executable = pathlib.Path(trusted_executable)
    assert trusted_executable.is_absolute() and trusted_executable == trusted_executable.resolve() \
        and trusted_executable.is_relative_to(ROOT), 'trusted evaluator path'
    assert locks and ROOT / locks[0]['path'] == trusted_executable, 'trusted evaluator source slot'
    normalized, seen = [], set()
    for index, lock in enumerate(locks):
        assert type(lock) is dict and set(lock) == {'path', 'sha256'}, 'resolved source lock'
        relative = pathlib.PurePosixPath(lock['path'])
        assert not relative.is_absolute() and relative.as_posix() == lock['path'] \
            and all(part not in ('.', '..') for part in relative.parts) and lock['path'] not in seen, 'canonical source path'
        seen.add(lock['path'])
        file = ROOT / lock['path']
        # Match native public roots and exact input slots before any source
        # contents are opened. A signature is not permission to read secrets.
        public = not any(part.startswith('.') for part in relative.parts) and (
            (lock['path'].startswith('src-tauri/src/') and file.suffix == '.rs')
            or (lock['path'].startswith('test/fixtures/tauri/') and file.suffix == '.json')
            or lock['path'] in public_files)
        label = lock['path'] in label_files or any(
            lock['path'].startswith(prefix) and len(relative.parts) == len(pathlib.PurePosixPath(prefix).parts) + 1
            and not file.name.startswith('.') and (file.name == 'SHA256SUMS' or file.suffix in ('.json', '.md', '.py'))
            for prefix in label_roots)
        response, wire = file == public_plan.with_name('response-fixture.json'), file == public_plan.with_name('response-wire.json')
        executable = index == 0 and file == trusted_executable and lock['sha256'] == bindings['executable']
        assert not file.name.startswith('.env') or canonical(lock) == canonical(credential), 'not a public credential source'
        assert public or label or executable or file == public_plan or response or wire \
            or lock['path'] == runtime or canonical(lock) == canonical(credential), 'owner public source eligibility'
        assert not any((ROOT / pathlib.Path(*relative.parts[:i])).is_symlink()
            for i in range(1, len(relative.parts) + 1)), 'source symlink'
        info = file.stat()
        limit = 1024 if wire else 65536 if response or file == public_plan else 512 * 1048576
        assert file.resolve().is_relative_to(ROOT) and stat.S_ISREG(info.st_mode) \
            and info.st_nlink == 1 and info.st_size <= limit, 'source bounded regular single-link file'
        normalized.append(dict(path=runtime, runtime='nativeSupervision') if lock['path'] == runtime else lock)
    assert runtime in seen and canonical(normalized) == canonical(proposal['sourcesAndTools']), 'source locks/order/runtime join'
    assert canonical(public_file(public_plan, 65536)) == canonical(plan), 'owner public plan contents'
    owner_offline_response_bytes(run, plan, locks)
    for lock in locks:
        assert digest(ROOT / lock['path']) == lock['sha256'], 'actual owner source bytes: ' + lock['path']
    assert locks[0]['sha256'] == bindings['executable'], 'actual executable source lock'
    assert stat.S_IMODE(credential_file.stat().st_mode) == 0o600, 'public fake credential mode'
    assert credential_file.read_bytes() == expected_credential, 'public fake credential bytes'

    coordination = public_file(run / 'owner-approval-coordination.json')
    expected_coordination = {key: frozen[key] for key in ('runId', 'checkpoint', 'bindings', 'membership', 'preparations', 'counters', 'credentialLookups')}
    expected_coordination.update(componentCoordinationOnly=True, liveAuthority=False, humanPresenceVerified=False,
        qualityCertificate=False, successCertificate=False, approvalSha256=approval_check['approvalSha256'],
        approvalExpires=approval_check['expiresUnixSeconds'], consumptions=1, state='executing', actualFakeLookups=0)
    assert canonical(coordination) == canonical(expected_coordination), 'consumed approval coordination join'
    final = public_file(run / 'authority/provisional.json')
    assert final['state'] == 'provisional_complete' and final['ownerLive'] is False \
        and final['liveAuthority'] is False and type(final['consumptions']) is int and final['consumptions'] == 1, 'terminal owner receipt'
    for key in ('runId', 'checkpoint', 'bindings', 'membership', 'preparations'):
        assert canonical(final[key]) == canonical(frozen[key]), 'final signed owner scope: ' + key
    assert final['approvalSha256'] == approval_check['approvalSha256'] \
        and final['approvalExpires'] == approval_check['expiresUnixSeconds'], 'final owner approval binding'
    assert final['resultSha256'] == digest(run / 'results.json'), 'final published result digest'
    checked = audit(run, _owner_public_key=public_key)
    checked['ownerExecution'] = dict(intent=intent_check, approval=approval_check, sourceBytesVerified=True,
        executionKind='offlineRehearsal', userPresenceVerified=False, nonceCustodyVerified=False,
        liveAuthority=False, groundedAssistanceAccuracyCredit=0)
    return checked


def audit(run, *, _owner_public_key=None):
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
    rehearsal = model_rehearsal_audit(run, load(run / 'authority/provisional.json'), requested,
        _owner_public_key=_owner_public_key)
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
    plan = load(run / 'authority/provisional.json')['bindings'].get('modelRehearsalPlan', {})
    hosted = rehearsal is not None and 'offlineHosted' in plan
    offline_http = rehearsal is not None and plan.get('offlineHttp') is True
    offline_response = rehearsal is not None and 'offlineResponse' in plan
    if _owner_public_key is not None:
        original, source = owner_offline_retained_auditor_source(rehearsal, plan)
    else:
        original, source = retained_auditor_source(hosted, offline_http, offline_response)
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
    if offline_response:
        result['auditBindings']['offlineResponseNativeAdaptations'] = 3
    if _owner_public_key is not None:
        result['auditBindings']['ownerOfflineAdaptations'] = dict(native=2, durable=1)
    if config.get('storage'):
        result['storage'] = storage_audit(run, config)
    return result


if __name__ == '__main__':
    run = pathlib.Path(sys.argv[1]).resolve()
    print(json.dumps(incomplete_audit(run) if sys.argv[2:] == ['--incomplete'] else audit(run), indent=2))
