// Offline native baseline/candidate loop. Run from the repository root.
// freeze NEW_BUILD; compare NEW_RUN FROZEN_BASELINE FROZEN_CANDIDATE [SELECTION]
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const { spawn, spawnSync, execFileSync } = require('node:child_process');
const { once } = require('node:events');
const { createService, importPools } = require('../mock-provider-service.cjs');
const root = process.cwd();
const fixtures = path.join(root, 'test/fixtures/tauri/auto-tag-eval');
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
  const directory = path.join(output, arm, job.profile, job.caseId);
  fs.mkdirSync(directory, { recursive: true });
  const log = fs.openSync(path.join(directory, 'native.log'), 'wx');
  const server = createService(providerManifest);
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  const base = `http://127.0.0.1:${server.address().port}`;
  let timer;
  let execution;
  try {
    const env = { ...toolEnv(), SOUNDROBE_AUTO_TAG_EVAL_CORPUS: job.corpusFile,
      SOUNDROBE_AUTO_TAG_EVAL_EXPECTATIONS: job.expectations, SOUNDROBE_AUTO_TAG_EVAL_SOURCE_ROOT: job.sourceRoot,
      SOUNDROBE_AUTO_TAG_EVAL_ARTIFACT_DIR: directory, SOUNDROBE_AUTO_TAG_EVAL_RUN_ID: `${arm}-${job.profile}-${job.caseId}`,
      SOUNDROBE_AUTO_TAG_EVAL_PROFILE: job.profile, SOUNDROBE_AUTO_TAG_EVAL_CASES: job.caseId,
      SOUNDROBE_AUTO_TAG_EVAL_MOCK_URL: base, SOUNDROBE_AUTO_TAG_EVAL_ASSISTANCE: 'disabled' };
    const child = spawn(binary, ['--exact', 'commands::auto_tag::eval_tests::live_auto_tag_eval', '--ignored', '--nocapture'],
      { cwd: root, env, stdio: ['ignore', log, log] });
    let deadline = false;
    timer = setTimeout(() => { deadline = true; child.kill('SIGKILL'); }, budget.processSeconds * 1000);
    const [exitCode, signal] = await once(child, 'exit');
    execution = { exitCode, signal, deadline, caseId: job.caseId, profile: job.profile };
  } catch (error) {
    execution = { exitCode: null, signal: null, error: error.message, caseId: job.caseId, profile: job.profile };
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

async function compare(output, baseline, candidate, selectionFile) {
  const selection = read(selectionFile);
  if (selection.schemaVersion !== 1 || !selection.cases.length || new Set(selection.cases.map(c => c.caseId)).size !== selection.cases.length
      || selection.cases.some(c => !/^[a-f0-9]{12}$/.test(c.caseId) || !/^[a-z-]+$/.test(c.population)
        || path.basename(c.manifest) !== c.manifest)) throw Error('Explicit unique eligible case selection required');
  fs.mkdirSync(output);
  const snapshot = { schemaVersion: 1, profiles, jobs: profiles.flatMap(profile => selection.cases.map(c => ({ ...c, profile }))),
    implementations: {}, locks: {}, setupFailures: [],
    budget: { modelRequests: 0, processSeconds: 1300, nativeFolderSeconds: 600,
      cache: 'fresh per implementation/case/profile; warm reuses only its cold cache, with fresh media',
      provider: 'unchanged production policy; equal source-locked provider client and discovery limits' } };
  const lock = file => { snapshot.locks[path.resolve(file)] = hash(file); };
  try {
    lock(selectionFile);
    const harness = ['run-paired.cjs', 'audit-paired.py', 'score-best-aligned.py', 'bounded-storage.cjs']
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
    }
    for (const file of ['src-tauri/src/state/providers.rs', 'src-tauri/src/commands/album_search.rs']) {
      if (snapshot.implementations.baseline.sourceLocks[file] !== snapshot.implementations.candidate.sourceLocks[file]) throw Error(`Provider budget policy differs: ${file}`);
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
    snapshot.jobs = snapshot.jobs.map(job => ({ ...job, ...prepared.get(job.caseId) }));
    const manifest = path.join(output, 'providers.json');
    importPools(path.join(fixtures, 'candidate-pools.json'), manifest);
    lock(manifest);
    for (const record of read(manifest).records) if (record.bodyFile) lock(path.resolve(output, record.bodyFile));
  } catch (error) { snapshot.setupFailures.push(error.message); }
  save(path.join(output, 'comparison-inputs.json'), snapshot);
  if (!snapshot.setupFailures.length) {
    for (const job of snapshot.jobs) {
      for (const arm of ['baseline', 'candidate']) {
        try {
          verify(snapshot.locks);
          await runJob(output, arm, job, snapshot.implementations[arm].executable, path.join(output, 'providers.json'), snapshot.budget);
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

const [operation, outputArg, baselineArg, candidateArg, selectionArg] = process.argv.slice(2);
if (!outputArg || !['freeze', 'compare'].includes(operation) || (operation === 'compare' && (!baselineArg || !candidateArg))) {
  console.error('Usage: node scripts/auto-tag-eval-replay/run-paired.cjs freeze NEW_BUILD\n       node scripts/auto-tag-eval-replay/run-paired.cjs compare NEW_RUN FROZEN_BASELINE FROZEN_CANDIDATE [SELECTION]');
  process.exitCode = 1;
} else {
  (operation === 'freeze' ? freeze(path.resolve(outputArg))
    : compare(path.resolve(outputArg), path.resolve(baselineArg), path.resolve(candidateArg),
      path.resolve(selectionArg ?? path.join(__dirname, 'paired-selection.json'))))
    .catch(error => { console.error(error.message); process.exitCode = 1; });
}
