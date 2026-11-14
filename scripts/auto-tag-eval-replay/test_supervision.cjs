const { test } = require('node:test');
const assert = require('node:assert/strict');
const { validateProvisionalReceipt } = require('./mock-authority.cjs');

test('HTTP response audit independently derives bounded claims without authority or diagnostic leakage', () => {
  const { spawnSync } = require('node:child_process');
  const result = spawnSync('python3', ['-B', '-c', `
import sys,json,hashlib,copy
sys.path.insert(0,'scripts/auto-tag-eval-replay')
from importlib import import_module
m=import_module('audit-supervised')
assert callable(getattr(m,'http_response_audit',None)), 'missing independent HTTP response auditor'
sha=lambda b:hashlib.sha256(b).hexdigest()
encode=lambda v:json.dumps(v,ensure_ascii=False,separators=(',',':')).encode()
request=b'{"model":"request-alias"}'
model='response-model';provider='claimed-provider'
answer=dict(genre='Jazz',confidence=0.8)
body=dict(model=model,provider=provider,choices=[dict(finish_reason='stop',message=dict(content=encode(answer).decode()))],
    usage=dict(prompt_tokens=7,completion_tokens=3,total_tokens=10),diagnostic='RAW_SECRET_CANARY')
projection=dict(model=model,provider=provider,genre='Jazz',confidence=0.8,
    usage=dict(promptTokens=7,completionTokens=3,totalTokens=10,reasoningTokens=0))
count=0
def check(label,change=lambda b:None,validation='accepted',project=None,status=200,raw=None,req=request,transport='offline_local_http',damage=None,error=None):
    global count
    value=copy.deepcopy(body);change(value)
    response=encode(value) if raw is None else raw
    expected=dict(version=1,transport=transport,requestSha256=sha(req),validation=validation,
        projection=None if validation=='rejected' else copy.deepcopy(projection if project is None else project))
    receipt=dict(sha256=sha(response),bytes=len(response),status=status,identityClaimsVerified=False,
        pricingVerified=False,usageClaimsVerified=False,observation='native_http_response',remoteArrivalVerified=False,http=copy.deepcopy(expected))
    if damage: damage(receipt)
    try:
        observed=m.http_response_audit(req,response,receipt,model,provider,transport)
    except AssertionError as exc:
        assert error is not None and error in str(exc), (label,'wrong rejection',str(exc),error)
    else:
        assert error is None, (label,'accepted corrupt receipt')
        assert observed==expected, (label,observed,expected)
        assert observed is not receipt['http']
        if observed['projection'] is not None:
            assert observed['projection'] is not receipt['http']['projection']
        assert 'RAW_SECRET_CANARY' not in json.dumps(observed)
        assert set(observed)=={'version','transport','requestSha256','validation','projection'}
    count+=1
def content(value):
    return lambda b:b['choices'][0]['message'].update(content=encode(value).decode())
cases=[
    ('accepted',{}),('hosted HTTPS claims still unverified',dict(transport='hosted_https')),
    ('confidence numeric equality',dict(change=content(dict(genre='Jazz',confidence=1.0)),project=dict(projection,confidence=1))),
    ('threshold accepted',dict(change=content(dict(genre='Jazz',confidence=0.6)),project=dict(projection,confidence=0.6))),
    ('low confidence withheld',dict(change=content(dict(genre='Jazz',confidence=0.59)),validation='withheld',project=dict(projection,genre='',confidence=0.59))),
    ('empty genre withheld',dict(change=content(dict(genre='',confidence=0.8)),validation='withheld',project=dict(projection,genre=''))),
    ('optional empty fields',dict(change=lambda b:b['choices'][0]['message'].update(refusal=None,function_call=None,tool_calls=[],reasoning='',reasoning_content=None))),
    ('usage limit',dict(change=lambda b:b.update(usage=dict(prompt_tokens=8192,completion_tokens=1024,total_tokens=9216,completion_tokens_details=dict(reasoning_tokens=0))),project=dict(projection,usage=dict(promptTokens=8192,completionTokens=1024,totalTokens=9216,reasoningTokens=0)))),
    ('absent reasoning tokens',dict(change=lambda b:b['usage'].update(completion_tokens_details={}))),
    ('genre UTF8 boundary',dict(change=content(dict(genre='é'*128,confidence=0.8)),project=dict(projection,genre='é'*128))),
    ('genre UTF8 overflow',dict(change=content(dict(genre='é'*128+'x',confidence=0.8)),validation='rejected')),
    ('request byte boundary',dict(req=b'x'*32768)),
    ('request byte overflow',dict(req=b'x'*32769,error='request bytes')),
    ('invalid UTF8',dict(raw=b'\\xff',validation='rejected')),
    ('malformed JSON',dict(raw=b'{',validation='rejected')),
    ('non-object JSON',dict(raw=b'[]',validation='rejected')),
    ('nonfinite confidence',dict(change=content(dict(genre='Jazz',confidence=float('nan'))),validation='rejected')),
    ('invalid JSON diagnostic constant',dict(change=lambda b:b.update(diagnostic=float('nan')),validation='rejected')),
    ('duplicate genre field',dict(change=lambda b:b['choices'][0]['message'].update(content='{"genre":"Jazz","genre":"Jazz","confidence":0.8}'),validation='rejected')),
    ('duplicate confidence field',dict(change=lambda b:b['choices'][0]['message'].update(content='{"genre":"Jazz","confidence":0.8,"confidence":0.8}'),validation='rejected')),
    ('overflowing diagnostic float',dict(raw=encode(body).replace(b'"diagnostic":"RAW_SECRET_CANARY"',b'"diagnostic":1e400'),validation='rejected')),
    ('overflowing diagnostic integer',dict(raw=encode(body).replace(b'"diagnostic":"RAW_SECRET_CANARY"',b'"diagnostic":'+b'9'*400),validation='rejected')),
    ('negative-zero usage is not unsigned integer evidence',dict(raw=encode(dict(body,usage=dict(prompt_tokens=0,completion_tokens=3,total_tokens=3))).replace(b'"prompt_tokens":0',b'"prompt_tokens":-0'),validation='rejected')),
    ('infinite confidence',dict(change=content(dict(genre='Jazz',confidence=float('inf'))),validation='rejected')),
]
for status in (100,201,400,599): cases.append(('status '+str(status),dict(status=status,validation='rejected')))
for key,value in [('genre',True),('genre','bad\\n'),('genre','bad\\x7f'),('genre','bad\\x85'),('confidence',True),('confidence',-0.1),('confidence',1.1),('confidence','0.8')]:
    cases.append(('answer '+key+repr(value),dict(change=content(dict(answer,**{key:value})),validation='rejected')))
for value in ({'genre':'Jazz'},dict(answer,extra=1),['Jazz'],None):
    cases.append(('answer shape '+repr(value),dict(change=content(value),validation='rejected')))
for key,value in [('refusal','no'),('refusal',False),('function_call',{}),('tool_calls',[{}]),('tool_calls',False),('reasoning','secret'),('reasoning_content','secret'),('reasoning',False),('content',{}),('content','not JSON')]:
    cases.append(('message '+key+repr(value),dict(change=lambda b,k=key,v=value:b['choices'][0]['message'].update({k:v}),validation='rejected')))
for key,value in [('model','wrong'),('provider','wrong'),('model',True),('provider',None),('choices',[]),('choices',[{},{}]),('choices',{}),('usage',None)]:
    cases.append(('outer '+key+repr(value),dict(change=lambda b,k=key,v=value:b.update({k:v}),validation='rejected')))
cases.extend([('unfinished',dict(change=lambda b:b['choices'][0].update(finish_reason='length'),validation='rejected')),
    ('missing message',dict(change=lambda b:b['choices'][0].pop('message'),validation='rejected')),
    ('missing finish reason',dict(change=lambda b:b['choices'][0].pop('finish_reason'),validation='rejected')),
    ('missing content',dict(change=lambda b:b['choices'][0]['message'].pop('content'),validation='rejected'))])
for key in ('model','provider','choices','usage'):
    cases.append(('missing response '+key,dict(change=lambda b,k=key:b.pop(k),validation='rejected')))
for key,value in [('prompt_tokens',True),('prompt_tokens',7.0),('prompt_tokens',8193),('prompt_tokens',-1),('completion_tokens',1025),('total_tokens',11),('total_tokens',True),('completion_tokens_details',None),('completion_tokens_details',[]),('completion_tokens_details',dict(reasoning_tokens=1)),('completion_tokens_details',dict(reasoning_tokens=False))]:
    cases.append(('usage '+key+repr(value),dict(change=lambda b,k=key,v=value:b['usage'].update({k:v}),validation='rejected')))
for key in ('prompt_tokens','completion_tokens','total_tokens'):
    cases.append(('missing usage '+key,dict(change=lambda b,k=key:b['usage'].pop(k),validation='rejected')))
# Limits count bytes, including JSON whitespace, rather than Unicode code points.
inner=encode(dict(answer,genre='é')).decode()
for size in (4096,4097):
    cases.append(('content byte '+str(size),dict(change=lambda b,n=size:b['choices'][0]['message'].update(content=inner+' '*(n-len(inner.encode()))),project=dict(projection,genre='é'),validation='accepted' if size==4096 else 'rejected')))
utf8=encode(dict(body,diagnostic='é'))
for size in (65536,65537):
    cases.append(('response byte '+str(size),dict(raw=utf8+b' '*(size-len(utf8)),**({} if size==65536 else {'error':'response bytes'}))))
for label,options in cases: check(label,**options)
damage=[('response hash',lambda r:r.update(sha256='0'*64),'response binding'),
    ('response length',lambda r:r.update(bytes=r['bytes']+1),'response binding'),
    ('boolean bytes',lambda r:r.update(bytes=True),'response binding'),
    ('request hash',lambda r:r['http'].update(requestSha256='0'*64),'http receipt'),
    ('changed projection',lambda r:r['http']['projection'].update(genre='Invented'),'http receipt'),
    ('boolean confidence',lambda r:r['http']['projection'].update(confidence=True),'http receipt'),
    ('boolean version',lambda r:r['http'].update(version=True),'http receipt'),
    ('float version',lambda r:r['http'].update(version=1.0),'http receipt'),
    ('boolean usage',lambda r:r['http']['projection']['usage'].update(reasoningTokens=False),'http receipt'),
    ('float usage',lambda r:r['http']['projection']['usage'].update(promptTokens=7.0),'http receipt'),
    ('HTTP extra diagnostic',lambda r:r['http'].update(diagnostic='RAW_SECRET_CANARY'),'http receipt'),
    ('receipt extra header',lambda r:r.update(headers={'authorization':'secret'}),'receipt fields'),
    ('receipt not object',lambda r:r.clear(),'receipt fields'),
    ('old observation',lambda r:r.update(observation='offline_injected_native_response'),'observation'),
    ('transport drift',lambda r:r['http'].update(transport='hosted_https'),'http receipt'),
    ('projection extra authority',lambda r:r['http']['projection'].update(liveAuthority=True),'http receipt'),
    ('null projection accepted',lambda r:r['http'].update(projection=None),'http receipt'),
    ('forged rejected',lambda r:r['http'].update(validation='rejected',projection=None),'http receipt')]
for key in ('identityClaimsVerified','pricingVerified','usageClaimsVerified','remoteArrivalVerified'):
    for value in (True,0,None): damage.append((key+repr(value),lambda r,k=key,v=value:r.update({k:v}),'verification flags'))
for value in (True,200.0,99,600,'200'):
    damage.append(('invalid status '+repr(value),lambda r,v=value:r.update(status=v),'status'))
for key in ('sha256','bytes','status','identityClaimsVerified','pricingVerified','usageClaimsVerified','observation','remoteArrivalVerified','http'):
    damage.append(('missing receipt '+key,lambda r,k=key:r.pop(k),'receipt fields'))
for key in ('version','transport','requestSha256','validation','projection'):
    damage.append(('missing http '+key,lambda r,k=key:r['http'].pop(k),'http receipt'))
for key in ('model','provider','genre','confidence','usage'):
    damage.append(('missing projection '+key,lambda r,k=key:r['http']['projection'].pop(k),'http receipt'))
for key in ('promptTokens','completionTokens','totalTokens','reasoningTokens'):
    damage.append(('missing projected usage '+key,lambda r,k=key:r['http']['projection']['usage'].pop(k),'http receipt'))
for label,mutate,error in damage: check(label,damage=mutate,error=error)
check('unknown transport',transport='fixture',error='transport')
check('boolean confidence cannot equal one',change=content(dict(answer,confidence=1)),project=dict(projection,confidence=1),damage=lambda r:r['http']['projection'].update(confidence=True),error='http receipt')
check('rejected cannot carry projection',status=400,validation='rejected',damage=lambda r:r['http'].update(projection=projection),error='http receipt')
check('withheld cannot expose genre',change=content(dict(genre='Jazz',confidence=0.5)),validation='withheld',project=dict(projection,genre='',confidence=0.5),damage=lambda r:r['http']['projection'].update(genre='Jazz'),error='http receipt')
check('corrupt exact request bytes',damage=lambda r:r['http'].update(requestSha256=sha(request+b' ')),error='http receipt')
print(str(count)+' independent HTTP audit cases passed; no authority or quality promotion')
`], { env: { PATH: process.env.PATH }, encoding: 'utf8' });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /independent HTTP audit cases passed; no authority or quality promotion/);
  console.log(result.stdout.trim());
});

const receipt = () => ({ state: 'provisional_complete', consumptions: 1, denied: 0,
  ownerLive: false, liveAuthority: false, runId: 'fresh-run', checkpoint: 'frozen',
  membership: { requestedFieldSlots: 216 }, bindings: { supervision: { generation: 'fresh-generation' } },
  counters: { reserved: 1, arrived: 1, completed: 1 }, preparations: {},
  resultSha256: 'a'.repeat(64),
  sessions: Object.fromEntries(Array.from({ length: 12 }, (_, i) => [String(i), { closed: true }])),
  operations: { one: { dispatched: true, arrived: true, completed: true } } });

test('explicitly released staging is valid evidence, never complete authority', () => {
  const value = receipt();
  assert.doesNotThrow(() => validateProvisionalReceipt(value, structuredClone(value)));
  for (const mutate of [r => r.state = 'complete', r => r.ownerLive = true,
    r => r.membership.requestedFieldSlots--, r => r.resultSha256 = 'b'.repeat(64),
    r => r.operations.one.completed = false, r => r.bindings.supervision.generation = 'other']) {
    const changed = structuredClone(value); mutate(changed);
    assert.throws(() => validateProvisionalReceipt(changed, value));
  }
});

for (const mode of ['fixture', 'offline hosted', 'offline HTTP']) test(`prepared ${mode} model rehearsal audit joins exact alias request, response receipts and retained provenance`, () => {
  const hosted = mode !== 'fixture', offlineHttp = mode === 'offline HTTP';
  const fs = require('node:fs'), path = require('node:path'), crypto = require('node:crypto');
  const { spawnSync } = require('node:child_process');
  const root = fs.mkdtempSync(path.resolve('.amp/in/model-audit-'));
  const sha = b => crypto.createHash('sha256').update(b).digest('hex');
  const canonical = value => JSON.stringify(value, (_, v) => v && typeof v === 'object' && !Array.isArray(v)
    ? Object.fromEntries(Object.keys(v).sort().map(k => [k, v[k]])) : v);
  const plan = { model: '~deepseek/deepseek-v4-flash-latest', observedTarget: 'deepseek/deepseek-v4-flash-0731',
    route: 'wafer/us', provider: 'Wafer', input: { artist: '盧廣仲', album: 'Album', tracks: ['Intro', 'Title'] } };
  if (hosted) plan.offlineHosted = { promptPicoUsd: 1000, completionPicoUsd: 2000,
    catalogSha256: 'a'.repeat(64), termsSha256: 'b'.repeat(64) };
  if (offlineHttp) plan.offlineHttp = true;
  const policy = hosted ? 'disabled-hosted-boundary-v1;native-file;native-response-observation;no-live;no-retry;no-repair'
    : 'offline-model-rehearsal-v1;separate-verifier;fake-credential;loopback-only;no-live;no-retry;no-repair';
  const body = { model: plan.model, temperature: 0.2, max_tokens: 1024, reasoning: { enabled: false }, stream: false,
    provider: { allow_fallbacks: false, require_parameters: true, only: [plan.route], order: [plan.route] },
    messages: [{ role: 'system', content: 'Infer a conservative music genre from the supplied artist, album, and track titles. Return a concise Discogs-style comma-separated genre and confidence. Use low confidence when the evidence is insufficient.' },
      { role: 'user', content: canonical(plan.input) }],
    response_format: { type: 'json_schema', json_schema: { name: 'GenreFillResponse', schema: {
      type: 'object', properties: { genre: { type: 'string' }, confidence: { type: 'number' } }, required: ['genre', 'confidence'] } } } };
  const request = Buffer.from(canonical(body)), requestSha256 = sha(request);
  const responseValue = { genre: 'MOCK_ACCEPTED_GENRE_CANARY', confidence: 0.8 };
  const response = (hosted ? canonical : JSON.stringify)({ model: plan.observedTarget, provider: plan.provider,
    choices: [{ finish_reason: 'stop', message: { content: (hosted ? canonical : JSON.stringify)(responseValue) } }],
    usage: { prompt_tokens: 7, completion_tokens: 3, total_tokens: 10 }, diagnostic: 'MOCK_RAW_DIAGNOSTIC_CANARY' });
  const names = ['disabled-billie', 'disabled-enya', 'disabled-crowd', 'assistance-billie', 'assistance-enya', 'assistance-crowd'];
  const files = {};
  const put = (file, value) => files[file] = value;
  // Public credential fixture sits beside the plan, outside the output tree.
  const output = path.join(root, 'output');
  const planFile = path.join(root, 'public-plan.json');
  fs.writeFileSync(planFile, JSON.stringify(plan));
  const credentialFile = path.join(root, 'credential-fixture/.env.local');
  const credential = `LLM_API_KEY=MOCK_ONLY_SECRET_CANARY\nLLM_MODEL=${plan.model}\n`;
  if (hosted) {
    fs.mkdirSync(path.dirname(credentialFile));
    fs.writeFileSync(credentialFile, credential, { mode: 0o600 });
  }
  put('selection.json', { model: plan.model, modelRehearsalPlan: structuredClone(plan) });
  const binding = { policy, model: plan.model, observedTarget: plan.observedTarget, providerClaim: plan.provider,
    route: plan.route, requestSha256, verifierSha256: sha(Buffer.from(hosted
      ? '4508a07aa941707f3eb2db94c8897a80b2c1197476b6de213ac273df7d86c4ff'
      : '197f6b23e16c8532c6abc838facd5ea789be0c76b2920334039bfa8b3d368d61', 'hex')), liveAuthority: false };
  if (hosted) binding.offlineFixture = true;
  if (offlineHttp) binding.offlineHttp = true;
  if (hosted) binding.boundary = { endpoint: 'https://openrouter.ai/api/v1/chat/completions',
    settingsSha256: sha(JSON.stringify({ model: plan.model, endpoint: 'https://openrouter.ai/api/v1', provider: 'openrouter', source: 'env_local_only' })),
    pricingEvidence: structuredClone(plan.offlineHosted), textEstimatePicoUsd: 10240000, allowanceUsdCents: 500,
    inputTokenIllustration: 8192, outputTokens: 1024, hardBillingCap: false,
    billingRiskAccepted: false, liveDispatchAvailable: false, readyForExecution: false,
    retention: { remote: 'documented_provider_defaults_for_public_metadata', local: 'until_result_review' } };
  const receipt = { testPolicy: policy, bindings: { modelRehearsalPlan: plan, modelRehearsal: binding,
    sourcesAndTools: [{ path: path.relative(process.cwd(), planFile), sha256: sha(JSON.stringify(plan)) },
      ...(hosted ? [{ path: path.relative(process.cwd(), credentialFile), sha256: sha(credential) }] : [])] },
    preparations: {}, sessions: {}, operations: {}, counters: { model: 1 }, credentialLookups: 1 };
  if (offlineHttp) {
    receipt.bindings.transport = { endpoint: 'native-owned-loopback-http/api/v1/chat/completions' };
    put('offline-http-observation.json', { version: 1, transport: 'offline_local_http', requests: 1,
      requestSha256, requestBytes: request.length, responseSha256: sha(response),
      responseBytes: Buffer.byteLength(response), status: 200, exactRequest: true,
      fakeCredentialPresented: true, liveAuthority: false });
  }
  const requested = { evaluations: names.map(name => ({ name })) };
  for (const name of names) {
    const active = name === 'assistance-crowd';
    const preparation = { plan: { events: active ? [{ kind: 'request', body }] : [] } };
    put(`${name}/preparation/preparation.json`, preparation);
    receipt.preparations[name] = { preparationSha256: sha(JSON.stringify(preparation)), requestSha256: active ? requestSha256 : null };
    put(`${name}/service.json`, active && !hosted ? { scriptedModel: { requestModel: plan.model, responseModel: plan.observedTarget,
      provider: plan.provider, maxRequests: 1, bodySha256: requestSha256, payload: plan.input,
      response: responseValue, usage: { prompt_tokens: 7, completion_tokens: 3, total_tokens: 10 }, privacyCanaries: true } } : {});
    for (const [stage, folder] of [['preparation', 'preparation'], ['execution', 'cold']]) {
      const id = `${name}-${stage}`;
      receipt.sessions[id] = { job: name, stage };
      put(`${name}/${folder}/session.json`, { job: name, stage, model_rehearsal: structuredClone(plan) });
      put(`${name}/${folder}/inventory-final.json`, { modelRequests: active && !hosted && stage === 'execution'
        ? [{ status: 200, sha256: requestSha256, bytes: request.length, responseSha256: sha(response), responseBytes: Buffer.byteLength(response), mockCredentialPresented: true }] : [],
      wireArrivals: active && !hosted && stage === 'execution'
        ? [{ operation: 'single-model', method: 'POST', route: '/model/chat/completions', status: 200, state: 'responded' }] : [] });
    }
    put(`${name}/scoring-contract.json`, { spec: { mode: 'mock' } });
    put(`${name}/score.provisional.json`, { groundedAssistanceAccuracyCredit: 0, writableDelta: [], acceptedCertificate: false });
    if (hosted) put(`${name}/result.provisional.json`, { modelRequests: active ? 1 : 0, wireArrivals: 0 });
    const assisted = active ? { model: plan.model, input_sha256: sha(canonical(plan.input)), value: responseValue.genre } : null;
    put(`${name}/cold/records.json`, [{ native: { candidate: { assisted_genre: assisted, genre: active ? responseValue.genre : null } },
      persistedEvidence: { assistedGenre: assisted }, fieldObservations: { tracks: [{ genre: active ? responseValue.genre : null }] } }]);
    if (active) files[`${name}/independent-request.json`] = request;
  }
  receipt.operations['single-model'] = { session: 'assistance-crowd-execution', category: 'model',
    dispatched: true, arrived: !hosted, completed: true,
    request_sha256: sha(JSON.stringify(['POST', '/model/chat/completions', requestSha256])),
    response: { status: 200, bytes: Buffer.byteLength(response), sha256: sha(response),
      identityClaimsVerified: false, pricingVerified: false, usageClaimsVerified: false,
      ...(hosted ? { observation: 'offline_injected_native_response', remoteArrivalVerified: false } : {}) } };
  const http = { version: 1, transport: 'offline_local_http', requestSha256, validation: 'accepted',
    projection: { model: plan.observedTarget, provider: plan.provider, genre: responseValue.genre,
      confidence: 0.8, usage: { promptTokens: 7, completionTokens: 3, totalTokens: 10, reasoningTokens: 0 } } };
  if (offlineHttp) Object.assign(receipt.operations['single-model'].response,
    { observation: 'native_http_response', http });
  if (hosted) for (const name of names) put(`${name}/native-model-observations.json`, name === 'assistance-crowd'
    ? { 'single-model': structuredClone(receipt.operations['single-model']) } : {});
  const check = (mutate = () => {}) => {
    const current = structuredClone({ files, receipt, requested, credential, credentialMode: 0o600 }); mutate(current);
    fs.writeFileSync(planFile, JSON.stringify(current.publicPlan || plan));
    if (hosted) { fs.writeFileSync(credentialFile, current.credential); fs.chmodSync(credentialFile, current.credentialMode); }
    fs.rmSync(output, { recursive: true, force: true });
    for (const [file, value] of Object.entries(current.files)) {
      const target = path.join(output, file); fs.mkdirSync(path.dirname(target), { recursive: true });
      fs.writeFileSync(target, value instanceof Uint8Array ? value : JSON.stringify(value));
    }
    fs.writeFileSync(path.join(root, 'context.json'), JSON.stringify(current));
    return spawnSync('python3', ['-B', '-c', `import pathlib,sys,json,importlib
sys.path.insert(0,'scripts/auto-tag-eval-replay')
m=importlib.import_module('audit-supervised');r=pathlib.Path(sys.argv[1]);c=json.loads((r/'context.json').read_text())
print(json.dumps(m.model_rehearsal_audit(r/'output',c['receipt'],c['requested'])))`, root], { env: { PATH: process.env.PATH }, encoding: 'utf8' });
  };
  try {
    const valid = check(); assert.equal(valid.status, 0, valid.stderr);
    assert.equal(JSON.parse(valid.stdout).liveAuthority, false);
    const damage = [
      c => delete c.receipt.bindings.modelRehearsalPlan,
      c => c.receipt.bindings.modelRehearsalPlan.verifier = 'injected',
      c => c.receipt.bindings.modelRehearsal.verifierSha256 = 'a'.repeat(64),
      c => c.receipt.testPolicy = 'historical',
      c => c.receipt.bindings.sourcesAndTools = [],
      c => c.receipt.bindings.sourcesAndTools[0].sha256 = 'c'.repeat(64),
      c => c.publicPlan = { ...plan, route: 'other/route' },
      c => c.files['selection.json'].model = plan.observedTarget,
      c => delete c.files['disabled-enya/cold/session.json'].model_rehearsal,
      c => {
        const prepared = c.files['assistance-crowd/preparation/preparation.json'];
        prepared.plan.events[0].body.provider.only = ['other'];
        c.receipt.preparations['assistance-crowd'].preparationSha256 = sha(JSON.stringify(prepared));
      },
      c => c.receipt.preparations['disabled-billie'].requestSha256 = requestSha256,
      ...(hosted ? [
        c => c.receipt.operations['single-model'].arrived = true,
        c => c.receipt.operations['single-model'].response.observation = 'fixture_wire',
        c => c.receipt.operations['single-model'].response.remoteArrivalVerified = true,
        c => c.receipt.bindings.modelRehearsal.offlineFixture = false,
        c => delete c.receipt.bindings.modelRehearsal.offlineFixture,
        c => {
          delete c.receipt.bindings.modelRehearsal.offlineFixture;
          c.receipt.bindings.offlineFixture = true;
        },
        c => c.receipt.bindings.sourcesAndTools.pop(),
        c => c.receipt.bindings.sourcesAndTools.push(c.receipt.bindings.sourcesAndTools[1]),
        c => c.credential += '# drift\n',
        c => c.credentialMode = 0o644,
        c => c.receipt.bindings.modelRehearsal.boundary.pricingEvidence.promptPicoUsd++,
        c => c.receipt.bindings.modelRehearsal.boundary.textEstimatePicoUsd++,
        c => c.receipt.bindings.modelRehearsal.boundary.liveDispatchAvailable = true,
        c => c.receipt.bindings.modelRehearsal.policy = 'offline-model-rehearsal-v1;separate-verifier;fake-credential;loopback-only;no-live;no-retry;no-repair',
        c => c.receipt.bindings.modelRehearsal.verifierSha256 = sha(Buffer.from('197f6b23e16c8532c6abc838facd5ea789be0c76b2920334039bfa8b3d368d61', 'hex')),
        c => c.receipt.bindings.modelRehearsal.verifierSha256 = sha(Buffer.from('d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a', 'hex')),
        c => c.files['assistance-crowd/native-model-observations.json'] = {},
        c => c.files['disabled-enya/native-model-observations.json'] = c.files['assistance-crowd/native-model-observations.json'],
        c => c.files['assistance-crowd/native-model-observations.json']['single-model'].completed = false,
        c => c.files['assistance-crowd/native-model-observations.json']['single-model'].completed = 1,
        c => c.files['assistance-crowd/service.json'].scriptedModel = {},
        c => c.files['assistance-crowd/cold/inventory-final.json'].wireArrivals.push({ operation: 'single-model', method: 'POST' }),
        c => c.files['assistance-crowd/result.provisional.json'].modelRequests = 0,
        c => c.files['assistance-crowd/result.provisional.json'].wireArrivals = 1,
        c => c.receipt.bindings.sourcesAndTools[1].sha256 = 'f'.repeat(64),
        ...[null, {}, { ...plan.offlineHosted, extra: 1 }, { ...plan.offlineHosted, promptPicoUsd: true },
          { ...plan.offlineHosted, promptPicoUsd: -1 }, { ...plan.offlineHosted, completionPicoUsd: 0.5 },
          { ...plan.offlineHosted, promptPicoUsd: 2 ** 64 }, { ...plan.offlineHosted, completionPicoUsd: 5e12 },
          { ...plan.offlineHosted, catalogSha256: 'not-a-digest' }].map(pricing => c => {
          c.publicPlan = c.receipt.bindings.modelRehearsalPlan;
          c.publicPlan.offlineHosted = pricing;
          c.receipt.bindings.sourcesAndTools[0].sha256 = sha(JSON.stringify(c.publicPlan));
          c.receipt.bindings.modelRehearsal.boundary.pricingEvidence = pricing;
          if (pricing) c.receipt.bindings.modelRehearsal.boundary.textEstimatePicoUsd = pricing.promptPicoUsd * 8192 + pricing.completionPicoUsd * 1024;
        }),
      ] : [
        c => c.files['assistance-crowd/service.json'].scriptedModel.responseModel = plan.model,
        c => c.files['assistance-crowd/cold/inventory-final.json'].modelRequests[0].responseSha256 = 'b'.repeat(64),
      ]),
      c => c.receipt.operations['single-model'].response.identityClaimsVerified = true,
      ...(!offlineHttp ? [c => Object.assign(c.receipt.operations['single-model'].response,
        { observation: 'native_http_response', remoteArrivalVerified: false, http: {
          version: 1, transport: 'offline_local_http', requestSha256, validation: 'accepted',
          projection: { model: plan.observedTarget, provider: plan.provider, genre: responseValue.genre,
            confidence: 0.8, usage: { promptTokens: 7, completionTokens: 3, totalTokens: 10, reasoningTokens: 0 } } } })] : []),
      ...(offlineHttp ? [
        c => delete c.receipt.bindings.modelRehearsal.offlineHttp,
        c => delete c.receipt.bindings.modelRehearsalPlan.offlineHttp,
        c => { delete c.receipt.bindings.modelRehearsalPlan.offlineHttp; delete c.receipt.bindings.modelRehearsal.offlineHttp; },
        c => delete c.receipt.bindings.modelRehearsalPlan.offlineHosted,
        c => delete c.files['assistance-crowd/cold/session.json'].model_rehearsal.offlineHttp,
        c => c.receipt.bindings.transport.endpoint = 'http://127.0.0.1:1234/api/v1/chat/completions',
        c => c.receipt.operations['single-model'].response.observation = 'offline_injected_native_response',
        c => delete c.receipt.operations['single-model'].response.http,
        c => c.receipt.operations['single-model'].response.sha256 = 'e'.repeat(64),
        c => c.receipt.operations['single-model'].response.bytes++,
        c => c.receipt.operations['single-model'].response.http.transport = 'hosted_https',
        c => c.receipt.operations['single-model'].response.http.projection.genre = 'Invented',
        c => c.receipt.operations['single-model'].response.http.projection.usage.promptTokens++,
        c => c.receipt.operations['single-model'].response.http.requestSha256 = 'd'.repeat(64),
        c => c.receipt.operations['single-model'].response.http.validation = 'withheld',
        c => delete c.files['offline-http-observation.json'],
        c => {
          c.publicPlan = { ...plan, offlineHttp: 1 };
          c.receipt.bindings.sourcesAndTools[0].sha256 = sha(JSON.stringify(c.publicPlan));
        },
        c => c.files['selection.json'].modelRehearsalPlan.offlineHttp = 1,
        ...names.flatMap(name => ['preparation', 'cold'].map(folder => c => c.files[`${name}/${folder}/session.json`].model_rehearsal.offlineHttp = 1)),
        c => {
          c.publicPlan = c.receipt.bindings.modelRehearsalPlan;
          delete c.publicPlan.offlineHttp; delete c.receipt.bindings.modelRehearsal.offlineHttp;
          delete c.files['selection.json'].modelRehearsalPlan.offlineHttp;
          for (const [file, value] of Object.entries(c.files)) if (file.endsWith('/session.json')) delete value.model_rehearsal.offlineHttp;
          c.receipt.bindings.sourcesAndTools[0].sha256 = sha(JSON.stringify(c.publicPlan));
        },
        ...[false, 1, 'true', null].flatMap(value => [
          c => c.receipt.bindings.modelRehearsal.offlineHttp = value,
          c => {
            c.publicPlan = c.receipt.bindings.modelRehearsalPlan;
            c.publicPlan.offlineHttp = value;
            c.receipt.bindings.sourcesAndTools[0].sha256 = sha(JSON.stringify(c.publicPlan));
          },
        ]),
        ...Object.entries({ version: true, transport: 'hosted_https', requests: 2,
          requestSha256: 'a'.repeat(64), requestBytes: request.length + 1,
          responseSha256: 'b'.repeat(64), responseBytes: Buffer.byteLength(response) + 1,
          status: 201, exactRequest: false, fakeCredentialPresented: false, liveAuthority: true,
          authorization: 'MOCK_ONLY_SECRET_CANARY' }).map(([key, value]) => c => c.files['offline-http-observation.json'][key] = value),
        ...['requests', 'requestBytes', 'responseBytes', 'status'].flatMap(key => [
          c => c.files['offline-http-observation.json'][key] = true,
          c => delete c.files['offline-http-observation.json'][key],
        ]),
      ] : [
        c => c.receipt.bindings.modelRehearsal.offlineHttp = true,
        c => c.receipt.bindings.modelRehearsalPlan.offlineHttp = true,
      ]),
      c => c.receipt.operations.extra = structuredClone(c.receipt.operations['single-model']),
      c => c.files['disabled-billie/cold/inventory-final.json'].modelRequests.push({}),
      c => c.files['assistance-crowd/cold/records.json'][0].native.candidate.assisted_genre.model = plan.observedTarget,
      c => c.files['assistance-crowd/cold/records.json'][0].fieldObservations.tracks[0].genre = 'wrong',
      c => c.files['assistance-crowd/scoring-contract.json'].spec.mode = 'live',
      c => c.files['assistance-crowd/score.provisional.json'].groundedAssistanceAccuracyCredit = 1,
    ];
    for (const [i, mutate] of damage.entries()) assert.notEqual(check(mutate).status, 0, `accepted semantic damage ${i}`);
    console.log(`${mode}: one accepted join and ${damage.length} rejected semantic corruptions`);
    const historical = check(c => {
      c.receipt.bindings = {}; delete c.receipt.testPolicy;
      c.files['selection.json'] = { model: 'scripted/genre-v1' };
      for (const [file, value] of Object.entries(c.files)) if (file.endsWith('/session.json')) delete value.model_rehearsal;
    });
    assert.equal(historical.status, 0, historical.stderr);
    assert.equal(JSON.parse(historical.stdout), null);
  } finally { fs.rmSync(root, { recursive: true }); }
});

for (const offlineHttp of [false, true]) test(`${offlineHttp ? 'HTTP' : 'hosted'} retained auditor adaptations reconcile 497 settled operations with only 496 fixture arrivals`, () => {
  const { spawnSync } = require('node:child_process');
  const result = spawnSync('python3', ['-B', '-c', `
import pathlib,sys,json,importlib,tempfile,hashlib,copy
sys.path.insert(0,'scripts/auto-tag-eval-replay')
m=importlib.import_module('audit-supervised')
offline_http=${offlineHttp ? 'True' : 'False'}
assert hashlib.sha256(m.retained_auditor_source()[1].encode()).hexdigest()=='540699a2153af67f86f845eebecaed879164908fdc35644284be6043aee3a0f1'
assert hashlib.sha256(m.retained_auditor_source(True)[1].encode()).hexdigest()=='319be49ed4c0156723624b1934aac9256b4185f3c0af820ba6e875cb1c31f67a'
original,source=m.retained_auditor_source(True,offline_http=True) if offline_http else m.retained_auditor_source(True)
compile(source,'adapted-durable','exec')
# Obtain the exact nested native adaptation without executing either auditor.
ns={}
sys.argv=['audit',str(pathlib.Path('.amp/in/artifacts').resolve())]
exec(source[:source.index('native = json.loads')],ns)
native=ns['adapted'];compile(native,'adapted-native','exec')
wire_checks=source[source.index('all_wires = []'):source.index('print(json.dumps')]
request_checks=native[native.index("    requests = [e['body']"):native.index("    if name.endswith('billie')")]
summary=native[native.index('    audit.append(dict'):native.index('for rows in pairs.values():')]
digest=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
load=lambda p:json.loads(p.read_text())
with tempfile.TemporaryDirectory(dir='.amp/in/artifacts',prefix='hosted-wire-audit-') as temp:
    run=pathlib.Path(temp).resolve()
    put=lambda p,v:(p.parent.mkdir(parents=True,exist_ok=True),p.write_text(json.dumps(v)))
    names=['disabled-billie','disabled-enya','disabled-crowd','assistance-billie','assistance-enya','assistance-crowd']
    requested={'evaluations':[{'name':n} for n in names]}
    receipt={'sessions':{},'operations':{},'preparations':{},'counters':dict(provider=472,inventory=24,model=1,reserved=497,arrived=496,completed=497)}
    request_bytes=json.dumps({'model':'alias'},sort_keys=True,separators=(',',':')).encode()
    request_sha=hashlib.sha256(request_bytes).hexdigest()
    model={'category':'model','session':'assistance-crowd-execution','dispatched':True,'arrived':False,'completed':True,
        'request_sha256':hashlib.sha256(json.dumps(['POST','/model/chat/completions',request_sha],separators=(',',':')).encode()).hexdigest(),
        'response':{'observation':'native_http_response' if offline_http else 'offline_injected_native_response','remoteArrivalVerified':False}}
    receipt['operations']['model']=model
    for index,name in enumerate(names):
        directory=run/name
        prep={'plan':{'events':[{'kind':'request','body':{'model':'alias'}}] if name=='assistance-crowd' else []}}
        put(directory/'preparation/preparation.json',prep)
        receipt['preparations'][name]={'preparationSha256':digest(directory/'preparation/preparation.json')}
        (directory/'independent-request.json').write_bytes(request_bytes)
        put(directory/'native-model-observations.json',{'model':model} if name=='assistance-crowd' else {})
        for stage,folder in [('preparation','preparation'),('execution','cold')]:
            sid=name+'-'+stage
            put(directory/folder/'cold.jsonl',{})
            receipt['sessions'][sid]=dict(job=name,stage=stage,attached=True,sealed=True,finished=True,closed=True,
                native_live=False,fixture_live=False,observation_sha256=digest(directory/folder/'cold.jsonl'))
            count=40 if index<2 else 39
            wires=[]
            for i in range(count+2):
                oid=sid+'-'+str(i);category='provider' if i<count else 'inventory'
                receipt['operations'][oid]=dict(session=sid,category=category,dispatched=True,arrived=True,completed=True)
                wires.append(dict(operation=oid,method='GET',route='/provider' if i<count else '/__fixtures/compact'))
            inv=dict(compact=True,records=[{}]*1533,misses=[],requests=[{}]*count,modelRequests=[],wireArrivals=wires)
            put(directory/folder/'inventory-final.json',inv);put(directory/(folder+'-fixture-ingress.json'),inv)
    put(run/'wire-reconciliation.json',dict(counts=receipt['counters'],nativeModelObservations=1,actualWireArrivals=496,uniqueOperations=497))
    def check():
        scope=dict(run=run,receipt=receipt,requested=requested,load=load,digest=digest,json=json)
        exec(wire_checks,scope)
        assert scope['model']==1 and len(scope['all_wires'])==496
        audit=[]
        for name in names:
            directory=run/name;prep=load(directory/'preparation/preparation.json');inventory=load(directory/'cold/inventory-final.json')
            row=dict(reviewPendingAfterRestart=True,actionableReviewAfterRestart=True,persistedEvidence={'assistedGenre':None},native={'candidate':{'assisted_genre':None}},outcome='test',pairedMock={'matchedEvents':[]},elapsedMs=0)
            prep['plan']['variant']='test'
            context=dict(directory=directory,name=name,prep=prep,inventory=inventory,row=row,load=load,json=json,hashlib=hashlib,audit=audit,inv_file=directory/'cold/inventory-final.json')
            exec('if True:\\n'+request_checks,context);exec('if True:\\n'+summary,context)
        assert sum(a['modelRequests'] for a in audit)==1
        assert sum(a['wireArrivals'] for a in audit)==248
    check()
    def rejected(change,restore):
        change()
        try: check()
        except (AssertionError,KeyError): pass
        else: raise AssertionError('accepted corrupted hosted wire evidence')
        finally: restore()
    rejected(lambda:model.update(arrived=True),lambda:model.update(arrived=False))
    rejected(lambda:receipt['counters'].update(arrived=497),lambda:receipt['counters'].update(arrived=496))
    rejected(lambda:receipt['operations'].update(extra=copy.deepcopy(model)),lambda:receipt['operations'].pop('extra'))
    observation=model['response']['observation']
    rejected(lambda:model['response'].update(observation='offline_injected_native_response' if offline_http else 'native_http_response'),lambda:model['response'].update(observation=observation))
    projection=run/'assistance-crowd/native-model-observations.json'
    rejected(lambda:put(projection,{}),lambda:put(projection,{'model':model}))
    wirefile=run/'wire-reconciliation.json';wire=load(wirefile)
    rejected(lambda:put(wirefile,dict(wire,actualWireArrivals=497)),lambda:put(wirefile,wire))
    invfile=run/'assistance-crowd/cold/inventory-final.json';snapshot=run/'assistance-crowd/cold-fixture-ingress.json';inv=load(invfile)
    invented=copy.deepcopy(inv);invented['modelRequests']=[{}];invented['wireArrivals'].append(dict(operation='model',method='POST',route='/model/chat/completions'))
    rejected(lambda:(put(invfile,invented),put(snapshot,invented)),lambda:(put(invfile,inv),put(snapshot,inv)))
print(('HTTP' if offline_http else 'hosted')+' counted native/durable adaptations and 7 corruption controls passed')
`], { env: { PATH: process.env.PATH }, encoding: 'utf8' });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /corruption controls passed/);
  console.log(result.stdout.trim());
});

test('recovery records observed receiver success, never staged files or eventual supervisor exit', () => {
  const { validateOutcome } = require('./run-supervised.cjs');
  const config = { generation: 'fresh', receiverPid: 123, receiverGroup: 321 };
  const status = { pid: 123, status: 0, signal: null };
  const value = { state: 'complete', authority: 'native_witness_and_timely_supervisor_exit',
    generation: 'fresh', receiverPid: 123, receiverGroup: 321, supervisorGroup: 987,
    executionGroup: 789, supervisorExitObservedAt: 190, acceptedAt: 191, deadline: 200,
    requestedFieldSlots: 216, liveAuthority: false, controlOnly: false,
    evidenceSha256: 'a'.repeat(64), resultSha256: 'b'.repeat(64) };
  assert.doesNotThrow(() => validateOutcome(status, config, value));
  for (const mutate of [r => r.state = 'provisional_complete', r => r.generation = 'old',
    r => r.acceptedAt = 200, r => r.supervisorExitObservedAt = 201,
    r => r.supervisorExitObservedAt = null, r => r.executionGroup = 321,
    r => r.resultSha256 = null, r => r.controlOnly = true, r => delete r.controlOnly]) {
    const changed = structuredClone(value); mutate(changed);
    assert.throws(() => validateOutcome(status, config, changed));
  }
  for (const change of [{ status: 78 }, { signal: 'SIGKILL' }, { pid: 124 }, { error: Error('unknown') }]) {
    assert.throws(() => validateOutcome({ ...status, ...change }, config, value));
  }
});

test('launcher returns incomplete without waiting for inherited descendant pipes', async () => {
  const fs = require('node:fs'), path = require('node:path');
  const { spawn } = require('node:child_process');
  const { once } = require('node:events');
  const root = fs.mkdtempSync(path.resolve('.amp/in/launcher-pipe-'));
  const executable = path.join(root, 'fixture.cjs');
  // Finite disposable descendant: no cleanup signal or stored PID is needed.
  const exiting = path.join(root, 'exiting.json');
  fs.writeFileSync(executable, `#!${process.execPath}\nrequire('node:child_process').spawn(process.execPath,
    ['-e', 'setTimeout(() => {}, 2500)'], {stdio: 'inherit'});
    require('node:fs').writeFileSync(${JSON.stringify(exiting)}, String(Date.now())); process.exit(78);\n`, { mode: 0o700 });
  const child = spawn(process.execPath, [require.resolve('./run-supervised.cjs'), path.join(root, 'output'), executable],
    { env: { PATH: process.env.PATH }, stdio: 'ignore' });
  const [status] = await once(child, 'exit');
  const elapsed = Date.now() - Number(fs.readFileSync(exiting, 'utf8'));
  try {
    assert.equal(status, 1);
    assert.ok(elapsed < 1500, `descendant pipe delayed incomplete return: ${elapsed}ms`);
  } finally {
    fs.rmSync(root, { recursive: true });
  }
});

test('failed log storage stops a silent driver without waiting for another chunk', async () => {
  const fs = require('node:fs'), path = require('node:path');
  const { spawn } = require('node:child_process');
  const { once } = require('node:events');
  const root = fs.mkdtempSync(path.resolve('.amp/in/log-failure-'));
  const executable = path.join(root, 'failed-sink.cjs');
  const entered = path.join(root, 'failure-entered');
  fs.writeFileSync(executable, `#!${process.execPath}\nrequire('node:fs').writeFileSync(${JSON.stringify(entered)}, 'entered'); process.exit(78);\n`, { mode: 0o700 });
  const child = spawn(process.execPath, ['-e', `
    const fs = require('node:fs');
    // Test the response to an actual failure, not helper startup latency.
    const watch = fs.watch(${JSON.stringify(root)}, () => {
      if (fs.existsSync(${JSON.stringify(entered)})) {
        watch.close();
        setTimeout(() => process.exit(0), 1500);
      }
    });
    require('./scripts/auto-tag-eval-replay/bounded-storage.cjs').logSink(process.env.SOUNDROBE_STORAGE_TARGET);
  `], { env: { SOUNDROBE_STORAGE_REQUIRED: '1', SOUNDROBE_NATIVE_OUTPUT: root,
    SOUNDROBE_NATIVE_GENERATION: 'test-generation', SOUNDROBE_STORAGE_EXECUTABLE: executable,
    SOUNDROBE_STORAGE_TARGET: path.join(root, 'native.log') }, stdio: 'ignore' });
  try { assert.equal((await once(child, 'exit'))[0], 78); }
  finally { fs.rmSync(root, { recursive: true }); }
});

test('incomplete audit retains all slots when storage recording is unavailable', () => {
  const fs = require('node:fs'), path = require('node:path');
  const { spawnSync } = require('node:child_process');
  const root = fs.mkdtempSync(path.resolve('.amp/in/storage-missing-'));
  try {
    fs.copyFileSync('test/fixtures/tauri/auto-tag-eval/supervised-requested-membership.json', path.join(root, 'requested-membership.json'));
    fs.writeFileSync(path.join(root, 'native-supervision.json'), '{}');
    const project = () => {
      const result = spawnSync('python3', ['-B', '-c',
        `import pathlib,sys,json;sys.path.insert(0,'scripts/auto-tag-eval-replay');from importlib import import_module;print(json.dumps(import_module('audit-supervised').incomplete_audit(pathlib.Path(sys.argv[1]))))`,
        root], { env: { PATH: process.env.PATH }, encoding: 'utf8' });
      assert.equal(result.status, 0, result.stderr);
      const audit = JSON.parse(result.stdout);
      assert.equal(audit.slots.length, 216);
      assert.equal(new Set(audit.slots.map(s => s.id)).size, 216);
      assert.equal(audit.unavailableFieldSlots, 216);
      assert.equal(audit.validatedFieldSlots, 0);
      assert.equal(audit.storage.state, 'unknown');
      assert.ok(audit.slots.every(s => s.effects === 'unknown' && s.observation === 'unavailable'));
      return audit;
    };
    assert.equal(project().membershipEvidence, 'matched');
    fs.unlinkSync(path.join(root, 'native-supervision.json'));
    fs.unlinkSync(path.join(root, 'requested-membership.json'));
    assert.equal(project().membershipEvidence, 'unavailable');
    fs.writeFileSync(path.join(root, 'native-supervision.json'), '{');
    fs.writeFileSync(path.join(root, 'requested-membership.json'), '{}');
    fs.writeFileSync(path.join(root, '.storage-ledger'), 'torn');
    assert.equal(project().membershipEvidence, 'mismatched');
  } finally { fs.rmSync(root, { recursive: true }); }
});

test('read-only recovery reconstructs source bindings and rejects missing or corrupt split state', () => {
  const fs = require('node:fs'), path = require('node:path');
  const { execFileSync } = require('node:child_process');
  const { recovered } = require('./check-mock-authority.cjs');
  const root = fs.mkdtempSync(path.resolve('.amp/in/binding-recovery-'));
  const cases = ['split', 'legacy', 'missing', 'malformed', 'malformed_hashed', 'changed', 'reference', 'inline', 'table', 'uncommitted'];
  try {
    execFileSync('python3', ['-B', '-c', `import sqlite3,json,hashlib,pathlib,sys
root=pathlib.Path(sys.argv[1])
for case in json.loads(sys.argv[2]):
    directory=root/case/'authority'
    directory.mkdir(parents=True)
    c=sqlite3.connect(directory/'registry.sqlite')
    c.executescript('CREATE TABLE authority (id INTEGER PRIMARY KEY,state TEXT); CREATE TABLE authority_bindings (id INTEGER PRIMARY KEY,value TEXT);')
    binding='{"executable":"independent-source-binding","supervision":{"generation":"fresh"}}'
    state=dict(bindings=None,bindings_sha256=hashlib.sha256(binding.encode()).hexdigest(),operations={},state='preparing')
    if case=='legacy': state['bindings']=json.loads(binding); del state['bindings_sha256']
    if case=='reference': del state['bindings_sha256']
    if case=='inline': state['bindings']={}
    if case=='malformed_hashed': state['bindings_sha256']=hashlib.sha256(b'{').hexdigest()
    if case!='uncommitted': c.execute('INSERT INTO authority VALUES (1,?)',(json.dumps(state),))
    if case not in ('missing','uncommitted'): c.execute('INSERT INTO authority_bindings VALUES (1,?)',('{' if case in ('malformed','malformed_hashed') else '{}' if case=='changed' else binding,))
    if case in ('table','legacy'): c.execute('DROP TABLE authority_bindings')
    c.commit()
    c.close()`, root, JSON.stringify(cases)], { env: { PATH: process.env.PATH } });
    for (const name of cases) {
      const directory = path.join(root, name), registry = path.join(directory, 'authority/registry.sqlite');
      const before = fs.readFileSync(registry);
      if (name === 'split' || name === 'legacy') {
        assert.deepEqual(recovered(directory).bindings,
          { executable: 'independent-source-binding', supervision: { generation: 'fresh' } });
      } else assert.throws(() => recovered(directory), `accepted ${name} source bindings`);
      assert.deepEqual(fs.readFileSync(registry), before, 'read-only export modified evidence');
    }
  } finally { fs.rmSync(root, { recursive: true }); }
});

test('storage audit validates issuance history, terminal reserve and both original caps', () => {
  const fs = require('node:fs'), path = require('node:path'), crypto = require('node:crypto');
  const { spawnSync } = require('node:child_process');
  const root = fs.mkdtempSync(path.resolve('.amp/in/storage-audit-'));
  const generation = '12345678-1234-1234-1234-123456789012';
  const record = (sequence, growth, writes, terminal, ticketGrowth, ticketWrites) => {
    const bytes = Buffer.alloc(256);
    Buffer.from(generation.replaceAll('-', ''), 'hex').copy(bytes, 1);
    crypto.createHash('sha256').update(root).digest().copy(bytes, 17);
    for (const [offset, value] of [[49,512*1048576], [57,1024*1048576], [65,growth],
      [73,writes], [81,sequence], [91,ticketGrowth], [99,ticketWrites]]) bytes.writeBigUInt64LE(BigInt(value), offset);
    bytes[89] = Number(terminal); bytes.writeUInt32LE(123,107); bytes[111] = 0;
    bytes.write('mock-storage-v1\0',112);
    crypto.createHash('sha256').update(bytes.subarray(1,224)).digest().copy(bytes,224);
    return bytes;
  };
  const check = (allowStopped = false) => spawnSync('python3', ['-B','-c',
    `import pathlib,sys;sys.path.insert(0,'scripts/auto-tag-eval-replay');from importlib import import_module;print(import_module('audit-supervised').storage_audit(pathlib.Path(sys.argv[1]), {'generation':sys.argv[2]}, allow_stopped=sys.argv[3]=='1'))`,
    root,generation,allowStopped ? '1' : '0'], {env:{PATH:process.env.PATH},encoding:'utf8'});
  try {
    const ledger = Buffer.alloc(2*1048576);
    const terminal = record(1,2*1048576,4*1048576,true,2*1048576,8*1048576);
    terminal.copy(ledger,0); terminal.copy(ledger,256);
    fs.writeFileSync(path.join(root,'.storage-owner-000'),'');
    const target = path.join(root,'.storage-ledger');
    fs.writeFileSync(target,ledger);
    assert.equal(check().status,0,check().stderr);
    const revocation = ledger.length - 65536;
    ledger[revocation] = 1; fs.writeFileSync(target,ledger);
    assert.notEqual(check().status,0, 'revocation admitted complete storage evidence');
    const revoked = check(true);
    assert.equal(revoked.status,0,revoked.stderr);
    assert.match(revoked.stdout, /'stopped': True/);
    ledger[revocation + 1] = 1; fs.writeFileSync(target,ledger);
    assert.notEqual(check(true).status,0, 'unrelated tail corruption accepted');
    ledger[revocation + 1] = 0;
    ledger[revocation] = 0;
    // Intact latest anchor cannot hide a missing/torn issuance record.
    ledger[256+99] ^= 1; fs.writeFileSync(target,ledger);
    assert.notEqual(check().status,0);
    // Re-checking only hashes would miss an internally valid overspend.
    const overspend = record(2,2*1048576,1016*1048576+1,true,0,1012*1048576+1);
    terminal.copy(ledger,256); overspend.copy(ledger,0); overspend.copy(ledger,512);
    fs.writeFileSync(target,ledger);
    assert.notEqual(check().status,0);
  } finally { fs.rmSync(root,{recursive:true}); }
});
