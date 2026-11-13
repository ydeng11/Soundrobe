const { test } = require('node:test');
const assert = require('node:assert/strict');
const { validateProvisionalReceipt } = require('./mock-authority.cjs');

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

for (const hosted of [false, true]) test(`prepared ${hosted ? 'offline hosted' : 'fixture'} model rehearsal audit joins exact alias request, response receipts and retained provenance`, () => {
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
  put('selection.json', { model: plan.model, modelRehearsalPlan: plan });
  const binding = { policy, model: plan.model, observedTarget: plan.observedTarget, providerClaim: plan.provider,
    route: plan.route, requestSha256, verifierSha256: sha(Buffer.from(hosted
      ? '4508a07aa941707f3eb2db94c8897a80b2c1197476b6de213ac273df7d86c4ff'
      : '197f6b23e16c8532c6abc838facd5ea789be0c76b2920334039bfa8b3d368d61', 'hex')), liveAuthority: false };
  if (hosted) binding.offlineFixture = true;
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
      put(`${name}/${folder}/session.json`, { job: name, stage, model_rehearsal: plan });
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
  if (hosted) for (const name of names) put(`${name}/native-model-observations.json`, name === 'assistance-crowd'
    ? { 'single-model': structuredClone(receipt.operations['single-model']) } : {});
  const check = (mutate = () => {}) => {
    const current = structuredClone({ files, receipt, requested, credential, credentialMode: 0o600 }); mutate(current);
    fs.writeFileSync(planFile, JSON.stringify(current.publicPlan || plan));
    if (hosted) { fs.writeFileSync(credentialFile, current.credential); fs.chmodSync(credentialFile, current.credentialMode); }
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
      c => c.receipt.operations.extra = structuredClone(c.receipt.operations['single-model']),
      c => c.files['disabled-billie/cold/inventory-final.json'].modelRequests.push({}),
      c => c.files['assistance-crowd/cold/records.json'][0].native.candidate.assisted_genre.model = plan.observedTarget,
      c => c.files['assistance-crowd/cold/records.json'][0].fieldObservations.tracks[0].genre = 'wrong',
      c => c.files['assistance-crowd/scoring-contract.json'].spec.mode = 'live',
      c => c.files['assistance-crowd/score.provisional.json'].groundedAssistanceAccuracyCredit = 1,
    ];
    for (const [i, mutate] of damage.entries()) assert.notEqual(check(mutate).status, 0, `accepted semantic damage ${i}`);
    const historical = check(c => {
      c.receipt.bindings = {}; delete c.receipt.testPolicy;
      c.files['selection.json'] = { model: 'scripted/genre-v1' };
      for (const [file, value] of Object.entries(c.files)) if (file.endsWith('/session.json')) delete value.model_rehearsal;
    });
    assert.equal(historical.status, 0, historical.stderr);
    assert.equal(JSON.parse(historical.stdout), null);
  } finally { fs.rmSync(root, { recursive: true }); }
});

test('hosted retained auditor adaptations reconcile 497 settled operations with only 496 fixture arrivals', () => {
  const { spawnSync } = require('node:child_process');
  const result = spawnSync('python3', ['-B', '-c', `
import pathlib,sys,json,importlib,tempfile,hashlib,copy
sys.path.insert(0,'scripts/auto-tag-eval-replay')
m=importlib.import_module('audit-supervised')
original,source=m.retained_auditor_source(True)
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
        'response':{'observation':'offline_injected_native_response','remoteArrivalVerified':False}}
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
    projection=run/'assistance-crowd/native-model-observations.json'
    rejected(lambda:put(projection,{}),lambda:put(projection,{'model':model}))
    wirefile=run/'wire-reconciliation.json';wire=load(wirefile)
    rejected(lambda:put(wirefile,dict(wire,actualWireArrivals=497)),lambda:put(wirefile,wire))
    invfile=run/'assistance-crowd/cold/inventory-final.json';snapshot=run/'assistance-crowd/cold-fixture-ingress.json';inv=load(invfile)
    invented=copy.deepcopy(inv);invented['modelRequests']=[{}];invented['wireArrivals'].append(dict(operation='model',method='POST',route='/model/chat/completions'))
    rejected(lambda:(put(invfile,invented),put(snapshot,invented)),lambda:(put(invfile,inv),put(snapshot,inv)))
print('hosted counted native/durable adaptations and corruption controls passed')
`], { env: { PATH: process.env.PATH }, encoding: 'utf8' });
  assert.equal(result.status, 0, result.stderr);
  assert.match(result.stdout, /corruption controls passed/);
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
