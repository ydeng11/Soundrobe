// Bounded scripted-assistance diagnostic, not a live model accuracy benchmark.
// Usage: node scripts/auto-tag-eval-replay/run-assisted.cjs NEW_OUTPUT BUILD_JSONL
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const { spawn, execFileSync } = require('node:child_process');
const { once } = require('node:events');
const { createService, importPools } = require('../mock-provider-service.cjs');
const root = process.cwd();
const [output, buildFile] = process.argv.slice(2).map(file => path.resolve(file));
const read = file => JSON.parse(fs.readFileSync(file));
const hash = file => crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const save = (file, value) => fs.writeFileSync(file, JSON.stringify(value, null, 2) + '\n', { flag: 'wx' });
fs.mkdirSync(output); // Never append to historical runs.
const fixtures = path.join(root, 'test/fixtures/tauri/auto-tag-eval');
const dummy = path.join(root, '.amp/in/eval-dummies/original');
const corpusFile = path.join(dummy, 'corpus.json');
const caseId = 'fb0a478438a7';
const item = read(corpusFile).cases.find(item => item.caseId === caseId);
const builds = fs.readFileSync(buildFile, 'utf8').trim().split('\n').map(JSON.parse);
const built = builds.find(item => item.reason === 'compiler-artifact' && item.target.name === 'soundrobe_lib' && item.executable)?.executable;
if (!built) throw Error('Native test executable missing');
const binary = path.join(output, 'native-evaluator');
fs.copyFileSync(built, binary, fs.constants.COPYFILE_EXCL);
const locks = Object.fromEntries(read(path.join(fixtures, 'batch-suite.json')).inputs.map(item =>
  [path.join('test/fixtures/tauri', item.path), item.sha256]));
for (const file of execFileSync('rg', ['--files', 'src-tauri/src'], { encoding: 'utf8' }).trim().split('\n')
  .concat(['src-tauri/Cargo.toml', 'src-tauri/Cargo.lock', 'data/provider-title-aliases.json',
    'scripts/mock-provider-service.cjs', 'scripts/auto-tag-eval-replay/run-assisted.cjs',
    'scripts/auto-tag-eval-replay/audit-assisted.py', path.relative(root, corpusFile)])) locks[file] = hash(file);
for (const track of item.tracks) {
  const file = path.join(dummy, 'media', item.sourceRelativeFolder, track.relativePath);
  if (hash(file) !== track.sourceSha256) throw Error('Changed dummy input');
  locks[path.relative(root, file)] = track.sourceSha256;
}
const reviews = path.join(output, 'review');
execFileSync('python3', ['-B', 'scripts/review_auto_tag_subset.py', '--corpus', corpusFile,
  '--manifest', path.join(fixtures, 'reviewed-billie-20261108.json'), '--fixture-root', fixtures,
  '--output-dir', reviews], { stdio: 'pipe' });
const manifestFile = path.join(output, 'providers.json');
importPools(path.join(fixtures, 'candidate-pools.json'), manifestFile);
const providerManifest = read(manifestFile);
const profiles = ['folder_filename', 'assisted_without_ids', 'tagged_recovery'];
const scenarios = ['accepted', 'rejected', 'failed', 'disabled'];
save(path.join(output, 'snapshot.json'), { schemaVersion: 1, caseId, profiles, scenarios,
  requestedPhysicalFiles: item.tracks.length * profiles.length * scenarios.length * 2,
  modelEvidence: 'scripted, not observed model quality; genre is not independently verified',
  budget: { modelRequestsPerProcess: 2, providerBudgets: 'unchanged production coordinator' },
  gitHead: execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim(),
  binarySha256: hash(binary), locks });
fs.writeFileSync(path.join(output, 'source.diff'), execFileSync('git', ['diff', '--', 'src-tauri', 'scripts', 'src/components']));
async function run() {
  const executions = [];
  const jobs = scenarios.flatMap(scenario => profiles.map(profile => ({ scenario, profile })));
  async function execute({ scenario, profile }) {
    const directory = path.join(output, `${scenario}-${profile}`);
    fs.mkdirSync(directory);
    const manifest = path.join(directory, 'providers.json');
    // Imported paths are relative to the parent output directory.
    save(manifest, { ...providerManifest,
      records: providerManifest.records.map(record => ({ ...record,
        bodyFile: path.relative(directory, path.resolve(output, record.bodyFile)) })),
      ...(scenario === 'disabled' ? {} : { scriptedModel: { maxRequests: 2,
        payload: { artist: 'Billie Eilish', album: 'Ocean Eyes',
          tracks: [profile === 'tagged_recovery' ? 'Ocean Eyes' : 'ocean eyes'] },
        response: { genre: 'Pop', confidence: scenario === 'rejected' ? 0.2 : 0.8 },
        status: scenario === 'failed' ? 400 : 200 } }) });
    const server = createService(manifest);
    server.listen(0, '127.0.0.1');
    await once(server, 'listening');
    const base = `http://127.0.0.1:${server.address().port}`;
    const log = fs.openSync(path.join(directory, 'native.log'), 'wx');
    try {
      // Do not inherit keys, proxy settings, or accidental evaluation overrides.
      const env = Object.fromEntries(['PATH', 'HOME', 'TMPDIR'].filter(key => process.env[key]).map(key => [key, process.env[key]]));
      Object.assign(env, { SOUNDROBE_AUTO_TAG_EVAL_CORPUS: corpusFile,
        SOUNDROBE_AUTO_TAG_EVAL_EXPECTATIONS: path.join(reviews, 'expectations.json'),
        SOUNDROBE_AUTO_TAG_EVAL_SOURCE_ROOT: path.join(dummy, 'media'),
        SOUNDROBE_AUTO_TAG_EVAL_ARTIFACT_DIR: directory, SOUNDROBE_AUTO_TAG_EVAL_RUN_ID: `${scenario}-${profile}`,
        SOUNDROBE_AUTO_TAG_EVAL_PROFILE: profile, SOUNDROBE_AUTO_TAG_EVAL_CASES: caseId,
        SOUNDROBE_AUTO_TAG_EVAL_MOCK_URL: base,
        SOUNDROBE_AUTO_TAG_EVAL_ASSISTANCE: scenario === 'disabled' ? 'disabled' : 'scripted' });
      const child = spawn(binary, ['--exact', 'commands::auto_tag::eval_tests::live_auto_tag_eval', '--ignored', '--nocapture'],
        { cwd: root, env, stdio: ['ignore', log, log] });
      const [exitCode, signal] = await once(child, 'exit');
      const inventory = await (await fetch(`${base}/__fixtures`)).json();
      save(path.join(directory, 'inventory.json'), inventory);
      executions.push({ scenario, profile, exitCode, signal, missingRoutes: inventory.misses.length });
      console.log(scenario, profile, exitCode, inventory.modelRequests.length, 'model requests');
    } finally {
      fs.closeSync(log);
      server.closeAllConnections();
      await new Promise(resolve => server.close(resolve));
    }
  }
  async function worker() { while (jobs.length) await execute(jobs.shift()); }
  await Promise.all(Array.from({ length: 3 }, worker));
  save(path.join(output, 'executions.json'), executions);
  for (const [file, expected] of Object.entries(locks)) if (hash(file) !== expected) throw Error(`Changed input: ${file}`);
  if (executions.some(item => item.exitCode !== 0 || item.missingRoutes)) process.exitCode = 1;
}
run().catch(error => { console.error(error); process.exitCode = 1; });
