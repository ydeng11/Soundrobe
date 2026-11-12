// Mock-only integration of the maintained native evaluator and frozen scorer.
// Usage: node scripts/auto-tag-eval-replay/run-bounded.cjs NEW_OUTPUT TEST_EXECUTABLE [FROZEN_USER_LABEL_DIRECTORY]
const fs = require('node:fs'), path = require('node:path'), crypto = require('node:crypto');
const { spawn, execFileSync } = require('node:child_process');
const { once } = require('node:events');
const { createService, importPools } = require('../mock-provider-service.cjs');
const root = process.cwd();
const read = p => JSON.parse(fs.readFileSync(p));
const sha = bytes => crypto.createHash('sha256').update(bytes).digest('hex');
const binding = p => ({ path: path.relative(root, p), sha256: sha(fs.readFileSync(p)) });
const save = (p, value) => fs.writeFileSync(p, JSON.stringify(value, null, 2) + '\n', { flag: 'wx' });
const canonical = value => value === null || typeof value !== 'object' ? JSON.stringify(value)
  : Array.isArray(value) ? '[' + value.map(canonical).join(',') + ']'
    : '{' + Object.keys(value).sort().map(k => JSON.stringify(k) + ':' + canonical(value[k])).join(',') + '}';
const fixtures = path.join(root, 'test/fixtures/tauri/auto-tag-eval');
const scorer = path.join(root, 'scripts/auto-tag-eval-replay/score-observations.py');
const orchestration = { childTimeoutMs: 600000, pythonTimeoutMs: 60000, inventoryTimeoutMs: 15000, inventoryBytes: 128 * 1024 * 1024 };
let admittedOutput;
function pairedColdJobs() {
  const cases = {
    crowd: { caseId: '036e0a00e45b', profile: 'tagged_recovery', population: 'artist-batch-extension', review: 'reviewed-cjk-artist-2-20261108.json' },
    billie: { caseId: '54e7923a3000', profile: 'tagged_recovery', population: 'original', review: 'reviewed-expansion-2-20261108.json' },
    enya: { caseId: 'f7756354af37', profile: 'folder_filename', population: 'original', review: 'reviewed-expansion-2-20261108.json' },
  };
  return [['disabled', 'crowd'], ['disabled', 'billie'], ['disabled', 'enya'],
    ['assistance', 'billie'], ['assistance', 'enya'], ['assistance', 'crowd']].map(([arm, name]) => ({
      ...cases[name], name: `${arm}-${name}`, arm, role: arm === 'assistance' && name === 'crowd' ? 'GenreFillResponse' : null }));
}
function failureEnvelope(output) {
  const requested = read(path.join(output, 'requested-membership.json'));
  const evaluations = requested.evaluations.map(e => {
    const ids = e.physicalPaths.flatMap(p => e.fields.map(f => `${e.arm}/${e.caseId}/${e.profile}/${e.phase}/${p}/${f}`));
    const resultFile = path.join(output, e.name, 'result.provisional.json');
    let score = null;
    try {
      const result = read(resultFile);
      if (['name', 'arm', 'caseId', 'profile'].every(k => result[k] === e[k])
          && result.score.requestedFieldSlots === ids.length
          && canonical(result.score.fields.map(f => f.id).sort()) === canonical([...ids].sort())) score = result.score;
    } catch { /* Unvalidated exports never become observations. */ }
    return { ...e, nativeObservation: score ? 'validated_projection' : 'unavailable',
      score, requestedFieldSlots: ids.length, observedFieldSlots: score?.observedFieldSlots ?? 0,
      serviceFailureSlots: score?.serviceFailureSlots ?? ids.length,
      projectionFile: score ? path.relative(root, resultFile) : null };
  });
  return { ...requested, evaluations, outcome: 'incomplete',
    observedFieldSlots: evaluations.reduce((n, e) => n + e.observedFieldSlots, 0),
    serviceFailureSlots: evaluations.reduce((n, e) => n + e.serviceFailureSlots, 0),
    groundedAssistanceAccuracyCredit: 0, acceptedCertificate: false, writableDelta: [] };
}
module.exports = { pairedColdJobs, failureEnvelope };
async function inventoryAt(base, compact = false, session) {
  const response = await fetch(`${base}/__fixtures${compact ? '/compact' : ''}`, { signal: AbortSignal.timeout(orchestration.inventoryTimeoutMs), redirect: 'error',
    headers: session ? { 'x-soundrobe-mock-session': session } : {} });
  if (!response.ok) throw Error('Inventory status failed');
  const chunks = []; let size = 0;
  for await (const chunk of response.body) {
    size += chunk.length;
    if (size > (compact ? 8388608 : orchestration.inventoryBytes)) throw Error('Inventory byte ceiling');
    chunks.push(chunk);
  }
  return JSON.parse(Buffer.concat(chunks).toString());
}

async function main(output, executable, userLabelDirectory, mode) {
  if (mode !== undefined && !['--paired-cold', '--durable-mock'].includes(mode)) throw Error('Unsupported evaluation mode');
  const durable = mode === '--durable-mock';
  const paired = mode === '--paired-cold' || durable;
  if (paired && !userLabelDirectory) throw Error('Paired cold selection requires frozen scorer-only user labels');
  const phases = paired ? ['cold'] : ['cold', 'warm'];
  output = path.resolve(output);
  if (!output.startsWith(path.join(root, '.amp/in/artifacts') + path.sep)) throw Error('Use a new local evidence directory');
  const evidenceRoot = fs.realpathSync(path.join(root, '.amp/in/artifacts'));
  if (!evidenceRoot.startsWith(fs.realpathSync(root) + path.sep)) throw Error('Evidence anchor escaped checkout');
  if (!fs.realpathSync(path.dirname(output)).startsWith(evidenceRoot + path.sep)
      && fs.realpathSync(path.dirname(output)) !== evidenceRoot) throw Error('Evidence parent escaped local artifacts');
  fs.mkdirSync(output);
  admittedOutput = output;
  const binary = path.join(output, 'native-evaluator');
  fs.copyFileSync(path.resolve(executable), binary, fs.constants.COPYFILE_EXCL);
  // Expectations and role/safety scope precede ALL preparation and final outputs.
  const jobs = paired ? pairedColdJobs() : [
    { name: 'provider', caseId: '0c9e8b505ea7', profile: 'assisted_without_ids', population: 'original', role: null, review: 'reviewed-subset.json' },
    { name: 'genre', caseId: '036e0a00e45b', profile: 'tagged_recovery', population: 'artist-batch-extension', role: 'GenreFillResponse', review: 'reviewed-cjk-artist-2-20261108.json' },
    { name: 'unknown-usage', caseId: '036e0a00e45b', profile: 'tagged_recovery', population: 'artist-batch-extension', role: 'GenreFillResponse', review: 'reviewed-cjk-artist-2-20261108.json', unknownUsage: true },
    { name: 'cancel', caseId: '036e0a00e45b', profile: 'tagged_recovery', population: 'artist-batch-extension', role: 'GenreFillResponse', review: 'reviewed-cjk-artist-2-20261108.json', cancelAfterGenre: true },
    { name: 'hold', caseId: 'f7756354af37', profile: 'folder_filename', population: 'original', role: null, review: 'reviewed-expansion-2-20261108.json' },
  ];
  const userBindings = userLabelDirectory ? {
    userAcceptance: binding(path.resolve(userLabelDirectory, 'labels.json')),
    userAcceptanceSeal: binding(path.resolve(userLabelDirectory, 'SHA256SUMS')),
  } : {};
  // Scoring-only admission happens before any native invocation. The inspector
  // returns membership/locks, never answer strings for request construction.
  const userLabels = userLabelDirectory ? JSON.parse(execFileSync('python3', ['-B', scorer, 'inspect-user-acceptance',
    userBindings.userAcceptance.path, userBindings.userAcceptance.sha256,
    userBindings.userAcceptanceSeal.path, userBindings.userAcceptanceSeal.sha256], { timeout: orchestration.pythonTimeoutMs })) : null;
  const matchesUserLabels = job => userLabels && job.caseId === userLabels.caseId && job.profile === userLabels.profile;
  const lockedFiles = [binary, __filename, scorer, 'scripts/mock-provider-service.cjs', 'scripts/auto-tag-eval-replay/mock-authority.cjs',
    ...Object.values(userBindings).map(b => b.path), ...(userLabels?.lockedPaths ?? []),
    'scripts/review_auto_tag_subset.py', 'src-tauri/Cargo.toml', 'src-tauri/Cargo.lock',
    '.planning/research/bounded-assistance-machine-labels-20261111.json',
    '.planning/research/bounded-assistance-label-review-20261111.json',
    ...execFileSync('rg', ['--files', 'src-tauri/src'], { encoding: 'utf8' }).trim().split('\n'),
    ...jobs.flatMap(j => [path.join(root, '.amp/in/eval-dummies', j.population, 'corpus.json'),
      path.join(fixtures, j.review)])];
  const locks = [...new Set(lockedFiles.map(p => path.resolve(p)))].map(binding);
  save(path.join(output, 'source-and-tool-locks.json'), locks);
  const checkLocks = () => { for (const lock of locks) {
    if (binding(path.join(root, lock.path)).sha256 !== lock.sha256) throw Error(`Changed frozen source/tool: ${lock.path}`);
  } };
  const python = args => {
    checkLocks();
    try { return execFileSync('python3', ['-B', ...args], { timeout: orchestration.pythonTimeoutMs }); }
    finally { checkLocks(); }
  };
  save(path.join(output, 'selection.json'), { version: paired ? 2 : 1, jobs, phases, fields: ['title', 'artists', 'genre'],
    model: 'scripted/genre-v1', purpose: 'scripted mock wiring, zero grounded accuracy',
    realProviderCalls: 0, realModelCalls: 0, originalMediaOperations: 0,
    orchestration: { ...orchestration, ...(paired ? { inventoryBytes: 8388608 } : {}) }, userBindings });
  const independentLabels = read(path.join(root, '.planning/research/bounded-assistance-machine-labels-20261111.json'));
  const evaluations = jobs.flatMap(job => {
    const corpus = read(path.join(root, '.amp/in/eval-dummies', job.population, 'corpus.json'));
    const selected = corpus.cases.filter(c => c.caseId === job.caseId);
    if (selected.length !== 1) throw Error('Ambiguous requested physical case');
    return phases.map(phase => ({ name: job.name, ...(paired ? { arm: job.arm } : {}), caseId: job.caseId, profile: job.profile, phase,
      physicalPaths: selected[0].tracks.map(t => t.relativePath), fields: ['title', 'artists', 'genre'],
      userLabelSelectors: matchesUserLabels(job) ? userLabels.labelSelectors : [],
      labelSelectors: independentLabels.occurrences.filter(l => l.caseId === job.caseId)
        .flatMap(l => ['title', 'artists'].map(field => `${l.caseId}/${l.inventorySource}/${l.inventoryOccurrence}/${field}`)) }));
  });
  save(path.join(output, 'requested-membership.json'), { evaluations,
    requestedPhysicalOccurrences: evaluations.reduce((n, e) => n + e.physicalPaths.length, 0),
    requestedFieldSlots: evaluations.reduce((n, e) => n + e.physicalPaths.length * e.fields.length, 0),
    predeclaredLabelableSlots: evaluations.reduce((n, e) => n + e.labelSelectors.length + e.userLabelSelectors.length, 0),
    uniqueUserAcceptanceLabels: new Set(evaluations.flatMap(e => e.userLabelSelectors)).size,
    uniqueSelectedOccurrenceFieldLabels: new Set(evaluations.flatMap(e => [...e.labelSelectors, ...e.userLabelSelectors])).size });
  const providers = path.join(output, 'providers.json');
  importPools(path.join(fixtures, 'candidate-pools.json'), providers);
  const pool = read(providers);
  const records = pool.records.map(record => ({ ...record,
    ...(record.bodyFile ? { bodyFile: path.resolve(output, record.bodyFile) } : {}) }));
  const fixtureLocks = [...new Set(records.filter(r => r.bodyFile).map(r => r.bodyFile))].map(binding);
  locks.push(...fixtureLocks, binding(path.join(fixtures, 'candidate-pools.json')), binding(path.join(fixtures, 'provider-discovery.json')));
  save(path.join(output, 'fixture-source-locks.json'), fixtureLocks);
  const executions = [];
  const pending = [], preparations = [];
  if (durable) activeAuthority = await require('./mock-authority.cjs').start(binary, output,
    read(path.join(output, 'requested-membership.json')), { executable: binding(binary).sha256, sourcesAndTools: locks });
  const authority = durable ? activeAuthority : null;
  for (const job of jobs) {
    const directory = path.join(output, job.name);
    fs.mkdirSync(directory);
    const corpus = path.join(root, '.amp/in/eval-dummies', job.population, 'corpus.json');
    const sourceRoot = path.join(root, '.amp/in/eval-dummies', job.population, 'media');
    if (!fs.realpathSync(sourceRoot).startsWith(path.join(root, '.amp/in/eval-dummies') + path.sep)) throw Error('Generated root escaped');
    const item = read(corpus).cases.find(c => c.caseId === job.caseId);
    if (!item) throw Error('Selected physical case missing');
    const folder = path.join(sourceRoot, item.sourceRelativeFolder);
    if (path.isAbsolute(item.sourceRelativeFolder) || item.sourceRelativeFolder.split('/').some(p => !p || p === '.' || p === '..')
        || !fs.realpathSync(folder).startsWith(fs.realpathSync(sourceRoot) + path.sep)) throw Error('Invalid source folder');
    // Match the maintained native collector: visible regular audio files only,
    // excluding symlinks and subdirectories. Check names BEFORE content reads.
    const actual = fs.readdirSync(folder, { withFileTypes: true })
      .filter(e => e.isFile() && !e.name.startsWith('.') && /\.(mp3|flac|m4a|mp4|wav|ogg|opus|aiff|ape)$/i.test(e.name))
      .map(e => e.name).sort();
    if (JSON.stringify(actual) !== JSON.stringify(item.tracks.map(t => t.relativePath).sort())) throw Error('Source directory differs from frozen audio membership');
    for (const track of item.tracks) {
      for (const relative of [item.sourceRelativeFolder, track.relativePath]) {
        if (path.isAbsolute(relative) || relative.split('/').some(p => !p || p === '.' || p === '..')) throw Error('Invalid relative media path');
      }
      const file = path.join(sourceRoot, item.sourceRelativeFolder, track.relativePath);
      if (!fs.realpathSync(file).startsWith(fs.realpathSync(sourceRoot) + path.sep)) throw Error('Generated media escaped its root');
      if (sha(fs.readFileSync(file)) !== track.sourceSha256) throw Error('Changed generated media');
    }
    const review = path.join(directory, 'review');
    const reviewed = read(path.join(fixtures, job.review));
    const selectedReview = path.join(directory, 'selected-review.json');
    const selectedRows = reviewed.cases.filter(c => c.caseId === job.caseId);
    if (selectedRows.length !== 1) throw Error('Selected independent review is absent or ambiguous');
    save(selectedReview, { ...reviewed, cases: selectedRows });
    python(['scripts/review_auto_tag_subset.py', '--corpus', corpus,
      '--manifest', selectedReview, '--fixture-root', fixtures, '--output-dir', review]);
    const expectations = path.join(review, 'expectations.json');
    const serviceManifest = path.join(directory, 'service.json');
    // A genuine, independently captured catalog source supplies the prompt INPUT,
    // never a model answer key. The scripted response is not an accepted genre.
    const source = job.role ? read(path.join(fixtures, 'provider-snapshots/review-expansion-2-20261108/musicbrainz-release-7935e252-463d-4207-bce9-594d0a84e63a.json')) : null;
    const payload = source ? { artist: '盧廣仲', album: source.title, tracks: source.media.flatMap(m => m.tracks.map(t => t.title)) } : null;
    const body = job.role ? { model: 'scripted/genre-v1', temperature: 0.2, max_tokens: 1024,
      messages: [{ role: 'system', content: 'Infer a conservative music genre from the supplied artist, album, and track titles. Return a concise Discogs-style comma-separated genre and confidence. Use low confidence when the evidence is insufficient.' },
        { role: 'user', content: canonical(payload) }],
      response_format: { type: 'json_schema', json_schema: { name: job.role, schema: {
        type: 'object', properties: { genre: { type: 'string' }, confidence: { type: 'number' } }, required: ['genre', 'confidence'] } } } } : null;
    if (paired && body) fs.writeFileSync(path.join(directory, 'independent-request.json'), canonical(body), { flag: 'wx' });
    save(serviceManifest, { ...pool, records, ...(job.role ? { scriptedModel: { maxRequests: paired ? 1 : 2, payload,
      ...(paired ? { bodySha256: sha(Buffer.from(canonical(body))) } : {}),
      response: { genre: 'Scripted diagnostic genre', confidence: 0.8 },
      ...(job.unknownUsage ? {} : { usage: { prompt_tokens: 7, completion_tokens: 3, total_tokens: 10 } }) } } : {}) });
    let session, fixtureLease;
    let fixtureAuthority = authority ? { socket: authority.socket, onFatal: () => fixtureLease?.abandon() } : undefined;
    let service = createService(serviceManifest, fixtureAuthority);
    service.listen(0, '127.0.0.1'); await once(service, 'listening');
    let base = `http://127.0.0.1:${service.address().port}`;
    async function register(stage) {
      session = await authority.command({ op: 'register', job: job.name, stage, endpoint: base,
        manifest_sha256: binding(serviceManifest).sha256 });
      fixtureAuthority.service = session.service;
      fixtureLease = await require('./mock-authority.cjs').lease(authority.socket, session.service, 'fixture');
      base = authority.gateway;
    }
    if (authority) await register('preparation');
    const inventoryFile = path.join(directory, 'approved-fixtures.json');
    const env = Object.fromEntries(['PATH', 'HOME', 'TMPDIR'].filter(k => process.env[k]).map(k => [k, process.env[k]]));
    Object.assign(env, { SOUNDROBE_AUTO_TAG_EVAL_CORPUS: corpus, SOUNDROBE_AUTO_TAG_EVAL_SOURCE_ROOT: sourceRoot,
      SOUNDROBE_AUTO_TAG_EVAL_EXPECTATIONS: expectations, SOUNDROBE_AUTO_TAG_EVAL_PROFILE: job.profile,
      SOUNDROBE_AUTO_TAG_EVAL_CASES: job.caseId, SOUNDROBE_AUTO_TAG_EVAL_ARTISTS: '',
      SOUNDROBE_AUTO_TAG_EVAL_MOCK_URL: base, SOUNDROBE_AUTO_TAG_EVAL_ASSISTANCE: 'disabled' });
    if (paired) Object.assign(env, { SOUNDROBE_AUTO_TAG_EVAL_ARM: job.arm,
      SOUNDROBE_AUTO_TAG_EVAL_ASSISTANCE: job.role ? 'scripted' : 'disabled' });
    const safetyInput = path.join(directory, 'safety-input.json');
    save(safetyInput, { cancelAfterGenre: job.cancelAfterGenre === true });
    env.SOUNDROBE_AUTO_TAG_EVAL_SAFETY_INPUT = safetyInput;
    async function invoke(name, bounded) {
      checkLocks();
      const target = path.join(directory, name); fs.mkdirSync(target);
      if (authority) {
        const descriptor = path.join(target, 'session.json');
        save(descriptor, { client: { socket: authority.socket, handle: session.client }, job: job.name,
          stage: name === 'preparation' ? 'preparation' : 'execution', gateway: base,
          retained_root: path.join(directory, 'retained-native'),
          preparation: name === 'preparation' ? null : path.join(directory, 'preparation', 'preparation.json') });
        env.SOUNDROBE_MOCK_EVALUATION = descriptor;
      }
      const log = fs.openSync(path.join(target, 'native.log'), 'wx');
      try {
        const child = spawn(binary, ['--exact', `commands::auto_tag::eval_tests::${authority ? 'durable_cold_mock_auto_tag_eval' : paired ? 'paired_cold_mock_auto_tag_eval' : bounded ? 'bounded_mock_auto_tag_eval' : 'live_auto_tag_eval'}`,
          '--ignored', '--nocapture'], { cwd: root, env: { ...env, SOUNDROBE_AUTO_TAG_EVAL_ARTIFACT_DIR: target,
            SOUNDROBE_AUTO_TAG_EVAL_RUN_ID: `${job.name}-${name}` }, detached: true, stdio: ['ignore', log, log] });
        let watchdogExpired = false;
        const timer = setTimeout(() => { watchdogExpired = true;
          try { process.kill(-child.pid, 'SIGKILL'); } catch (error) { if (error.code !== 'ESRCH') console.error(error); }
        }, orchestration.childTimeoutMs);
        const [exitCode, signal] = await once(child, 'exit').finally(() => clearTimeout(timer));
        checkLocks();
        const exported = phases.flatMap(phase => {
          const file = path.join(target, phase + '.jsonl');
          return fs.existsSync(file) ? fs.readFileSync(file, 'utf8').trim().split('\n').filter(Boolean).map(JSON.parse) : [];
        });
        save(path.join(target, 'records.json'), exported);
        const observed = await inventoryAt(base, paired, session?.client);
        save(path.join(target, 'inventory-final.json'), observed);
        save(path.join(target, 'execution.json'), { exitCode, signal, watchdogExpired, records: exported.length,
          providerRequests: observed.requests.length, modelRequests: observed.modelRequests.length,
          wireArrivals: observed.wireArrivals.length, misses: observed.misses.length });
        return { target, exitCode, signal, exported, inventory: observed };
      } finally { fs.closeSync(log); }
    }
    try {
      if (paired) {
        save(inventoryFile, { records: (await inventoryAt(base, true, session?.client)).records });
        async function project(final, preparationFile) {
        if (!fs.existsSync(preparationFile)) throw Error(`No validated preparation for ${job.name}; retained requested membership`);
        const bindings = { corpus: binding(corpus), expectations: binding(expectations),
          ...(matchesUserLabels(job) ? userBindings : {}),
          labels: binding(path.join(root, '.planning/research/bounded-assistance-machine-labels-20261111.json')),
          packet: binding(path.join(root, '.planning/research/bounded-assistance-label-review-20261111.json')),
          preparation: binding(preparationFile), source: binding(path.join(root, 'src-tauri/src/commands/auto_tag.rs')),
          fixtures: binding(inventoryFile), service: binding(serviceManifest), executable: binding(binary), driver: binding(__filename), scorer: binding(scorer) };
        const specFile = path.join(directory, 'projection-spec.json');
        save(specFile, { version: 2, mode: 'mock', arm: job.arm, bindings, fields: ['title', 'artists', 'genre'], phases,
          sourceRoot: path.relative(root, sourceRoot), caseIds: [job.caseId], profile: job.profile });
        const preparationRows = path.join(directory, 'preparation-inputs.json'); save(preparationRows, [read(preparationFile)]);
        const contract = path.join(directory, 'scoring-contract.json');
        python([scorer, 'freeze-native', specFile, preparationRows, contract]);
        const scoreFile = path.join(directory, 'score.provisional.json');
        python([scorer, 'project-native', contract, binding(contract).sha256, path.join(final.target, 'records.json'), '-', scoreFile]);
        const plan = read(preparationFile).plan;
        const requestEvents = plan.events.filter(e => e.kind === 'request');
        if (requestEvents.length > (job.role ? 1 : 0) || final.inventory.modelRequests.length !== requestEvents.length
            || requestEvents.some(e => canonical(e.body) !== canonical(body))
            || final.inventory.modelRequests.some(r => r.sha256 !== sha(Buffer.from(canonical(body))))) throw Error('Conditional request or serialized-body binding differs');
        executions.push({ name: job.name, arm: job.arm, caseId: job.caseId, profile: job.profile,
          exitCode: final.exitCode, signal: final.signal, requestedPhysicalOccurrences: item.tracks.length,
          providerRequests: final.inventory.requests.length, modelRequests: final.inventory.modelRequests.length,
          wireArrivals: final.inventory.wireArrivals.length, misses: final.inventory.misses.length,
          planVariant: plan.variant, liveAuthority: false, score: read(scoreFile) });
        save(path.join(directory, 'result.provisional.json'), executions.at(-1));
        console.log(JSON.stringify({ name: job.name, variant: plan.variant, modelRequests: final.inventory.modelRequests.length, score: read(scoreFile).outcomes }));
        if (final.exitCode !== 0 || final.signal || final.inventory.misses.length || final.exported.length !== 1
            || executions.at(-1).score.observedFieldSlots !== executions.at(-1).score.requestedFieldSlots
            || executions.at(-1).score.serviceFailureSlots !== 0
            || final.exported.some(r => r.serviceFailureObserved || !r.payloadUnchanged || r.pairedMock.stopped)) throw Error('Cold mock execution non-clean; retain partial scoring and full membership');
        }
        if (authority) {
          const preparation = await invoke('preparation', false);
          const preparationFile = path.join(preparation.target, 'preparation.json');
          if (preparation.exitCode !== 0 || preparation.signal || preparation.inventory.misses.length
              || preparation.exported.length || preparation.inventory.modelRequests.length || !fs.existsSync(preparationFile)) throw Error('Durable preparation incomplete');
          const prepared = read(preparationFile);
          const requestEvents = prepared.plan.events.filter(e => e.kind === 'request');
          if (requestEvents.length > (job.role ? 1 : 0) || requestEvents.some(e => canonical(e.body) !== canonical(body))) throw Error('Prepared mock request differs');
          preparations.push({ job: job.name, input: prepared.fieldObservations.inputSha256, inventory: preparation.inventory });
          await require('./mock-authority.cjs').rpc(authority.socket, session.client, { op: 'close' });
          pending.push(async () => {
            fixtureAuthority = { socket: authority.socket, onFatal: () => fixtureLease?.abandon() };
            service = createService(serviceManifest, fixtureAuthority);
            service.listen(0, '127.0.0.1'); await once(service, 'listening');
            base = `http://127.0.0.1:${service.address().port}`;
            await register('execution');
            env.SOUNDROBE_AUTO_TAG_EVAL_MOCK_URL = base;
            try {
              const final = await invoke('cold', false);
              await project(final, preparationFile);
              await require('./mock-authority.cjs').rpc(authority.socket, session.client, { op: 'close' });
            } finally {
              service.closeAllConnections(); await new Promise(resolve => service.close(resolve));
              fs.writeFileSync(path.join(directory, 'cold-fixture-ingress.json'), service.compactInventory(), { flag: 'wx' });
              await fixtureLease.release();
            }
          });
        } else {
          const final = await invoke('cold', false);
          await project(final, path.join(final.target, 'preparation.json'));
        }
        continue;
      }
      const preparation = await invoke('preparation-disabled', false);
      if (preparation.exitCode !== 0 || preparation.exported.length !== 2 || preparation.inventory.modelRequests.length || preparation.inventory.misses.length) {
        throw Error(`Preparation failed for ${job.name}; inspect retained native log`);
      }
      // Freeze only reached authentic routes, preserving their original body
      // bytes and acquisition receipts. Unreached routes still fail closed.
      const reached = new Set(preparation.inventory.requests.map(r => JSON.stringify([r.provider, r.path, JSON.stringify(r.query)])));
      const allowed = preparation.inventory.records.filter(r => reached.has(JSON.stringify([r.provider, r.path, r.query])));
      const routeKey = r => JSON.stringify([r.provider, r.path, JSON.stringify([...new URLSearchParams(r.query ?? {}).entries()]
        .sort(([a, av], [b, bv]) => a.localeCompare(b) || av.localeCompare(bv)))]);
      const allowedKeys = new Set(allowed.map(r => JSON.stringify([r.provider, r.path, r.query])));
      const finalServiceManifest = path.join(directory, 'final-service.json');
      save(finalServiceManifest, { ...read(serviceManifest), records: records.filter(r => allowedKeys.has(routeKey(r))) });
      service.closeAllConnections(); await new Promise(resolve => service.close(resolve));
      service = createService(finalServiceManifest);
      service.listen(0, '127.0.0.1'); await once(service, 'listening');
      base = `http://127.0.0.1:${service.address().port}`;
      env.SOUNDROBE_AUTO_TAG_EVAL_MOCK_URL = base;
      save(inventoryFile, { records: (await inventoryAt(base)).records });
      const bindings = { corpus: binding(corpus), input: binding(corpus), expectations: binding(expectations),
        ...(matchesUserLabels(job) ? userBindings : {}),
        labels: binding(path.join(root, '.planning/research/bounded-assistance-machine-labels-20261111.json')),
        packet: binding(path.join(root, '.planning/research/bounded-assistance-label-review-20261111.json')),
        source: binding(path.join(root, 'src-tauri/src/commands/auto_tag.rs')), executable: binding(binary),
        fixtures: binding(inventoryFile), scorer: binding(scorer), driver: binding(__filename),
        mockService: binding(finalServiceManifest), safetyInput: binding(safetyInput) };
      for (const file of execFileSync('rg', ['--files', 'src-tauri/src'], { encoding: 'utf8' }).trim().split('\n')) {
        bindings[`source:${file}`] = binding(path.join(root, file));
      }
      for (const track of item.tracks) {
        const file = path.join(sourceRoot, item.sourceRelativeFolder, track.relativePath);
        bindings[`media:${track.relativePath}`] = binding(file);
      }
      const limits = { attempts: 2, retries: 0, repairs: 0, reservedOutputTokens: 2048,
        requestBytes: 65536, responseBytes: 32 * 1024 * 1024, elapsedMs: 240000 };
      const approvedJobs = preparation.exported.map(row => ({ id: `${row.caseId}/${row.profile}/${row.phase}`,
        inputSha256: row.fieldObservations.inputSha256,
        requests: job.role ? [{ role: job.role, bodySha256: sha(Buffer.from(canonical(body))) }] : [] }));
      const transport = { provider: 'openai', model: 'scripted/genre-v1', endpoint: base + '/model', limits,
        hardCostMicrounits: null, hardTotalBilledTokens: null, jobs: approvedJobs };
      const spec = { version: 1, bindings, fields: ['title', 'artists', 'genre'], phases: ['cold', 'warm'],
        sourceRoot: path.relative(root, sourceRoot), caseIds: [job.caseId], profile: job.profile, transport };
      const specFile = path.join(directory, 'projection-spec.json'); save(specFile, spec);
      const contract = path.join(directory, 'scoring-contract.json');
      python([scorer, 'freeze-native', specFile, path.join(preparation.target, 'records.json'), contract]);
      const manifest = path.join(directory, 'bounded-manifest.json');
      save(manifest, { version: 1, mode: 'mock', runId: crypto.randomUUID(), expiresUnixMs: Date.now() + 300000,
        ...transport, bindings: { ...bindings, scoringContract: binding(contract) } });
      env.SOUNDROBE_BOUNDED_EVAL_MANIFEST = manifest;
      env.SOUNDROBE_BOUNDED_EVAL_APPROVED_SHA256 = binding(manifest).sha256;
      env.SOUNDROBE_AUTO_TAG_EVAL_ASSISTANCE = 'scripted';
      const final = await invoke('bounded', true);
      // Failed executions retain raw exports, never a scored projection.
      if (final.exitCode !== 0 || final.signal || final.inventory.misses.length) throw Error('Bounded invocation failed; retained evidence is non-clean');
      const expectedModelWires = !job.role ? 0 : job.unknownUsage || job.cancelAfterGenre ? 1 : 2;
      if (final.inventory.modelRequests.length !== expectedModelWires) throw Error('Declared diagnostic model role was not fully exercised');
      if (job.unknownUsage || job.cancelAfterGenre) {
        if (final.exported[1]?.outcome !== 'admission_denied' || !final.exported.every(row => row.serviceFailureObserved
            && row.payloadUnchanged && Object.entries(row.after).every(([p, hashes]) => hashes.fullSha256 === row.profileBaseline[p].fullSha256))) {
          throw Error('Stopped diagnostic lost membership or changed a protected baseline');
        }
      }
      // Preparation and final execution have separate strict servers/receipts.
      // Every per-job projection stays provisional until selection-status.json
      // references the complete selection, after ALL jobs and gates succeed.
      const scoreFile = path.join(directory, 'score.provisional.json');
      python([scorer, 'project-native', contract, binding(contract).sha256,
        path.join(final.target, 'records.json'), manifest, scoreFile]);
      executions.push({ name: job.name, caseId: job.caseId, profile: job.profile,
        projectionStatus: 'provisional', selectionStatusFile: path.relative(root, path.join(output, 'selection-status.json')),
        exitCode: final.exitCode, signal: final.signal, requestedPhysicalOccurrences: item.tracks.length * 2,
        preparationProviderRequests: preparation.inventory.requests.length,
        providerRequests: final.inventory.requests.length,
        wireArrivals: final.inventory.wireArrivals.length,
        deniedTunnels: final.inventory.wireArrivals.filter(r => r.method === 'CONNECT').length,
        modelRequests: final.inventory.modelRequests.length, misses: final.inventory.misses.length,
        limits, score: read(scoreFile) });
      save(path.join(directory, 'result.provisional.json'), executions.at(-1));
      console.log(JSON.stringify({ name: job.name, projectionStatus: 'provisional', exitCode: final.exitCode, score: read(scoreFile).outcomes,
        modelRequests: final.inventory.modelRequests.length, misses: final.inventory.misses.length }));
    } finally {
      service.closeAllConnections(); await new Promise(resolve => service.close(resolve));
      if (authority) {
        fs.writeFileSync(path.join(directory, 'preparation-fixture-ingress.json'), service.compactInventory(), { flag: 'wx' });
        await fixtureLease.release();
      }
    }
  }
  if (authority) {
    for (const subject of ['crowd', 'billie', 'enya']) {
      if (preparations.find(p => p.job === `disabled-${subject}`).input !== preparations.find(p => p.job === `assistance-${subject}`).input) throw Error('Paired pre-freeze baseline mismatch');
    }
    const frozen = await authority.command({ op: 'freeze' });
    save(path.join(output, 'frozen-checkpoint.json'), await authority.command({ op: 'report' }));
    await authority.command({ op: 'consume', fake_checkpoint_sha256: frozen.checkpoint });
    for (const execute of pending) await execute();
    const report = await authority.command({ op: 'report' });
    for (const job of jobs) {
      const session = Object.values(report.sessions).find(s => s.stage === 'execution' && s.job === job.name);
      if (!session || session.observation_sha256 !== binding(path.join(output, job.name, 'cold', 'cold.jsonl')).sha256) {
        throw Error('Durable observation receipt differs from consumed export');
      }
    }
    const inventories = [...preparations.map(p => p.inventory), ...jobs.map(j => read(path.join(output, j.name, 'cold', 'inventory-final.json')))];
    const wires = inventories.flatMap(i => i.wireArrivals), ids = wires.map(w => w.operation);
    if (new Set(ids).size !== ids.length || ids.some(id => !report.operations[id]?.arrived)
        || ids.length !== report.counters.arrived || report.counters.reserved !== report.counters.completed
        || report.counters.inventory !== wires.filter(w => w.route === '/__fixtures/compact').length
        || report.counters.model > 1 || report.counters.provider !== inventories.reduce((n,i) => n+i.requests.length,0)
        || report.counters.model !== inventories.reduce((n,i) => n+i.modelRequests.length,0)) throw Error('Durable wire reconciliation failed');
    save(path.join(output, 'wire-reconciliation.json'), { counts: report.counters, actualWireArrivals: wires.length, uniqueOperations: new Set(ids).size });
    await authority.command({ op: 'publish' });
    save(path.join(output, 'authority-receipt.json'), await authority.settle(false, report));
    activeAuthority = null;
  }
  const requested = read(path.join(output, 'requested-membership.json'));
  for (const [key, actual] of Object.entries({
    requestedPhysicalOccurrences: executions.reduce((n, j) => n + j.requestedPhysicalOccurrences, 0),
    requestedFieldSlots: executions.reduce((n, j) => n + j.score.requestedFieldSlots, 0),
    predeclaredLabelableSlots: executions.reduce((n, j) => n + j.score.predeclaredLabelableSlots, 0),
  })) if (actual !== requested[key]) throw Error(`Projected selection denominator differs: ${key}`);
  save(path.join(output, 'results.json'), { mode: 'mock_only', executions,
    selectionStatusFile: path.relative(root, path.join(output, 'selection-status.json')),
    uniqueSelectedOccurrenceFieldLabels: requested.uniqueSelectedOccurrenceFieldLabels,
    uniqueUserAcceptanceLabels: requested.uniqueUserAcceptanceLabels,
    userAcceptance: Object.fromEntries(['requestedFieldSlots', 'observedFieldSlots', 'agreeingFieldSlots', 'serviceFailureSlots']
      .map(key => [key, executions.reduce((n, j) => n + j.score.userAcceptance[key], 0)])),
    requestedPhysicalOccurrences: executions.reduce((n, j) => n + j.requestedPhysicalOccurrences, 0),
    requestedFieldSlots: executions.reduce((n, j) => n + j.score.requestedFieldSlots, 0),
    predeclaredLabelableSlots: executions.reduce((n, j) => n + j.score.predeclaredLabelableSlots, 0),
    groundedAssistanceAccuracyCredit: 0, acceptedCertificate: false, writableDelta: [] });
  save(path.join(output, 'selection-status.json'), { executionStatus: 'complete', scoringStatus: 'insufficient',
    resultsFile: path.relative(root, path.join(output, 'results.json')) });
}
let activeAuthority;
if (require.main === module) main(...process.argv.slice(2)).catch(async error => {
  if (activeAuthority) {
    try { save(path.join(admittedOutput, 'authority-incomplete.json'), await activeAuthority.settle(true)); }
    catch (failure) { console.error(String(failure)); }
    activeAuthority = null;
  }
  if (admittedOutput && fs.existsSync(path.join(admittedOutput, 'requested-membership.json'))) {
    if (read(path.join(admittedOutput, 'selection.json')).version === 2) {
      save(path.join(admittedOutput, 'failure-envelope.json'), failureEnvelope(admittedOutput));
      save(path.join(admittedOutput, 'selection-status.json'), { executionStatus: 'incomplete', scoringStatus: 'partial_or_unavailable',
        failureEnvelope: path.relative(root, path.join(admittedOutput, 'failure-envelope.json')) });
      console.error(error); process.exitCode = 1; return;
    }
    const requested = read(path.join(admittedOutput, 'requested-membership.json'));
    save(path.join(admittedOutput, 'failure-envelope.json'), { ...requested,
      outcome: 'incomplete', error: String(error), projectedFieldSlots: 0,
      evaluations: requested.evaluations.map(e => {
        const file = path.join(admittedOutput, e.name, 'bounded', `${e.phase}.jsonl`);
        let rows = [], exportError = null;
        try { if (fs.existsSync(file)) rows = fs.readFileSync(file, 'utf8').trim().split('\n').filter(Boolean).map(JSON.parse); }
        catch (parseError) { exportError = String(parseError); }
        return { ...e, nativeObservation: rows.length === 1 && !exportError ? 'exported_not_validated' : 'unavailable',
          exportError, nativeExport: fs.existsSync(file) ? path.relative(root, file) : null };
      }), groundedAssistanceAccuracyCredit: 0, acceptedCertificate: false, writableDelta: [] });
    save(path.join(admittedOutput, 'selection-status.json'), { executionStatus: 'incomplete', scoringStatus: 'unavailable',
      failureEnvelope: path.relative(root, path.join(admittedOutput, 'failure-envelope.json')) });
  }
  console.error(error); process.exitCode = 1;
});
