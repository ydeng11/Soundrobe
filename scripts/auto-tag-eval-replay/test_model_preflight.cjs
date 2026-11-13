const { test } = require('node:test');
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
  assert.throws(() => testApproval({...report,testPolicy:'unapproved-live-policy'}));
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
