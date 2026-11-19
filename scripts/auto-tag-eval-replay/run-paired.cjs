// Offline native baseline/candidate loop. Run from the repository root.
// freeze NEW_BUILD; compare NEW_RUN FROZEN_BASELINE FROZEN_CANDIDATE [SELECTION]
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const { spawn, spawnSync, execFileSync } = require('node:child_process');
const { once } = require('node:events');
const { createService, importPools } = require('../mock-provider-service.cjs');
const { assertProviderSourceCompatible, pairedBudget, pairedNativeInvocation,
  pairedRunContext, approvedManifestBodyPath, rebaseManifestBodyFiles,
  pairedGenreServiceManifest, parseCompareOptions } = require('./paired-policy.cjs');
const root = process.cwd();
const fixtures = path.join(root, 'test/fixtures/tauri/auto-tag-eval');
const providerFixtureRoot = path.dirname(fixtures);
const profiles = ['folder_filename', 'assisted_without_ids', 'tagged_recovery'];
const read = file => JSON.parse(fs.readFileSync(file));
const hash = file => crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const save = (file, value) => fs.writeFileSync(file, JSON.stringify(value, null, 2) + '\n', { flag: 'wx' });
const toolEnv = () => Object.fromEntries(['PATH', 'HOME', 'TMPDIR'].filter(k => process.env[k]).map(k => [k, process.env[k]]));
const exec = (tool, args, options = {}) => execFileSync(tool, args, { cwd: root, env: toolEnv(), encoding: 'utf8', ...options });
const auditFile = path.join(__dirname, 'audit-paired.py');
const sourceFiles = () => exec('git', ['ls-files', '--cached', '--others', '--exclude-standard', '--', 'src-tauri/src',
  'src-tauri/Cargo.toml', 'src-tauri/Cargo.lock', 'src-tauri/build.rs', 'data']).trim().split('\n');
const locksFor = files => Object.fromEntries(files.map(file => [file, hash(file)]));
function verify(locks) {
  for (const [file, expected] of Object.entries(locks)) if (hash(file) !== expected) throw Error(`Changed input: ${file}`);
}

async function freeze(output) {
  fs.mkdirSync(output);
  const files = sourceFiles();
  const sourceLocks = locksFor(files);
  const gitHead = exec('git', ['rev-parse', 'HEAD']).trim();
  const diff = exec('git', ['diff', 'HEAD', '--', 'src-tauri', 'data']);
  fs.writeFileSync(path.join(output, 'source.diff'), diff, { flag: 'wx' });
  exec('tar', ['-czf', path.join(output, 'source.tgz'), ...files]);
  const log = fs.openSync(path.join(output, 'build.jsonl'), 'wx');
  const errors = fs.openSync(path.join(output, 'build.log'), 'wx');
  let result;
  try {
    const child = spawn('cargo', ['test', '--offline', '--manifest-path', 'src-tauri/Cargo.toml',
      '--no-default-features', '--features', 'server', '--lib', '--no-run', '--message-format=json'],
    { cwd: root, env: toolEnv(), stdio: ['ignore', log, errors] });
    result = await once(child, 'exit');
  } finally { fs.closeSync(log); fs.closeSync(errors); }
  if (result[0] !== 0) throw Error(`Offline build failed; see ${output}/build.log`);
  verify(sourceLocks);
  const builds = fs.readFileSync(path.join(output, 'build.jsonl'), 'utf8').trim().split('\n').map(JSON.parse);
  const built = builds.find(r => r.reason === 'compiler-artifact' && r.target.name === 'soundrobe_lib' && r.executable)?.executable;
  if (!built) throw Error('Native soundrobe_lib test executable missing');
  const binary = path.join(output, 'native-evaluator');
  fs.copyFileSync(built, binary, fs.constants.COPYFILE_EXCL);
  save(path.join(output, 'frozen.json'), { schemaVersion: 1, gitHead, dirtySource: !!diff || files.some(file =>
    spawnSync('git', ['ls-files', '--error-unmatch', file], { cwd: root, stdio: 'ignore' }).status !== 0),
  sourceLocks, sourceArchiveSha256: hash(path.join(output, 'source.tgz')), binarySha256: hash(binary),
  rust: exec('rustc', ['--version']).trim(), node: process.version });
  console.log(`Frozen ${gitHead}: ${output}`);
}

async function runJob(output, arm, job, binary, providerManifest, budget) {
  const { directory, runId, artifactDirectory } = pairedRunContext(output, arm, job);
  const executionContext = { runId, artifactDirectory, caseId: job.caseId, profile: job.profile,
    scenario: job.scenario ?? null };
  fs.mkdirSync(directory, { recursive: true });
  const log = fs.openSync(path.join(directory, 'native.log'), 'wx');
  const server = createService(job.serviceManifest ?? providerManifest);
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  const base = `http://127.0.0.1:${server.address().port}`;
  let timer;
  let execution;
  try {
    const env = { ...toolEnv(), SOUNDROBE_AUTO_TAG_EVAL_CORPUS: job.corpusFile,
      SOUNDROBE_AUTO_TAG_EVAL_EXPECTATIONS: job.expectations, SOUNDROBE_AUTO_TAG_EVAL_SOURCE_ROOT: job.sourceRoot,
      SOUNDROBE_AUTO_TAG_EVAL_ARTIFACT_DIR: directory, SOUNDROBE_AUTO_TAG_EVAL_RUN_ID: runId,
      SOUNDROBE_AUTO_TAG_EVAL_PROFILE: job.profile, SOUNDROBE_AUTO_TAG_EVAL_CASES: job.caseId,
      SOUNDROBE_AUTO_TAG_EVAL_MOCK_URL: base, SOUNDROBE_AUTO_TAG_EVAL_ASSISTANCE: budget.assistance };
    if (job.scenario) Object.assign(env, { SOUNDROBE_AUTO_TAG_EVAL_SCENARIO: job.scenario });
    const invocation = budget.assistance === 'scripted'
      ? pairedNativeInvocation(arm)
      : { args: ['--exact', 'commands::auto_tag::eval_tests::live_auto_tag_eval', '--ignored', '--nocapture'], env: {} };
    Object.assign(env, invocation.env);
    const child = spawn(binary, invocation.args,
      { cwd: root, env, stdio: ['ignore', log, log] });
    let deadline = false;
    timer = setTimeout(() => { deadline = true; child.kill('SIGKILL'); }, budget.processSeconds * 1000);
    const [exitCode, signal] = await once(child, 'exit');
    execution = { ...executionContext, exitCode, signal, deadline };
  } catch (error) {
    execution = { ...executionContext, exitCode: null, signal: null, error: error.message };
  } finally {
    clearTimeout(timer);
    fs.closeSync(log);
    try {
      const inventory = await (await fetch(`${base}/__fixtures?compact=1`)).json();
      save(path.join(directory, 'inventory.json'), inventory);
    } finally {
      server.closeAllConnections();
      await new Promise(resolve => server.close(resolve));
    }
  }
  save(path.join(directory, 'execution.json'), execution);
  console.log(JSON.stringify({ arm, ...execution }));
}

async function compare(output, baseline, candidate, selectionFile, assistance) {
  const selection = read(selectionFile);
  if (selection.schemaVersion !== 1 || !selection.cases.length || new Set(selection.cases.map(c => c.caseId)).size !== selection.cases.length
      || selection.cases.some(c => !/^[a-f0-9]{12}$/.test(c.caseId) || !/^[a-z-]+$/.test(c.population)
        || path.basename(c.manifest) !== c.manifest)) throw Error('Explicit unique eligible case selection required');
  fs.mkdirSync(output);
  const sourceFixture = assistance === 'scripted'
    ? read(path.join(__dirname, 'synthetic-genre-source.json')) : null;
  const jobs = profiles.flatMap(profile => selection.cases.map(c => ({ ...c, profile })));
  if (assistance === 'scripted') {
    const required = read(path.join(__dirname, 'paired-safety-selection.json')).cases.map(c => c.caseId).sort();
    const chosen = selection.cases.map(c => c.caseId).sort();
    if (JSON.stringify(chosen) !== JSON.stringify(required)) {
      throw Error('Scripted assistance requires the complete five-case safety selection');
    }
    const assisted = selection.cases.find(c => c.caseId === sourceFixture.caseId);
    if (!assisted) throw Error('Synthetic failure control requires the source-bound assisted case');
    jobs.push({ ...assisted, profile: sourceFixture.profile, scenario: 'assistance_failure', supplemental: true });
  }
  const snapshot = { schemaVersion: 1, profiles, jobs,
    implementations: {}, locks: {}, setupFailures: [],
    budget: { ...pairedBudget(assistance, jobs.length), processSeconds: 1300, nativeFolderSeconds: 600,
      cache: 'fresh per implementation/case/profile; warm reuses only its cold cache, with fresh media',
      provider: 'unchanged production policy; equal source-locked provider client and discovery limits' } };
  const lock = file => { snapshot.locks[path.resolve(file)] = hash(file); };
  try {
    lock(selectionFile);
    const harness = ['run-paired.cjs', 'paired-policy.cjs', 'audit-paired.py', 'score-best-aligned.py',
      'bounded-storage.cjs', 'synthetic-genre-source.json']
      .map(file => path.relative(root, path.join(__dirname, file)))
      .concat(['scripts/mock-provider-service.cjs', 'scripts/review_auto_tag_subset.py']);
    for (const file of harness) lock(file);
    const archive = path.join(output, 'harness.tgz');
    exec('tar', ['-czf', archive, ...harness]);
    lock(archive);
    for (const [arm, source] of Object.entries({ baseline, candidate })) {
      const frozen = read(path.join(source, 'frozen.json'));
      const binary = path.join(source, 'native-evaluator');
      if (hash(binary) !== frozen.binarySha256 || hash(path.join(source, 'source.tgz')) !== frozen.sourceArchiveSha256) throw Error(`Changed frozen implementation: ${source}`);
      const destination = path.join(output, `${arm}-evaluator`);
      fs.copyFileSync(binary, destination, fs.constants.COPYFILE_EXCL);
      lock(destination); lock(path.join(source, 'frozen.json')); lock(path.join(source, 'source.tgz'));
      snapshot.implementations[arm] = { ...frozen, frozenDirectory: source, executable: destination };
      const evaluatorSource = exec('tar', ['-xOzf', path.join(source, 'source.tgz'),
        'src-tauri/src/commands/auto_tag_eval.rs']);
      snapshot.implementations[arm].pairedReviewExport = evaluatorSource.includes(
        'SOUNDROBE_AUTO_TAG_EVAL_PAIRED_ASSISTED_REVIEW');
    }
    const providerFile = 'src-tauri/src/state/providers.rs';
    if (!snapshot.implementations.baseline.sourceLocks[providerFile]
        || !snapshot.implementations.candidate.sourceLocks[providerFile]) throw Error(`Missing provider policy lock: ${providerFile}`);
    const providerSource = implementation => exec('tar', ['-xOzf',
      path.join(implementation.frozenDirectory, 'source.tgz'), providerFile]);
    assertProviderSourceCompatible(providerSource(snapshot.implementations.baseline),
      providerSource(snapshot.implementations.candidate));
    const albumSearchFile = 'src-tauri/src/commands/album_search.rs';
    if (snapshot.implementations.baseline.sourceLocks[albumSearchFile]
        !== snapshot.implementations.candidate.sourceLocks[albumSearchFile]) {
      throw Error(`Provider budget policy differs: ${albumSearchFile}`);
    }
    for (const tool of ['python3', 'ffmpeg']) exec(tool, [tool === 'python3' ? '--version' : '-version']);
    const suite = read(path.join(fixtures, 'batch-suite.json'));
    for (const input of suite.inputs) {
      const file = path.join(root, 'test/fixtures/tauri', input.path);
      if (hash(file) !== input.sha256) throw Error(`Changed suite input: ${file}`);
      lock(file);
    }
    const prepared = new Map();
    for (const selected of selection.cases) {
      if (!suite.cases.some(c => c.caseId === selected.caseId && c.scored)) throw Error(`Unreviewed case: ${selected.caseId}`);
      const dummy = path.join(root, '.amp/in/eval-dummies', selected.population);
      const corpusFile = path.join(dummy, 'corpus.json');
      const caseData = read(corpusFile).cases.find(c => c.caseId === selected.caseId);
      if (!caseData) throw Error(`Missing dummy case: ${selected.caseId} in ${corpusFile}`);
      lock(corpusFile);
      for (const track of caseData.tracks) {
        const file = path.resolve(dummy, 'media', caseData.sourceRelativeFolder, track.relativePath);
        if (!file.startsWith(path.join(dummy, 'media') + path.sep) || hash(file) !== track.sourceSha256) throw Error(`Changed or escaped dummy: ${file}`);
        lock(file);
      }
      const review = read(path.join(fixtures, selected.manifest)).cases.find(c => c.caseId === selected.caseId && c.status === 'verified_match');
      if (!review) throw Error(`Missing reviewed match: ${selected.caseId}`);
      const reviewDir = path.join(output, `review-${selected.caseId}`);
      exec('python3', ['-B', 'scripts/review_auto_tag_subset.py', '--corpus', corpusFile,
        '--manifest', path.join(fixtures, selected.manifest), '--fixture-root', fixtures, '--output-dir', reviewDir]);
      const expectations = path.join(reviewDir, 'expectations.json');
      lock(expectations);
      const truth = read(expectations).cases.find(c => c.caseId === selected.caseId);
      const fieldInput = path.join(reviewDir, 'field-input.json');
      save(fieldInput, { review, truth });
      const expected = JSON.parse(exec('python3', ['-B', auditFile, 'fields', fieldInput, fixtures]));
      prepared.set(selected.caseId, { case: caseData, truth, review, expected, corpusFile, expectations, sourceRoot: path.join(dummy, 'media') });
    }
    const manifest = path.join(output, 'providers.json');
    importPools(path.join(fixtures, 'candidate-pools.json'), manifest);
    lock(manifest);
    const baseManifest = read(manifest);
    for (const record of baseManifest.records) if (record.bodyFile) {
      lock(approvedManifestBodyPath(manifest, record.bodyFile, providerFixtureRoot));
    }
    const serviceManifests = new Map();
    if (assistance === 'scripted') {
      const sourceFixturePath = path.join(__dirname, 'synthetic-genre-source.json');
      lock(sourceFixturePath);
      snapshot.syntheticGenreSource = sourceFixture;
      const fixtureDirectory = path.join(output, 'job-fixtures');
      fs.mkdirSync(fixtureDirectory);
      for (const job of jobs) {
        const suffix = job.scenario ? `-${job.scenario}` : '';
        const serviceManifest = path.join(fixtureDirectory, `${job.caseId}-${job.profile}${suffix}.json`);
        const jobManifest = rebaseManifestBodyFiles(
          pairedGenreServiceManifest(baseManifest, sourceFixture, job,
            bodyFile => read(approvedManifestBodyPath(manifest, bodyFile, providerFixtureRoot))),
          manifest, serviceManifest, providerFixtureRoot);
        save(serviceManifest, jobManifest);
        lock(serviceManifest);
        for (const record of jobManifest.records) if (record.bodyFile) {
          lock(approvedManifestBodyPath(serviceManifest, record.bodyFile, providerFixtureRoot));
        }
        serviceManifests.set(`${job.caseId}/${job.profile}/${job.scenario ?? 'primary'}`, serviceManifest);
      }
    }
    snapshot.jobs = jobs.map(job => ({ ...job, ...prepared.get(job.caseId),
      serviceManifest: serviceManifests.get(`${job.caseId}/${job.profile}/${job.scenario ?? 'primary'}`) ?? manifest }));
  } catch (error) { snapshot.setupFailures.push(error.message); }
  save(path.join(output, 'comparison-inputs.json'), snapshot);
  if (!snapshot.setupFailures.length) {
    for (const job of snapshot.jobs) {
      for (const arm of ['baseline', 'candidate']) {
        try {
          verify(snapshot.locks);
          await runJob(output, arm, job, snapshot.implementations[arm].executable,
            job.serviceManifest ?? path.join(output, 'providers.json'), snapshot.budget);
        } catch (error) {
          console.error(`${arm}/${job.caseId}/${job.profile}: ${error.message}`);
          // Missing execution remains unavailable in the full requested report.
        }
      }
    }
  }
  const result = spawnSync('python3', ['-B', auditFile, output], { cwd: root, env: toolEnv(), stdio: 'inherit' });
  if (result.error) throw Error(`Independent auditor unavailable: ${result.error.message}; requested membership retained in comparison-inputs.json`);
  process.exitCode = result.status ?? 1;
}

const [operation, outputArg, baselineArg, candidateArg, ...compareArgs] = process.argv.slice(2);
let compareOptions = { selection: null, assistance: 'disabled' };
try { compareOptions = parseCompareOptions(compareArgs); }
catch (error) { console.error(error.message); process.exitCode = 1; }
if (!outputArg || !['freeze', 'compare'].includes(operation) || (operation === 'compare' && (!baselineArg || !candidateArg))) {
  console.error('Usage: node scripts/auto-tag-eval-replay/run-paired.cjs freeze NEW_BUILD\n       node scripts/auto-tag-eval-replay/run-paired.cjs compare NEW_RUN FROZEN_BASELINE FROZEN_CANDIDATE [SELECTION] [--scripted-genre-assistance]');
  process.exitCode = 1;
} else if (!process.exitCode) {
  (operation === 'freeze' ? freeze(path.resolve(outputArg))
    : compare(path.resolve(outputArg), path.resolve(baselineArg), path.resolve(candidateArg),
      path.resolve(compareOptions.selection ?? path.join(__dirname,
        compareOptions.assistance === 'scripted' ? 'paired-safety-selection.json' : 'paired-selection.json')),
      compareOptions.assistance))
    .catch(error => { console.error(error.message); process.exitCode = 1; });
}
