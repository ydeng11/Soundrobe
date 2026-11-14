const { test } = require('node:test');

test('public offline response bytes are sibling-bound and never an activation input', () => {
  const fs = require('node:fs'), path = require('node:path'), crypto = require('node:crypto');
  const { offlineResponseFixture } = require('./run-bounded.cjs');
  const root = fs.mkdtempSync(path.join(process.cwd(), '.amp/in/public-response-'));
  const file = path.join(root, 'response-fixture.json'), planFile = path.join(root, 'public-plan.json');
  const bytes = Buffer.from('{"public":"asymmetric response","usage":17}\n');
  const sha = value => crypto.createHash('sha256').update(value).digest('hex');
  const plan = { offlineHosted: {}, offlineHttp: true, offlineResponse: { sha256: sha(bytes), status: 429 } };
  try {
    fs.writeFileSync(file, bytes);
    const result = offlineResponseFixture(plan, planFile);
    assert.deepEqual(result, { path: path.relative(process.cwd(), file), sha256: sha(bytes) });
    assert.equal(offlineResponseFixture({}, planFile), null);
    for (const change of [p => delete p.offlineHosted, p => p.offlineHttp = false,
      p => p.offlineResponse.status = true, p => p.offlineResponse.status = 600,
      p => p.offlineResponse.status = 99, p => p.offlineResponse.sha256 = '0'.repeat(64),
      p => p.offlineResponse.path = '.env.local', p => p.offlineResponse = null]) {
      const changed = structuredClone(plan); change(changed);
      assert.throws(() => offlineResponseFixture(changed, planFile));
    }
    fs.unlinkSync(file);
    assert.throws(() => offlineResponseFixture(plan, planFile));
    fs.symlinkSync(planFile, file);
    assert.throws(() => offlineResponseFixture(plan, planFile));
    fs.unlinkSync(file);
    fs.writeFileSync(file, Buffer.alloc(65537));
    assert.throws(() => offlineResponseFixture({ ...plan, offlineResponse: { sha256: sha(fs.readFileSync(file)), status: 200 } }, planFile));
  } finally { fs.rmSync(root, { recursive: true, force: true }); }
});
const assert = require('node:assert/strict');
const { spawnSync } = require('node:child_process');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const alias = '~deepseek/deepseek-v4-flash-latest';
const target = 'deepseek/deepseek-v4-flash-0731';
const env = `LLM_MODEL=${alias}\nLLM_API_KEY=SECRET_NEVER_EXPORTED\n`;
const model = () => ({ data: { id: alias, alias_target: { slug: target },
  supported_parameters: ['max_tokens', 'temperature', 'response_format', 'reasoning'],
  reasoning: { mandatory: false } } });
const hosts = () => ({ data: { id: target, endpoints: [
  { tag: 'cheaper/unsupported', model_id: target, status: 0, supported_parameters: ['max_tokens'],
    pricing: { prompt: '0.00000001', completion: '0.00000001' } },
  { tag: 'fixture/fp8', model_id: target, status: 0,
    supported_parameters: ['max_tokens', 'temperature', 'response_format', 'reasoning', 'structured_outputs'],
    pricing: { prompt: '0.00000006', completion: '0.00000018' } },
] } });
const prepare = (...args) => require('./preflight-model.cjs').preflight(...args);

const disabledPolicy = 'disabled-hosted-boundary-v1;native-file;native-response-observation;no-live;no-retry;no-repair';
function ownerCoordinationControl() {
  const frozen = { runId: 'native-component', checkpoint: 'a'.repeat(64), state: 'frozen',
    testPolicy: disabledPolicy, liveAuthority: false, consumptions: 0, credentialLookups: 0,
    actualFakeLookups: 0, counters: { model: 0 }, operations: {},
    membership: { requestedFieldSlots: 216 }, preparations: { job: { digest: 'prepared' } },
    bindings: { modelRehearsalPlan: { nativePrepared: true }, modelRehearsal: {
      policy: disabledPolicy, liveAuthority: false, boundary: { allowanceUsdCents: 500,
        hardBillingCap: false, billingRiskAccepted: false, liveDispatchAvailable: false, readyForExecution: false } } } };
  const consumed = { consumed: true, testFixtureOnly: true, expiresUnixSeconds: Math.floor(Date.now() / 1000) + 1200 };
  const report = { ...structuredClone(frozen), state: 'executing', consumptions: 1,
    approvalSha256: 'b'.repeat(64), approvalExpires: consumed.expiresUnixSeconds };
  const ready = { state: 'ready', checkpoint: frozen.checkpoint, approvalSha256: report.approvalSha256, liveAuthority: false };
  const control = { frozen, consumed, report, ready, calls: [], saved: [], statuses: [
    { state: 'pending', checkpoint: frozen.checkpoint, liveAuthority: false }, ready], scope: { remainingMs: 1000 } };
  control.authority = { async command(command) {
    control.calls.push(command);
    if (command.op === control.fail) throw Error('RPC lost; never retry');
    if (command.op === 'scope') return control.scope;
    if (command.op === 'owner_approval_status') return control.statuses.shift() ?? ready;
    if (command.op === 'approve_owner_file') return control.consumed;
    if (command.op === 'report') return control.report;
    throw Error('unexpected command');
  } };
  control.run = () => require('./mock-authority.cjs').approveFrozen(control.authority, frozen,
    (filename, value) => control.saved.push({ filename, value }));
  return control;
}

test('owner coordinator waits only for pending, scopes every check/consume, and retains joined disabled evidence', async () => {
  const c = ownerCoordinationControl();
  const receipt = await c.run();
  assert.deepEqual(c.calls.map(v => v.op), ['scope', 'owner_approval_status', 'scope', 'owner_approval_status',
    'scope', 'approve_owner_file', 'report']);
  assert.ok(c.calls.every(v => Object.keys(v).join(',') === 'op'));
  assert.deepEqual(c.saved, [{ filename: 'owner-approval-coordination.json', value: receipt }]);
  for (const key of ['runId', 'checkpoint', 'bindings', 'membership', 'preparations', 'approvalSha256',
    'approvalExpires', 'consumptions', 'state', 'credentialLookups', 'actualFakeLookups']) {
    assert.deepEqual(receipt[key], c.report[key], key);
  }
  assert.equal(receipt.liveAuthority, false);
  assert.equal(receipt.componentCoordinationOnly, true);
  assert.equal(receipt.successCertificate, false);
});

test('owner coordinator rejects missing/exhausted/malformed scope before status or consume', async () => {
  for (const scope of [undefined, null, {}, { remainingMs: 0 }, { remainingMs: -1 },
    { remainingMs: '1000' }, { remainingMs: 1.5 }, { remainingMs: Number.MAX_SAFE_INTEGER + 1 }]) {
    const c = ownerCoordinationControl(); c.scope = scope;
    await assert.rejects(c.run(), /scope/);
    assert.deepEqual(c.calls, [{ op: 'scope' }]);
  }
  const c = ownerCoordinationControl(); c.statuses = [c.ready];
  const command = c.authority.command;
  c.authority.command = value => { if (c.calls.length === 2) c.scope = { remainingMs: 0 }; return command(value); };
  await assert.rejects(c.run(), /scope/);
  assert.equal(c.calls.some(v => v.op === 'approve_owner_file'), false);
});

test('owner coordinator fails closed on status drift, wrong types and unknown status without polling', async () => {
  for (const mutate of [s => s.state = 'unknown', s => s.state = false, s => s.checkpoint = 'c'.repeat(64),
    s => s.liveAuthority = true, s => s.liveAuthority = 0, s => s.approvalSha256 = true,
    s => s.approvalSha256 = 'bad', s => s.extra = true, s => delete s.checkpoint,
    s => { s.state = 'pending'; }, s => { s.state = 'pending'; delete s.approvalSha256; s.extra = 1; }]) {
    const c = ownerCoordinationControl(), status = structuredClone(c.ready); mutate(status); c.statuses = [status];
    await assert.rejects(c.run(), /status/);
    assert.deepEqual(c.calls.map(v => v.op), ['scope', 'owner_approval_status']);
    assert.deepEqual(c.saved, []);
  }
});

test('owner coordinator joins immutable consumed scope, hash, expiry, state and zero activity', async () => {
  for (const mutate of [r => r.runId = 'other', r => r.checkpoint = 'c'.repeat(64),
    r => r.bindings.extra = 1, r => r.membership.requestedFieldSlots = 1, r => r.preparations = {},
    r => r.approvalSha256 = 'c'.repeat(64), r => r.consumptions = 2, r => r.state = 'frozen',
    r => r.liveAuthority = true, r => r.credentialLookups = 1, r => r.actualFakeLookups = 1,
    r => r.counters.model = 1, r => r.operations.model = { category: 'model' },
    r => r.approvalExpires = 1, r => r.approvalExpires++, r => delete r.credentialLookups]) {
    const c = ownerCoordinationControl(); c.statuses = [c.ready]; mutate(c.report);
    await assert.rejects(c.run(), /coordination/);
    assert.equal(c.calls.filter(v => v.op === 'approve_owner_file').length, 1);
    assert.deepEqual(c.saved, []);
  }
  for (const consumed of [null, {}, { consumed: false }, { consumed: true, expiresUnixSeconds: 1 }]) {
    const c = ownerCoordinationControl(); c.statuses = [c.ready]; c.consumed = consumed;
    await assert.rejects(c.run(), /coordination/);
    assert.equal(c.calls.filter(v => v.op === 'approve_owner_file').length, 1);
  }
});

test('owner coordinator never retries RPC loss including ambiguous consume or report', async () => {
  for (const fail of ['scope', 'owner_approval_status', 'approve_owner_file', 'report']) {
    const c = ownerCoordinationControl(); c.statuses = [c.ready]; c.fail = fail;
    await assert.rejects(c.run(), /RPC lost/);
    assert.equal(c.calls.filter(v => v.op === fail).length, 1);
    assert.deepEqual(c.saved, []);
  }
});

test('pending publication stops at the native scope deadline without consuming', async () => {
  const c = ownerCoordinationControl(), command = c.authority.command;
  c.authority.command = value => {
    const result = command(value);
    if (value.op === 'owner_approval_status') c.scope = { remainingMs: 0 };
    return result;
  };
  await assert.rejects(c.run(), /scope/);
  assert.deepEqual(c.calls.map(v => v.op), ['scope', 'owner_approval_status', 'scope']);
  assert.deepEqual(c.saved, []);
});

test('coordination result persists through the supplied saver; storage loss never retries consumption', async () => {
  const root = fs.mkdtempSync(path.resolve('.amp/in/owner-coordination-'));
  try {
    const c = ownerCoordinationControl(); c.statuses = [c.ready];
    const receipt = await require('./mock-authority.cjs').approveFrozen(c.authority, c.frozen,
      (filename, value) => fs.writeFileSync(path.join(root, filename), JSON.stringify(value), { flag: 'wx' }));
    assert.deepEqual(JSON.parse(fs.readFileSync(path.join(root, 'owner-approval-coordination.json'))), receipt);
    assert.deepEqual(fs.readdirSync(root), ['owner-approval-coordination.json']);
    const failed = ownerCoordinationControl(); failed.statuses = [failed.ready];
    await assert.rejects(require('./mock-authority.cjs').approveFrozen(failed.authority, failed.frozen,
      () => { throw Error('charged retention failed'); }), /charged retention failed/);
    assert.equal(failed.calls.filter(v => v.op === 'approve_owner_file').length, 1);
  } finally { fs.rmSync(root, { recursive: true, force: true }); }
});

test('native coordinator rejects fixture modes, loose native markers, enabled flags and changed allowance before RPC', async () => {
  for (const mutate of [r => r.state = 'executing', r => r.consumptions = 1,
    r => r.bindings.modelRehearsal.offlineFixture = false,
    ...['offlineHosted', 'offlineHttp', 'offlineResponse'].flatMap(key => [
      r => r.bindings.modelRehearsalPlan[key] = false, r => r.bindings.modelRehearsal[key] = false]),
    ...[false, 1, 'true', null].map(value => r => r.bindings.modelRehearsalPlan.nativePrepared = value),
    r => r.liveAuthority = true, r => r.bindings.modelRehearsal.liveAuthority = true,
    r => r.bindings.modelRehearsal.boundary.allowanceUsdCents = 501,
    ...['hardBillingCap', 'billingRiskAccepted', 'liveDispatchAvailable', 'readyForExecution'].map(key =>
      r => r.bindings.modelRehearsal.boundary[key] = true)]) {
    const c = ownerCoordinationControl(); mutate(c.frozen);
    await assert.rejects(c.run());
    assert.deepEqual(c.calls, []);
  }
});

test('fixture coordinator retains the existing public fixture approval and approves exactly once', async () => {
  const report = { runId: 'fixture', checkpoint: 'frozen', bindings: {}, membership: {}, preparations: {} };
  const saved = [], calls = [];
  await require('./mock-authority.cjs').approveFrozen({ async command(c) { calls.push(c); } }, report,
    (filename, value) => saved.push({ filename, value }));
  assert.equal(saved.length, 1);
  assert.equal(saved[0].filename, 'test-fixture-approval.json');
  assert.deepEqual(calls, [{ op: 'approve', approval: saved[0].value }]);
});

test('native launcher rejects every fixture combination before reading fixture files', () => {
  const { offlineResponseFixture } = require('./run-bounded.cjs');
  assert.equal(offlineResponseFixture({ nativePrepared: true }), null);
  for (const value of [false, 1, 'true', null]) {
    assert.throws(() => offlineResponseFixture({ nativePrepared: value }), /native/);
  }
  for (const key of ['offlineHosted', 'offlineHttp', 'offlineResponse']) {
    assert.throws(() => offlineResponseFixture({ nativePrepared: true, [key]: false }), /native/);
  }
});

test('file-only preflight preserves the exact moving alias without emitting credentials or authority', () => {
  const result = prepare(env, model(), hosts());
  assert.equal(result.model, alias);
  assert.equal(result.observedTarget, target);
  assert.equal(result.endpoint, 'https://openrouter.ai/api/v1');
  assert.equal(result.candidateRoute, 'fixture/fp8');
  assert.equal(result.liveAuthority, false);
  assert.equal(result.readyForExecution, false);
  assert.equal(result.allowanceUsdCents, 500);
  assert.equal(result.textEstimate.usd, '0.00067584');
  assert.equal(result.textEstimate.hardBillingCap, false);
  assert.equal(result.limits.modelAttempts, 1);
  assert.equal(result.limits.retries, 0);
  assert.equal(result.limits.repairs, 0);
  assert.equal(result.limits.preparationMs, 300000);
  assert.equal(result.limits.executionMs, 600000);
  assert.ok(result.blockers.includes('live_dispatch_disabled'));
  assert.ok(result.blockers.includes('real_run_approval_missing'));
  assert.ok(!JSON.stringify(result).includes('SECRET_NEVER_EXPORTED'));
});

test('secret rotation does not enter public settings fingerprints or inherited process environment', () => {
  const previous = process.env.LLM_MODEL;
  process.env.LLM_MODEL = 'different/inherited-setting';
  try {
    const a = prepare(env, model(), hosts());
    const b = prepare(env.replace('SECRET_NEVER_EXPORTED', 'OTHER_SECRET'), model(), hosts());
    assert.deepEqual(a, b);
    assert.equal(process.env.LLM_MODEL, 'different/inherited-setting');
  } finally {
    if (previous === undefined) delete process.env.LLM_MODEL;
    else process.env.LLM_MODEL = previous;
  }
});

test('preflight fails closed without echoing invalid private configuration', () => {
  for (const text of [env.replace('LLM_API_KEY', 'OTHER_KEY'), env.replace(alias, 'SECRET_BAD_MODEL'),
    env + 'LLM_PROVIDER=anthropic\n', env + 'LLM_BASE_URL=https://user:SECRET@openrouter.ai/api/v1\n',
    env + 'LLM_BASE_URL=https://other.invalid/v1\n', env.replace(alias, '${LLM_MODEL}')]) {
    assert.throws(() => prepare(text, model(), hosts()), /^Error: preflight_[a-z_]+$/);
  }
});

test('public catalog matching never silently substitutes another alias or target', () => {
  for (const mutate of [m => m.data.id = 'different/model', m => m.data.alias_target.slug = 'other/target',
    m => m.data.reasoning.mandatory = true, m => m.data.supported_parameters = []]) {
    const changed = model(); mutate(changed);
    assert.throws(() => prepare(env, changed, hosts()));
  }
  const changed = hosts(); changed.data.endpoints[1].model_id = 'other/model';
  assert.throws(() => prepare(env, model(), changed));
});

test('unsupported, inactive, unknown-price and over-allowance routes cannot become candidates', () => {
  for (const mutate of [e => e.status = 1, e => e.supported_parameters = [],
    e => delete e.pricing.completion, e => e.pricing.prompt = 'NaN',
    e => e.pricing.prompt = '-1', e => e.pricing.prompt = '1',
    e => e.tag = 'route?SECRET', e => e.tag = 'fixture//fp8',
    e => e.pricing.input_cache_read = 'NaN', e => e.pricing.input_cache_read = '1',
    e => e.pricing.request = '1']) {
    const changed = hosts(); mutate(changed.data.endpoints[1]);
    assert.throws(() => prepare(env, model(), changed));
  }
});

test('a report binds public catalog changes and exact non-secret settings but is not approval', () => {
  const a = prepare(env, model(), hosts());
  const changed = hosts(); changed.data.endpoints[1].pricing.prompt = '0.00000007';
  const b = prepare(env, model(), changed);
  assert.notEqual(a.catalogSha256, b.catalogSha256);
  assert.notEqual(a.planSha256, b.planSha256);
  assert.equal(a.settingsSha256, b.settingsSha256);
  assert.equal(b.readyForExecution, false);
});

test('CLI reads only supplied files, emits sanitized JSON and rejects execution switches', () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'sr-preflight-'));
  try {
    const files = ['settings.env', 'model.json', 'hosts.json'].map(n => path.join(root, n));
    [env, JSON.stringify(model()), JSON.stringify(hosts())].forEach((v, i) => fs.writeFileSync(files[i], v));
    const run = args => spawnSync(process.execPath, [path.join(__dirname, 'preflight-model.cjs'), ...args],
      { encoding: 'utf8', env: { PATH: process.env.PATH } });
    const good = run(files);
    assert.equal(good.status, 0, good.stderr);
    assert.equal(JSON.parse(good.stdout).model, alias);
    assert.ok(!good.stdout.includes('SECRET_NEVER_EXPORTED'));
    const bad = run([...files, '--execute']);
    assert.equal(bad.status, 1);
    assert.equal(bad.stdout, '');
    assert.match(bad.stderr, /^preflight_arguments_invalid\n$/);
    fs.writeFileSync(files[1], 'SECRET_INVALID_JSON');
    const invalid = run(files);
    assert.equal(invalid.status, 1);
    assert.equal(invalid.stdout, '');
    assert.equal(invalid.stderr, 'preflight_input_unavailable\n');
  } finally { fs.rmSync(root, { recursive: true }); }
});

test('supervised launcher passes only the public plan path before native bootstrap, never inherited credentials', () => {
  const root = fs.mkdtempSync(path.resolve('.amp/in/rehearsal-launcher-'));
  try {
    const executable = path.join(root,'receiver.cjs'), captured = path.join(root,'environment.json');
    const publicPlan = path.join(root,'not-read-before-native.json');
    fs.writeFileSync(executable,`#!${process.execPath}\nrequire('node:fs').writeFileSync(${JSON.stringify(captured)},JSON.stringify(process.env)); process.exit(78);\n`,{mode:0o700});
    const result = spawnSync(process.execPath,[require.resolve('./run-supervised.cjs'),path.join(root,'output'),executable,publicPlan],
      {encoding:'utf8',env:{PATH:process.env.PATH,LLM_API_KEY:'DO_NOT_INHERIT_SECRET_CANARY'}});
    assert.equal(result.status,1);
    const environment = JSON.parse(fs.readFileSync(captured));
    assert.equal(environment.SOUNDROBE_MODEL_REHEARSAL_PLAN,publicPlan);
    assert.equal(environment.LLM_API_KEY,undefined);
    assert.equal(fs.existsSync(publicPlan),false);
    assert.equal(JSON.parse(result.stdout).state,'incomplete');
  } finally { fs.rmSync(root,{recursive:true}); }
});

test('prepared-policy approval uses separate public fixture trust and rejects unknown policy', () => {
  const crypto = require('node:crypto');
  const policy = 'offline-model-rehearsal-v1;separate-verifier;fake-credential;loopback-only;no-live;no-retry;no-repair';
  const report = { runId: 'fixture', checkpoint: 'frozen', bindings: {}, membership: {}, preparations: {}, testPolicy: policy };
  const canonical = v => !v || typeof v !== 'object' ? JSON.stringify(v) : Array.isArray(v)
    ? '[' + v.map(canonical).join(',') + ']' : '{' + Object.keys(v).sort().map(k => JSON.stringify(k)+':'+canonical(v[k])).join(',') + '}';
  const { testApproval } = require('./mock-authority.cjs');
  const approval = testApproval(report);
  const fixture = crypto.createPrivateKey({format:'der',type:'pkcs8',key:Buffer.from('302e020100300506032b657004220420'+'2a'.repeat(32),'hex')});
  assert.ok(crypto.verify(null, Buffer.from(canonical(approval.payload)), crypto.createPublicKey(fixture), Buffer.from(approval.signature)));
  assert.equal(approval.payload.scopeSha256, crypto.createHash('sha256').update(canonical({runId:report.runId,checkpoint:report.checkpoint,bindings:{},membership:{},preparations:{},policy})).digest('hex'));
  const hosted = { ...report, bindings: { modelRehearsal: { offlineFixture: true } },
    testPolicy: 'disabled-hosted-boundary-v1;native-file;native-response-observation;no-live;no-retry;no-repair' };
  const signed = testApproval(hosted);
  const hostedFixture = crypto.createPrivateKey({format:'der',type:'pkcs8',key:Buffer.from('302e020100300506032b657004220420'+'2b'.repeat(32),'hex')});
  assert.ok(crypto.verify(null, Buffer.from(canonical(signed.payload)), crypto.createPublicKey(hostedFixture), Buffer.from(signed.signature)));
  assert.equal(crypto.verify(null, Buffer.from(canonical(signed.payload)), crypto.createPublicKey(fixture), Buffer.from(signed.signature)), false);
  assert.throws(() => testApproval({...report,testPolicy:'unapproved-live-policy'}));
  for (const value of [undefined, false, 1, null]) {
    const changed = structuredClone(hosted);
    if (value === undefined) delete changed.bindings.modelRehearsal.offlineFixture;
    else changed.bindings.modelRehearsal.offlineFixture = value;
    assert.throws(() => testApproval(changed), /fixture/);
  }
  for (const value of [true, false, 1, null]) {
    const changed = structuredClone(hosted);
    changed.bindings.modelRehearsalPlan = { nativePrepared: value };
    assert.throws(() => testApproval(changed), /native/);
  }
});

test('hosted offline staging requires one classified native observation without inventing an arrival', () => {
  const { validateProvisionalReceipt } = require('./mock-authority.cjs');
  const model = { category:'model',dispatched:true,arrived:false,completed:true,
    response:{observation:'offline_injected_native_response',remoteArrivalVerified:false} };
  const report = {state:'provisional_complete',consumptions:1,denied:0,ownerLive:false,liveAuthority:false,
    runId:'run',checkpoint:'frozen',membership:{requestedFieldSlots:216},
    testPolicy:'disabled-hosted-boundary-v1;native-file;native-response-observation;no-live;no-retry;no-repair',
    bindings:{supervision:{generation:'generation'},modelRehearsal:{offlineFixture:true}},
    resultSha256:'a'.repeat(64),counters:{reserved:1,arrived:0,completed:1,model:1},preparations:{},
    sessions:Object.fromEntries(Array.from({length:12},(_,i)=>[i,{closed:true}])),operations:{model}};
  assert.doesNotThrow(()=>validateProvisionalReceipt(report,structuredClone(report)));
  const http = structuredClone(report);
  http.bindings.modelRehearsalPlan = { offlineHosted: {}, offlineHttp: true };
  http.bindings.modelRehearsal.offlineHttp = true;
  http.operations.model.response.observation = 'native_http_response';
  http.operations.model.response.http = { version: 1, transport: 'offline_local_http' };
  assert.doesNotThrow(() => validateProvisionalReceipt(http, structuredClone(http)));
  for (const mutate of [r => delete r.bindings.modelRehearsal.offlineHttp,
    r => delete r.bindings.modelRehearsalPlan.offlineHttp,
    r => delete r.bindings.modelRehearsalPlan.offlineHosted,
    ...[false, 1, 'true', null].flatMap(value => [
      r => r.bindings.modelRehearsal.offlineHttp = value,
      r => r.bindings.modelRehearsalPlan.offlineHttp = value]),
    r => r.operations.model.response.observation = 'offline_injected_native_response',
    r => r.operations.model.response.http.transport = 'hosted_https',
    r => r.operations.model.response.remoteArrivalVerified = true,
    ...[true, false, 1, null].map(value => r => r.bindings.modelRehearsalPlan.nativePrepared = value)]) {
    const changed = structuredClone(http); mutate(changed);
    assert.throws(() => validateProvisionalReceipt(changed, structuredClone(changed)));
  }
  for (const mutate of [r=>r.operations.model.response.observation='wire',
    r=>r.operations.model.response.remoteArrivalVerified=true,r=>r.operations.model.arrived=true,
    r=>r.counters.arrived=1,r=>r.operations.extra=structuredClone(model),
    r=>r.testPolicy='unknown',r=>r.bindings.modelRehearsal.offlineFixture=false]) {
    const changed=structuredClone(report); mutate(changed);
    assert.throws(()=>validateProvisionalReceipt(changed,structuredClone(changed)));
  }
});

test('loopback fixture preserves distinct prepared request alias and response target without retries', async () => {
  const { createService } = require('../mock-provider-service.cjs');
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'sr-rehearsal-fixture-'));
  const file = path.join(root, 'service.json');
  const payload = {artist:'Asymmetric Artist',album:'Different Album',tracks:['Track One','Other Two']};
  fs.writeFileSync(file,JSON.stringify({schemaVersion:1,records:[],scriptedModel:{maxRequests:1,payload,
    requestModel:alias,responseModel:target,provider:'Wafer',response:{genre:'Diagnostic',confidence:0.8},usage:{prompt_tokens:7,completion_tokens:3,total_tokens:10}}}));
  const server = createService(file);
  try {
    await new Promise(resolve => server.listen(0,'127.0.0.1',resolve));
    const url = `http://127.0.0.1:${server.address().port}/model/chat/completions`;
    const body = JSON.stringify({model:alias,messages:[{role:'user',content:JSON.stringify(payload)}],response_format:{json_schema:{name:'GenreFillResponse'}}});
    const result = await fetch(url,{method:'POST',body});
    assert.equal(result.status,200);
    const parsed = await result.json();
    assert.equal(parsed.model,target);
    assert.equal(parsed.provider,'Wafer');
    assert.equal((await fetch(url,{method:'POST',body})).status,501);
  } finally {
    server.closeAllConnections();
    await new Promise(resolve=>server.close(resolve));
    fs.rmSync(root,{recursive:true});
  }
});
