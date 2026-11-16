const { test, before, after } = require('node:test');
const fs = require('node:fs');
const path = require('node:path');

// Proposal hashing needs corpus metadata, not generated audio. Provision only
// absent metadata for these controls and remove only files this test created.
const createdCorpusFiles = [];
before(() => {
  for (const [population, fixture] of [['original', 'corpus.json'],
    ['artist-batch-extension', 'artist-batch-extension/corpus.json']]) {
    const file = path.resolve('.amp/in/eval-dummies', population, 'corpus.json');
    fs.mkdirSync(path.dirname(file), { recursive: true });
    try {
      fs.copyFileSync(path.resolve('test/fixtures/tauri/auto-tag-eval', fixture), file, fs.constants.COPYFILE_EXCL);
      createdCorpusFiles.push(file);
    } catch (error) { if (error.code !== 'EEXIST') throw error; }
  }
});
after(() => { for (const file of createdCorpusFiles) fs.unlinkSync(file); });

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
const os = require('node:os');

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

test('public provider manifests can be inspected without creating run output', () => {
  const { readPools, importPools } = require('../mock-provider-service.cjs');
  const parent = fs.mkdtempSync(path.resolve('.amp/in/proposal-pools-'));
  const output = path.join(parent, 'uncreated-run', 'providers.json');
  try {
    const pool = path.resolve('test/fixtures/tauri/auto-tag-eval/candidate-pools.json');
    const manifest = readPools(pool, output);
    assert.equal(fs.existsSync(path.dirname(output)), false);
    assert.equal(manifest.groundTruth, 'unreviewed_provider_evidence');
    const record = manifest.records.find(r => r.provider === 'discogs' && r.path === '/releases/36441795');
    assert.equal(path.resolve(path.dirname(output), record.bodyFile),
      path.resolve('test/fixtures/tauri/relapse-deluxe/release-36441795.json'));
    assert.equal(record.sha256, '791a57e61770548bc7964c53fb7aa601702bf90528ea3bb2f7362c22a6f17c8f');
    importPools(pool, output);
    assert.deepEqual(JSON.parse(fs.readFileSync(output)), manifest);
  } finally { fs.rmSync(parent, { recursive: true, force: true }); }
});

test('owner proposal inputs bind the six jobs and every static lock before creating a run', () => {
  const crypto = require('node:crypto');
  const { ownerProposalInputs } = require('./run-bounded.cjs');
  const packet = fs.mkdtempSync(path.resolve('.amp/in/artifacts/proposal-inputs-'));
  const output = path.join(packet, 'uncreated-run'), binary = path.join(packet, 'public-fake-binary');
  const planFile = path.join(packet, 'public-plan.json');
  try {
    fs.writeFileSync(binary, 'public executable fixture');
    const plan = { nativePrepared: true, model: alias, observedTarget: target, route: 'fixture/fp8',
      provider: 'PublicFixture', input: { artist: 'Public Artist', album: 'Public Album', tracks: ['One', 'Different'] } };
    fs.writeFileSync(planFile, JSON.stringify(plan));
    const result = ownerProposalInputs(output, binary, planFile,
      path.resolve('.planning/plans/crowd-lu-user-acceptance-v1-20261111'));
    assert.equal(fs.existsSync(output), false);
    assert.equal(result.membership.requestedFieldSlots, 216);
    assert.equal(result.membership.evaluations.length, 6);
    assert.deepEqual(result.bindings.modelRehearsalPlan, plan);
    assert.equal(result.bindings.disposableRunRoot, output);
    assert.equal(result.bindings.executable, crypto.createHash('sha256').update(fs.readFileSync(binary)).digest('hex'));
    const locks = result.bindings.sourcesAndTools;
    const runtime = locks.filter(lock => lock.runtime);
    assert.deepEqual(runtime, [{ path: path.relative(process.cwd(), path.join(output, 'native-supervision.json')),
      runtime: 'nativeSupervision' }]);
    assert.equal(new Set(locks.map(lock => lock.path)).size, locks.length);
    for (const lock of locks.filter(lock => !lock.runtime)) {
      assert.equal(lock.sha256, crypto.createHash('sha256').update(fs.readFileSync(lock.path)).digest('hex'));
    }
    assert(locks.some(lock => lock.path === 'src-tauri/src/infra/mock_run_owner.rs'));
    assert(locks.some(lock => lock.path === 'test/fixtures/tauri/relapse-deluxe/release-36441795.json'));
    assert(!JSON.stringify(result).includes('LLM_API_KEY'));
    assert.throws(() => ownerProposalInputs(`${packet}/../escaped`, binary, planFile,
      path.resolve('.planning/plans/crowd-lu-user-acceptance-v1-20261111')), /path/);
    fs.mkdirSync(output);
    assert.throws(() => ownerProposalInputs(output, binary, planFile,
      path.resolve('.planning/plans/crowd-lu-user-acceptance-v1-20261111')), /exist/);
  } finally { fs.rmSync(packet, { recursive: true, force: true }); }
});

test('owner response fixtures enter the same signed source list without changing native plans', () => {
  const { ownerProposalInputs } = require('./run-bounded.cjs');
  const crypto = require('node:crypto');
  const packet = fs.mkdtempSync(path.resolve('.amp/in/artifacts/owner-response-'));
  const output = path.join(packet, 'uncreated-run'), binary = path.join(packet, 'public-binary');
  const planFile = path.join(packet, 'public-native-plan.json'), response = path.join(packet, 'response-fixture.json');
  const labels = path.resolve('.planning/plans/crowd-lu-user-acceptance-v1-20261111');
  try {
    fs.writeFileSync(binary, 'public native binary');
    const plan = { nativePrepared: true, model: alias, observedTarget: target, route: 'fixture/fp8',
      provider: 'PublicFixture', input: { artist: 'Public Artist', album: 'Public Album', tracks: ['One', 'Other'] } };
    fs.writeFileSync(planFile, JSON.stringify(plan));
    const bytes = Buffer.from('{"public":"different genre and usage"}\n');
    fs.writeFileSync(response, bytes);
    const args = [output, binary, planFile, labels, undefined];
    const expected = { path: path.relative(process.cwd(), response), sha256: crypto.createHash('sha256').update(bytes).digest('hex') };
    const result = ownerProposalInputs(...args, response);
    assert.deepEqual(result.bindings.sourcesAndTools.filter(lock => lock.path === expected.path), [expected]);
    assert.deepEqual(result.bindings.modelRehearsalPlan, plan);
    assert(!ownerProposalInputs(...args).bindings.sourcesAndTools.some(lock => lock.path === expected.path));
    assert.equal(fs.existsSync(output), false);
    const other = path.join(packet, 'not-the-fixed-response.json');
    fs.renameSync(response, other);
    assert.throws(() => ownerProposalInputs(...args, other));
    fs.symlinkSync(other, response);
    assert.throws(() => ownerProposalInputs(...args, response));
    fs.unlinkSync(response);
    fs.linkSync(other, response);
    assert.throws(() => ownerProposalInputs(...args, response));
    fs.unlinkSync(response);
    fs.writeFileSync(response, Buffer.alloc(65537));
    assert.throws(() => ownerProposalInputs(...args, response));
  } finally { fs.rmSync(packet, { recursive: true, force: true }); }
});

test('owner wire disconnect is optional, source-locked and leaves plan and membership unchanged', () => {
  const { ownerProposalInputs, ownerResponseFixturesFromSources } = require('./run-bounded.cjs');
  const crypto = require('node:crypto');
  const packet = fs.mkdtempSync(path.resolve('.amp/in/artifacts/owner-wire-'));
  try {
    const binary = path.join(packet, 'binary'), plan = path.join(packet, 'public-native-plan.json');
    const response = path.join(packet, 'response-fixture.json'), wire = path.join(packet, 'response-wire.json');
    fs.writeFileSync(binary, 'public binary');
    fs.writeFileSync(plan, JSON.stringify({ nativePrepared: true, model: alias }));
    fs.writeFileSync(response, '{"public":"公開 response"}');
    const args = [path.join(packet, 'uncreated-run'), binary, plan,
      path.resolve('.planning/plans/crowd-lu-user-acceptance-v1-20261111'), undefined, response];
    const ordinary = ownerProposalInputs(...args);
    const bytes = Buffer.from(JSON.stringify({ version: 1, truncateAfterBytes: fs.statSync(response).size - 1 }));
    fs.writeFileSync(wire, bytes);
    const result = ownerProposalInputs(...args, wire);
    const lock = { path: path.relative(process.cwd(), wire), sha256: crypto.createHash('sha256').update(bytes).digest('hex') };
    const sources = result.bindings.sourcesAndTools;
    assert.deepEqual(sources.filter(s => s.path === lock.path), [lock]);
    assert.equal(sources.findIndex(s => s.path === lock.path), sources.findIndex(s => s.path === path.relative(process.cwd(), response)) + 1);
    assert.deepEqual(result.bindings.modelRehearsalPlan, ordinary.bindings.modelRehearsalPlan);
    assert.deepEqual(result.membership, ordinary.membership);
    assert.deepEqual(sources.filter(s => s.path !== lock.path), ordinary.bindings.sourcesAndTools);
    assert.equal(fs.existsSync(args[0]), false);
    assert.deepEqual(ownerResponseFixturesFromSources(plan, sources), {
      responseFixture: sources.find(s => s.path === path.relative(process.cwd(), response)), wireFixture: lock });
    assert.equal(ownerResponseFixturesFromSources(plan, ordinary.bindings.sourcesAndTools).wireFixture, null,
      'an unsigned sibling must not activate disconnect');
    fs.writeFileSync(wire, '{"version":1,"truncateAfterBytes":1}');
    assert.throws(() => ownerResponseFixturesFromSources(plan, sources), /wire/);
    fs.writeFileSync(wire, bytes);
    for (const changed of [[...sources, lock], sources.filter(s => s.path !== path.relative(process.cwd(), response)),
      sources.map(s => s.path === lock.path ? { ...s, extra: true } : s)]) {
      assert.throws(() => ownerResponseFixturesFromSources(plan, changed), /wire/);
    }
    const aliased = sources.map(s => s.path === lock.path ? { ...s, path: path.relative(process.cwd(), packet) + '/./response-wire.json' } : s);
    assert.equal(ownerResponseFixturesFromSources(plan, aliased).wireFixture, null);
  } finally { fs.rmSync(packet, { recursive: true, force: true }); }
});

test('owner wire disconnect rejects malformed schema, bounds, missing response and file aliases', () => {
  const { ownerProposalInputs, ownerWireFixture } = require('./run-bounded.cjs');
  const packet = fs.mkdtempSync(path.resolve('.amp/in/artifacts/owner-wire-invalid-'));
  const outside = fs.mkdtempSync(path.resolve('.amp/in/owner-wire-outside-'));
  try {
    const binary = path.join(packet, 'binary'), plan = path.join(packet, 'public-native-plan.json');
    const response = path.join(packet, 'response-fixture.json'), wire = path.join(packet, 'response-wire.json');
    fs.writeFileSync(binary, 'public binary');
    fs.writeFileSync(plan, JSON.stringify({ nativePrepared: true, model: alias }));
    fs.writeFileSync(response, '公開'); // Six bytes, not two characters.
    const good = '{"version":1,"truncateAfterBytes":5}';
    for (const bad of ['null', '[]', 'true', '{}', 'not JSON',
      '{"version":1,"truncateAfterBytes":1,"extra":false}',
      '{"version":1,"version":1,"truncateAfterBytes":1}',
      ...[true, false, null, '1', 0, -1, 1.5, 6, 7].map(n => JSON.stringify({ version: 1, truncateAfterBytes: n })),
      ...[true, '1', 2, null].map(version => JSON.stringify({ version, truncateAfterBytes: 1 })),
      good + ' '.repeat(1025)]) {
      fs.writeFileSync(wire, bad);
      assert.throws(() => ownerWireFixture(wire, plan, response), /wire/);
    }
    fs.writeFileSync(wire, good + ' '.repeat(1024 - Buffer.byteLength(good)));
    assert.equal(typeof ownerWireFixture(wire, plan, response).sha256, 'string');
    fs.writeFileSync(wire, good);
    const args = [path.join(packet, 'uncreated-run'), binary, plan,
      path.resolve('.planning/plans/crowd-lu-user-acceptance-v1-20261111'), undefined];
    assert.throws(() => ownerProposalInputs(...args, undefined, wire), /wire/);
    for (const file of [path.join(packet, 'alias.json'), path.join(outside, 'response-wire.json')]) {
      fs.writeFileSync(file, good);
      assert.throws(() => ownerWireFixture(file, plan, response), /wire/);
    }
    assert.throws(() => ownerWireFixture(packet + '/./response-wire.json', plan, response), /wire/);
    assert.throws(() => ownerWireFixture(wire, plan, response.replace('response-fixture', 'alias')), /wire|response/);
    const other = path.join(packet, 'other.json');
    fs.renameSync(wire, other);
    for (const link of [fs.symlinkSync, fs.linkSync]) {
      link(other, wire);
      assert.throws(() => ownerWireFixture(wire, plan, response), /wire/);
      fs.unlinkSync(wire);
    }
    fs.mkdirSync(wire);
    assert.throws(() => ownerWireFixture(wire, plan, response), /wire/);
    fs.rmdirSync(wire); fs.writeFileSync(wire, good);
    fs.unlinkSync(response);
    assert.throws(() => ownerWireFixture(wire, plan, response), /wire|response/);
  } finally { fs.rmSync(packet, { recursive: true, force: true }); fs.rmSync(outside, { recursive: true, force: true }); }
});

test('owner wire disconnect failure evidence never passes completed response or receipt admission', () => {
  const { validateOwnerOfflineReceipt } = require('./mock-authority.cjs');
  const { validateOwnerOfflineHttpObservation } = require('./run-bounded.cjs');
  const report = ownerOfflineReceiptFixture();
  report.state = 'complete';
  const operation = report.operations.model;
  operation.completed = false;
  delete operation.response;
  operation.failure = { observation: 'native_http_failure', transport: 'offline_local_http',
    code: 'response_body_incomplete', receivedBytes: 5, remoteArrivalVerified: false };
  assert.throws(() => validateOwnerOfflineReceipt(report, structuredClone(report), 'complete'));
  assert.throws(() => validateOwnerOfflineHttpObservation(report, operation.failure));
  const result = spawnSync('python3', ['-B', '-c', `
import hashlib, runpy
audit = runpy.run_path('scripts/auto-tag-eval-replay/audit-supervised.py')['http_response_audit']
prefix = b'{"pub'
receipt = dict(sha256=hashlib.sha256(prefix).hexdigest(), bytes=len(prefix), status=200,
    identityClaimsVerified=False, pricingVerified=False, usageClaimsVerified=False,
    remoteArrivalVerified=False, observation='native_http_failure', http={})
try:
    audit(b'{}', prefix, receipt, 'public/model', 'PublicFixture', 'offline_local_http')
except AssertionError as error:
    assert str(error) == 'observation', str(error)
else:
    raise AssertionError('disconnect admitted as a fully received response')
`], { encoding: 'utf8', env: { PATH: process.env.PATH } });
  assert.equal(result.status, 0, result.stderr);
});

test('owner proposal source ordering is independent of filesystem enumeration', () => {
  const { execFileSync } = require('node:child_process');
  const { ownerProposalInputs } = require('./run-bounded.cjs');
  const packet = fs.mkdtempSync(path.resolve('.amp/in/artifacts/owner-source-order-'));
  const oldPath = process.env.PATH, oldOrder = process.env.TEST_SOURCE_ORDER;
  const actualRg = execFileSync('which', ['rg'], { encoding: 'utf8' }).trim();
  try {
    const binary = path.join(packet, 'public-binary'), planFile = path.join(packet, 'public-plan.json');
    fs.writeFileSync(binary, 'public native binary');
    fs.writeFileSync(planFile, JSON.stringify({ nativePrepared: true, model: alias }));
    fs.writeFileSync(path.join(packet, 'rg'), `#!${process.execPath}\nimport {execFileSync} from 'node:child_process';
const paths=execFileSync(${JSON.stringify(actualRg)},process.argv.slice(2),{encoding:'utf8'}).trim().split('\\n').sort();
process.stdout.write((process.env.TEST_SOURCE_ORDER==='reverse'?paths.reverse():paths).join('\\n')+'\\n');\n`, { mode: 0o700 });
    process.env.PATH = packet + path.delimiter + oldPath;
    const args = [path.join(packet, 'uncreated-run'), binary, planFile,
      path.resolve('.planning/plans/crowd-lu-user-acceptance-v1-20261111')];
    process.env.TEST_SOURCE_ORDER = 'forward';
    const first = ownerProposalInputs(...args);
    process.env.TEST_SOURCE_ORDER = 'reverse';
    assert.deepEqual(ownerProposalInputs(...args), first,
      'the same source bytes must produce the same signed scope regardless of enumeration order');
  } finally {
    process.env.PATH = oldPath;
    if (oldOrder === undefined) delete process.env.TEST_SOURCE_ORDER; else process.env.TEST_SOURCE_ORDER = oldOrder;
    fs.rmSync(packet, { recursive: true, force: true });
  }
});

test('owner proposal fake credential lock is optional, exact, bounded and artifact-only', () => {
  const { ownerProposalInputs } = require('./run-bounded.cjs');
  const crypto = require('node:crypto');
  const packet = fs.mkdtempSync(path.resolve('.amp/in/artifacts/owner-node-lock-'));
  const output = path.join(packet, 'uncreated-run'), binary = path.join(packet, 'public-binary');
  const planFile = path.join(packet, 'public-plan.json'), credential = path.join(packet, '.env.local');
  const labels = path.resolve('.planning/plans/crowd-lu-user-acceptance-v1-20261111');
  const outside = fs.mkdtempSync(path.resolve('.amp/in/owner-node-outside-'));
  try {
    fs.writeFileSync(binary, 'public native binary');
    const plan = { nativePrepared: true, model: alias, observedTarget: target, route: 'fixture/fp8',
      provider: 'PublicFixture', input: { artist: 'Public Artist', album: 'Public Album', tracks: ['One', 'Other'] } };
    fs.writeFileSync(planFile, JSON.stringify(plan));
    const bytes = Buffer.from(`LLM_API_KEY=MOCK_ONLY_SECRET_CANARY\nLLM_MODEL=${alias}\n`);
    fs.writeFileSync(credential, bytes);
    const result = ownerProposalInputs(output, binary, planFile, labels, credential);
    const expected = { path: path.relative(process.cwd(), credential), sha256: crypto.createHash('sha256').update(bytes).digest('hex') };
    assert.deepEqual(result.bindings.sourcesAndTools.filter(lock => lock.path === expected.path), [expected]);
    assert.equal(fs.existsSync(output), false);
    assert(!JSON.stringify(result).includes('MOCK_ONLY_SECRET_CANARY'));
    for (const badBytes of [Buffer.from('not a public credential'), Buffer.concat([bytes, Buffer.from('\n')]), Buffer.alloc(8193)]) {
      fs.writeFileSync(credential, badBytes);
      assert.throws(() => ownerProposalInputs(output, binary, planFile, labels, credential), /credential/);
    }
    fs.unlinkSync(credential);
    fs.mkdirSync(credential);
    assert.throws(() => ownerProposalInputs(output, binary, planFile, labels, credential), /credential/);
    fs.rmdirSync(credential);
    const external = path.join(outside, '.env.local'); fs.writeFileSync(external, bytes);
    fs.symlinkSync(external, credential);
    assert.throws(() => ownerProposalInputs(output, binary, planFile, labels, credential), /credential/);
    assert.throws(() => ownerProposalInputs(output, binary, planFile, labels, external), /credential/);
  } finally { fs.rmSync(packet, { recursive: true, force: true }); fs.rmSync(outside, { recursive: true, force: true }); }
});

function publicOwnerBootstrapFixture() {
  const crypto = require('node:crypto');
  const canonical = value => value === null || typeof value !== 'object' ? JSON.stringify(value)
    : Array.isArray(value) ? '[' + value.map(canonical).join(',') + ']'
      : '{' + Object.keys(value).sort().map(k => JSON.stringify(k) + ':' + canonical(value[k])).join(',') + '}';
  const sha = value => crypto.createHash('sha256').update(value).digest('hex');
  const packetRoot = fs.mkdtempSync(path.resolve('.amp/in/artifacts/owner-node-bootstrap-'));
  const output = path.join(packetRoot, 'run'); fs.mkdirSync(output);
  const credential = path.join(packetRoot, '.env.local');
  const bytes = Buffer.from(`LLM_API_KEY=MOCK_ONLY_SECRET_CANARY\nLLM_MODEL=${alias}\n`);
  fs.writeFileSync(credential, bytes);
  const offlineCredential = { path: path.relative(process.cwd(), credential), sha256: sha(bytes) };
  const plan = { nativePrepared: true, model: alias };
  const membership = { evaluations: [{ name: 'public native component' }], requestedFieldSlots: 216 };
  const runtime = path.relative(process.cwd(), path.join(output, 'native-supervision.json'));
  const sources = [offlineCredential, { path: runtime, runtime: 'nativeSupervision' }];
  const proposal = { version: 1, proposalId: crypto.randomUUID(), executionKind: 'offlineRehearsal',
    executableSha256: sha('native binary'), membershipSha256: sha(canonical(membership)),
    nativePlanSha256: sha(canonical(plan)), requestSha256: sha('native request'), verifierSha256: sha('public verifier'),
    boundaryPricingSha256: sha('public pricing'), contextSha256: sha('public context'), sourcesAndTools: sources };
  const intent = { version: 1, proposalId: proposal.proposalId, proposalSha256: sha(canonical(proposal)),
    executionKind: 'offlineRehearsal', nonce: crypto.randomUUID(), issuedUnixSeconds: Math.floor(Date.now() / 1000),
    expiresUnixSeconds: Math.floor(Date.now() / 1000) + 1200, hostedEstimateRiskAccepted: false, signature: Array(64).fill(17) };
  const ownerExecution = { version: 1, proposalId: proposal.proposalId, proposalSha256: intent.proposalSha256,
    intentSha256: sha(canonical(intent)), executionKind: 'offlineRehearsal', nonce: intent.nonce,
    expiresUnixSeconds: intent.expiresUnixSeconds };
  return { packetRoot, output, plan, membership, packet: { version: 1, proposal, intent, ownerExecution, offlineCredential },
    bindings: { disposableRunRoot: output, executable: proposal.executableSha256, modelRehearsalPlan: plan,
      sourcesAndTools: [offlineCredential, { path: runtime, sha256: sha('native runtime') }] } };
}

test('native public owner bootstrap joins only the planned offline proposal, intent, root and fake source', () => {
  const { validateOwnerBootstrap } = require('./run-bounded.cjs');
  const f = publicOwnerBootstrapFixture();
  try {
    assert.deepEqual(validateOwnerBootstrap(f.packet, f.output, f.plan, f.membership, f.bindings), f.packet.ownerExecution);
    for (const mutate of [p => p.extra = true, p => p.version = 2,
      p => p.proposal.executionKind = 'hostedSingleRequest', p => p.intent.executionKind = 'hostedSingleRequest',
      p => p.ownerExecution.executionKind = 'hostedSingleRequest', p => p.proposal.proposalId = 'bad',
      p => p.proposal.requestSha256 = 'bad', p => p.proposal.extra = true,
      p => p.intent.signature = [1], p => p.intent.hostedEstimateRiskAccepted = true,
      p => p.intent.nonce = 'bad', p => p.intent.expiresUnixSeconds = 1,
      p => p.ownerExecution.nonce = require('node:crypto').randomUUID(), p => p.ownerExecution.expiresUnixSeconds++,
      p => p.ownerExecution.intentSha256 = 'a'.repeat(64), p => p.ownerExecution.proposalSha256 = 'b'.repeat(64),
      p => p.offlineCredential.path = '.env.local', p => p.offlineCredential.sha256 = 'c'.repeat(64),
      p => p.offlineCredential.extra = true]) {
      const changed = structuredClone(f.packet); mutate(changed);
      assert.throws(() => validateOwnerBootstrap(changed, f.output, f.plan, f.membership, f.bindings), /owner|credential/);
    }
    for (const mutate of [b => b.disposableRunRoot += '-other', b => b.executable = 'd'.repeat(64),
      b => b.sourcesAndTools.reverse(), b => b.sourcesAndTools.pop(), b => b.sourcesAndTools[0].sha256 = 'e'.repeat(64)]) {
      const changed = structuredClone(f.bindings); mutate(changed);
      assert.throws(() => validateOwnerBootstrap(f.packet, f.output, f.plan, f.membership, changed), /owner|credential/);
    }
    assert.throws(() => validateOwnerBootstrap(f.packet, f.output + '-other', f.plan, f.membership, f.bindings), /owner/);
    assert.throws(() => validateOwnerBootstrap(f.packet, f.output, { ...f.plan, model: 'other' }, f.membership, f.bindings), /owner|credential/);
    assert.throws(() => validateOwnerBootstrap(f.packet, f.output, f.plan, { ...f.membership, requestedFieldSlots: 1 }, f.bindings), /owner/);
  } finally { fs.rmSync(f.packetRoot, { recursive: true, force: true }); }
});

test('native owner bootstrap wait checks its existing scope and never retries malformed publication', async () => {
  const { waitForOwnerBootstrap } = require('./run-bounded.cjs');
  const f = publicOwnerBootstrapFixture(), file = path.join(f.output, 'owner-execution-bootstrap.json');
  try {
    let checks = 0;
    await assert.rejects(waitForOwnerBootstrap(f.output, () => { if (++checks === 3) throw Error('native preparation scope exhausted'); }), /scope exhausted/);
    assert.equal(checks, 3);
    assert.equal(fs.existsSync(file), false);
    fs.writeFileSync(file, JSON.stringify(f.packet));
    checks = 0;
    assert.deepEqual(await waitForOwnerBootstrap(f.output, () => { checks++; }), f.packet);
    assert.equal(checks, 2);
    await assert.rejects(waitForOwnerBootstrap(f.output, () => { throw Error('scope exhausted'); }), /scope exhausted/);
    for (const bytes of ['not JSON', 'x'.repeat(1024 * 1024 + 1)]) {
      fs.writeFileSync(file, bytes); checks = 0;
      await assert.rejects(waitForOwnerBootstrap(f.output, () => { checks++; }), /owner/);
      assert.equal(checks, 1);
    }
    fs.unlinkSync(file); fs.symlinkSync(path.join(f.packetRoot, '.env.local'), file);
    await assert.rejects(waitForOwnerBootstrap(f.output, () => {}), /owner/);
  } finally { fs.rmSync(f.packetRoot, { recursive: true, force: true }); }
});

test('preflight accepts only credential syntax supported by the native owner-file reader', () => {
  for (const text of [env,
    `export LLM_API_KEY = 'SECRET_NEVER_EXPORTED' # public fixture\nLLM_MODEL="${alias}"\nOTHER=value\n`,
    `LLM_API_KEY='PUBLIC#FIXTURE'\nLLM_MODEL=${alias}\nLLM_PROVIDER=openrouter\nLLM_BASE_URL=https://openrouter.ai/api/v1\n`]) {
    assert.equal(prepare(text, model(), hosts()).credentialPresent, true);
  }
  for (const text of [
    env + `LLM_MODEL=${alias}\n`,
    env + 'LLM_API_KEY=SECOND_PUBLIC_FIXTURE\n',
    env + 'LLM_PROVIDER=openrouter\nLLM_PROVIDER=openrouter\n',
    env + 'LLM_BASE_URL=https://openrouter.ai/api/v1\nLLM_BASE_URL=https://openrouter.ai/api/v1\n',
    `LLM_MODEL=${alias}\nLLM_API_KEY="PUBLIC\\nFIXTURE"\n`,
    `LLM_MODEL=${alias}\nLLM_API_KEY="PUBLIC\nFIXTURE"\n`,
    `LLM_MODEL=${alias}\nLLM_API_KEY="PUBLIC"suffix\n`,
    `LLM_MODEL=${alias}\nLLM_API_KEY=PUBLIC FIXTURE\n`,
    `LLM_MODEL=${alias}\nLLM_API_KEY=PUBLIC\\FIXTURE\n`,
    `LLM_MODEL=${alias}\nLLM_API_KEY=\`PUBLIC_FIXTURE\`\n`,
    `LLM_MODEL=${alias}\nexport\tLLM_API_KEY=PUBLIC_FIXTURE\n`,
    env + 'NOT_AN_ASSIGNMENT\n',
    `LLM_MODEL=${alias}\nLLM_API_KEY=${'X'.repeat(4097)}\n`,
    `LLM_MODEL=${alias}\nLLM_API_KEY=公開fixture\n`,
  ]) {
    assert.throws(() => prepare(text, model(), hosts()), /preflight_config_invalid|preflight_credential_missing/);
  }
});

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

function receiverArchiveFixture() {
  const root = fs.mkdtempSync(path.resolve('.amp/in/receiver-archive-'));
  const executable = path.join(root, 'storage.cjs');
  const config = { generation: 'archive-generation', receiverPid: 123, receiverGroup: 321 };
  const receipt = { state: 'complete', authority: 'native_witness_and_timely_supervisor_exit',
    generation: config.generation, receiverPid: 123, receiverGroup: 321, supervisorGroup: 987,
    executionGroup: 789, supervisorExitObservedAt: 190, acceptedAt: 191, deadline: 200,
    requestedFieldSlots: 216, liveAuthority: false, controlOnly: false,
    evidenceSha256: 'a'.repeat(64), resultSha256: 'b'.repeat(64) };
  fs.writeFileSync(path.join(root, 'native-supervision.json'), JSON.stringify(config));
  fs.writeFileSync(path.join(root, 'receiver-outcome.json'), JSON.stringify(receipt));
  // Disposable stand-in for the native terminal-launcher storage entry point.
  fs.writeFileSync(executable, `#!${process.execPath}\nconst fs = require('node:fs');
    if (process.env.SOUNDROBE_STORAGE_REQUIRED !== '1'
      || process.env.SOUNDROBE_STORAGE_ACTION !== 'terminal_launcher'
      || process.env.SOUNDROBE_NATIVE_GENERATION !== 'archive-generation') process.exit(78);
    fs.writeFileSync(process.env.SOUNDROBE_STORAGE_TARGET, fs.readFileSync(0), {flag:'wx'});\n`, { mode: 0o700 });
  return { root, executable, config, receipt, status: { pid: 123, status: 0, signal: null } };
}

test('receiver archival shares outcome validation and writes only through terminal-launcher storage', () => {
  const { archiveReceiverOutcome } = require('./run-supervised.cjs');
  const f = receiverArchiveFixture();
  try {
    const outcome = archiveReceiverOutcome(f.root, f.executable, f.status);
    assert.equal(outcome.state, 'complete');
    assert.equal(outcome.actualReceiverExitObserved, true);
    assert.equal(outcome.receiverPid, 123);
    assert.equal(outcome.receiverOutcomeSha256, require('node:crypto').createHash('sha256')
      .update(fs.readFileSync(path.join(f.root, 'receiver-outcome.json'))).digest('hex'));
    assert.deepEqual(JSON.parse(fs.readFileSync(path.join(f.root, 'launcher-outcome.json'))), outcome);
    // Exclusive native storage rejects a second publication; no JS overwrite.
    assert.equal(archiveReceiverOutcome(f.root, f.executable, f.status).state, 'incomplete');
    assert.deepEqual(JSON.parse(fs.readFileSync(path.join(f.root, 'launcher-outcome.json'))), outcome);
  } finally { fs.rmSync(f.root, { recursive: true }); }
});

test('receiver archival preserves incomplete denial for failed observations and invalid receipts', () => {
  const { archiveReceiverOutcome } = require('./run-supervised.cjs');
  for (const change of [f => f.status.status = 78, f => f.status.signal = 'SIGKILL',
    f => f.status.pid++, f => f.status.error = Error('not observed'), f => f.receipt.generation = 'stale',
    f => f.receipt.acceptedAt = f.receipt.deadline, f => f.receipt.controlOnly = true,
    f => f.receipt.state = 'provisional_complete']) {
    const f = receiverArchiveFixture();
    try {
      change(f); fs.writeFileSync(path.join(f.root, 'receiver-outcome.json'), JSON.stringify(f.receipt));
      const outcome = archiveReceiverOutcome(f.root, f.executable, f.status);
      assert.equal(outcome.state, 'incomplete');
      assert.equal(outcome.validatedFieldSlots, 0); assert.equal(outcome.unavailableFieldSlots, 216);
      assert.equal(outcome.actualReceiverExitObserved, undefined);
      assert.deepEqual(JSON.parse(fs.readFileSync(path.join(f.root, 'launcher-outcome.json'))), outcome);
    } finally { fs.rmSync(f.root, { recursive: true }); }
  }
  const f = receiverArchiveFixture();
  try {
    fs.unlinkSync(f.executable);
    assert.equal(archiveReceiverOutcome(f.root, f.executable, f.status).state, 'incomplete');
    assert.equal(fs.existsSync(path.join(f.root, 'launcher-outcome.json')), false);
  } finally { fs.rmSync(f.root, { recursive: true }); }
});

test('launcher rejects public receiver PID/status claims without observing a child', () => {
  const script = require.resolve('./run-supervised.cjs');
  const f = receiverArchiveFixture();
  const run = args => spawnSync(process.execPath, [script, '--archive-receiver', f.root, f.executable, ...args],
    { encoding: 'utf8', env: { PATH: process.env.PATH } });
  try {
    for (const args of [['0', '0'], ['-1', '0'], ['1.5', '0'], ['NaN', '0'], ['Infinity', '0'],
      ['9007199254740992', '0'], ['123x', '0'], ['0123', '0'], [' 123', '0'], ['123', '00'],
      ['123', '-0'], ['123', '1'], ['123'], ['123', '0', 'SIGTERM']]) {
      const result = run(args);
      assert.equal(result.status, 1, JSON.stringify(args)); assert.equal(result.stdout, '');
      assert.equal(fs.existsSync(path.join(f.root, 'launcher-outcome.json')), false);
    }
    const result = run(['123', '0']);
    assert.equal(result.status, 1, 'public PID/status claims must not become observed exit');
    assert.equal(result.stdout, '');
    assert.equal(fs.existsSync(path.join(f.root, 'launcher-outcome.json')), false);
  } finally { fs.rmSync(f.root, { recursive: true }); }
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

function ownerOfflineReceiptFixture() {
  const crypto = require('node:crypto');
  const control = ownerCoordinationControl();
  const report = structuredClone(control.report);
  report.state = 'provisional_complete'; report.denied = 0; report.ownerLive = false;
  report.bindings.supervision = { generation: 'owner-generation' };
  report.bindings.ownerExecution = { version: 1, proposalId: crypto.randomUUID(),
    proposalSha256: 'c'.repeat(64), intentSha256: 'd'.repeat(64), executionKind: 'offlineRehearsal',
    nonce: crypto.randomUUID(), expiresUnixSeconds: control.consumed.expiresUnixSeconds };
  report.resultSha256 = 'e'.repeat(64);
  report.credentialLookups = report.actualFakeLookups = 1;
  report.counters = { reserved: 2, arrived: 1, completed: 2, model: 1, provider: 1 };
  report.sessions = Object.fromEntries(['preparation', 'execution'].flatMap(stage =>
    require('./run-bounded.cjs').pairedColdJobs().map(job => [stage + job.name, { stage, job: job.name, closed: true }])));
  const bodySha256 = crypto.createHash('sha256').update(JSON.stringify({ model: alias,
    messages: [{ role: 'user', content: 'Public owner offline request' }] })).digest('hex');
  const reservationSha256 = crypto.createHash('sha256')
    .update(JSON.stringify(['POST', '/model/chat/completions', bodySha256])).digest('hex');
  report.operations = {
    provider: { category: 'provider', dispatched: true, arrived: true, completed: true },
    model: { category: 'model', dispatched: true, arrived: false, completed: true,
      request_sha256: reservationSha256, response: { sha256: 'a'.repeat(64), bytes: 512, status: 200,
        observation: 'native_http_response', remoteArrivalVerified: false,
        http: { version: 1, transport: 'offline_local_http', requestSha256: bodySha256, validation: 'accepted' } } },
  };
  return report;
}

test('owner offline staging has a distinct validator, retaining native-only scope and no remote arrival claim', () => {
  const { validateOwnerOfflineReceipt, validateCompleteReceipt, validateProvisionalReceipt } = require('./mock-authority.cjs');
  const report = ownerOfflineReceiptFixture();
  assert.equal(validateOwnerOfflineReceipt(report, structuredClone(report)), report);
  assert.throws(() => validateProvisionalReceipt(report, structuredClone(report)), /native/);
  const complete = { ...report, state: 'complete' };
  assert.equal(validateOwnerOfflineReceipt(complete, structuredClone(complete), 'complete'), complete);
  assert.throws(() => validateCompleteReceipt(complete, structuredClone(complete)), /native/);
  for (const mutate of [r => delete r.bindings.ownerExecution,
    r => r.bindings.ownerExecution.executionKind = 'hostedSingleRequest',
    r => r.bindings.ownerExecution.extra = true, r => r.bindings.ownerExecution.version = 2,
    r => r.bindings.ownerExecution.intentSha256 = 'bad', r => r.bindings.ownerExecution.nonce = 'bad',
    r => r.bindings.ownerExecution.expiresUnixSeconds = 0,
    r => r.bindings.modelRehearsalPlan.nativePrepared = false,
    ...['offlineFixture', 'offlineHosted', 'offlineHttp', 'offlineResponse'].flatMap(key => [
      r => r.bindings.modelRehearsalPlan[key] = false, r => r.bindings.modelRehearsal[key] = false]),
    r => r.bindings.modelRehearsal.boundary.liveDispatchAvailable = true,
    r => r.testPolicy = 'unknown', r => r.credentialLookups = 0, r => r.actualFakeLookups = 2,
    r => r.operations.model.arrived = true, r => r.operations.model.response.remoteArrivalVerified = true,
    r => r.operations.model.response.observation = 'offline_injected_native_response',
    r => r.operations.model.response.http.transport = 'hosted_https',
    r => r.operations.extra = structuredClone(r.operations.model), r => r.counters.model = 0,
    r => r.counters.arrived = 2, r => r.membership.requestedFieldSlots = 215,
    r => delete r.sessions['executionassistance-crowd'], r => r.sessions['executionassistance-crowd'].job = 'other',
    r => r.sessions['executionassistance-crowd'].closed = false,
    r => r.resultSha256 = 'bad', r => delete r.bindings.supervision]) {
    const changed = structuredClone(report); mutate(changed);
    assert.throws(() => validateOwnerOfflineReceipt(changed, structuredClone(changed)));
  }
  const changed = structuredClone(report); changed.bindings.ownerExecution.proposalSha256 = '0'.repeat(64);
  assert.throws(() => validateOwnerOfflineReceipt(changed, report));
});

test('only the validated owner offline binding unlocks native prepared pending execution', () => {
  const { validateOwnerOfflineExecution } = require('./run-bounded.cjs');
  const report = ownerOfflineReceiptFixture();
  report.state = 'executing'; report.credentialLookups = report.actualFakeLookups = 0;
  report.counters = { model: 0 }; report.operations = {};
  assert.equal(validateOwnerOfflineExecution(report.bindings.modelRehearsalPlan, report.bindings.ownerExecution,
    report), true);
  for (const mutate of [r => delete r.bindings.ownerExecution,
    r => r.bindings.ownerExecution.executionKind = 'hostedSingleRequest', r => r.consumptions = 0,
    r => r.bindings.modelRehearsalPlan.offlineHttp = true, r => r.liveAuthority = true,
    r => r.state = 'frozen', r => r.credentialLookups = 1, r => r.counters.model = 1]) {
    const changed = structuredClone(report); mutate(changed);
    assert.throws(() => validateOwnerOfflineExecution(report.bindings.modelRehearsalPlan,
      report.bindings.ownerExecution, changed), /native_prepared_execution_disabled/);
  }
});

test('owner offline HTTP observation reconciles the native model separately from provider wire arrivals', () => {
  const { validateOwnerOfflineHttpObservation } = require('./run-bounded.cjs');
  const report = ownerOfflineReceiptFixture();
  const observation = { version: 1, transport: 'offline_local_http', requests: 1,
    requestSha256: report.operations.model.response.http.requestSha256,
    requestBytes: 1024, responseSha256: 'a'.repeat(64), responseBytes: 512,
    status: 200, exactRequest: true, fakeCredentialPresented: true, liveAuthority: false };
  assert.notEqual(report.operations.model.request_sha256, observation.requestSha256);
  assert.equal(validateOwnerOfflineHttpObservation(report, observation), true);
  for (const mutate of [o => o.requests = 2, o => o.requestSha256 = '0'.repeat(64),
    o => o.responseSha256 = '0'.repeat(64), o => o.responseBytes++, o => o.status = 429,
    o => o.transport = 'hosted_https', o => o.requestBytes = 32769, o => o.exactRequest = false,
    o => o.fakeCredentialPresented = false, o => o.liveAuthority = true]) {
    const changed = structuredClone(observation); mutate(changed);
    assert.throws(() => validateOwnerOfflineHttpObservation(report, changed));
  }
  for (const substituted of [observation.requestSha256,
    require('node:crypto').createHash('sha256').update(JSON.stringify([
      'POST', '/api/v1/chat/completions', observation.requestSha256])).digest('hex')]) {
    const changed = structuredClone(report); changed.operations.model.request_sha256 = substituted;
    assert.throws(() => validateOwnerOfflineHttpObservation(changed, observation), /reconciliation failed/);
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
