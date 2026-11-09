// Runs the existing native evaluator. No upstream requests or original media.
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const { spawn, execFileSync } = require('node:child_process');
const { once } = require('node:events');
const { createService, importPools } = require('../../../../scripts/mock-provider-service.cjs');
const root = process.cwd();
const here = __dirname;
const fixtures = path.join(root, 'test/fixtures/tauri/auto-tag-eval');
const read = file => JSON.parse(fs.readFileSync(file));
const hash = file => crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const save = (file, value) => fs.writeFileSync(file, JSON.stringify(value, null, 2) + '\n', { flag: 'wx' });
const groups = [
  ['original', 'reviewed-subset.json'],
  ['original', 'reviewed-billie-20261108.json'],
  ['artist-batch-extension', 'reviewed-cjk-artist-20261108.json'],
  ['library-sample', 'reviewed-cjk-library-20261108.json'],
  ['original', 'reviewed-expansion-2-20261108.json'],
  ['artist-batch-extension', 'reviewed-cjk-artist-2-20261108.json'],
  ['library-sample', 'reviewed-library-2-20261108.json'],
  ['curated-hard-cases', 'reviewed-curated-2-20261108.json'],
];
const profiles = ['folder_filename', 'assisted_without_ids', 'tagged_recovery'];
const selected = [];
const locks = Object.fromEntries(read(path.join(fixtures, 'batch-suite.json')).inputs.map(item =>
  [path.join('test/fixtures/tauri', item.path), item.sha256]));
for (const [file, expected] of Object.entries(locks)) if (hash(path.join(root, file)) !== expected) throw Error(`Changed input: ${file}`);
const builds = fs.readFileSync(path.join(here, 'build.jsonl'), 'utf8').trim().split('\n').map(JSON.parse);
const built = builds.find(item => item.reason === 'compiler-artifact' && item.target.name === 'soundrobe_lib' && item.executable)?.executable;
if (!built) throw Error('Native test executable missing');
const binary = path.join(here, 'native-evaluator');
fs.copyFileSync(built, binary, fs.constants.COPYFILE_EXCL);
for (const file of execFileSync('rg', ['--files', 'src-tauri/src'], { encoding: 'utf8' }).trim().split('\n')) locks[file] = hash(file);
locks['src-tauri/Cargo.toml'] = hash('src-tauri/Cargo.toml');
locks['src-tauri/Cargo.lock'] = hash('src-tauri/Cargo.lock');
locks['data/provider-title-aliases.json'] = hash('data/provider-title-aliases.json');
locks['scripts/mock-provider-service.cjs'] = hash('scripts/mock-provider-service.cjs');
locks['scripts/review_auto_tag_subset.py'] = hash('scripts/review_auto_tag_subset.py');
locks[path.relative(root, __filename)] = hash(__filename);
locks[path.relative(root, path.join(here, 'audit.py'))] = hash(path.join(here, 'audit.py'));
locks['.amp/in/artifacts/album-content-comparison-20261107/score-best-aligned.py'] = hash('.amp/in/artifacts/album-content-comparison-20261107/score-best-aligned.py');
const binaryHash = hash(binary);
for (const [population, manifest] of groups) {
  const dummyRoot = path.join(root, '.amp/in/eval-dummies', population);
  const corpusFile = path.join(dummyRoot, 'corpus.json');
  const corpus = read(corpusFile);
  const reviewDirectory = path.join(here, 'reviews', path.basename(manifest, '.json'));
  execFileSync('/usr/bin/python3', ['-B', 'scripts/review_auto_tag_subset.py',
    '--corpus', corpusFile, '--manifest', path.join(fixtures, manifest), '--fixture-root', fixtures,
    '--output-dir', reviewDirectory], { stdio: 'pipe' });
  for (const review of read(path.join(reviewDirectory, 'review.json')).cases.filter(item => item.status === 'verified_match')) {
    const item = corpus.cases.find(item => item.caseId === review.caseId);
    for (const track of item.tracks) {
      const file = path.join(dummyRoot, 'media', item.sourceRelativeFolder, track.relativePath);
      if (hash(file) !== track.sourceSha256) throw Error(`Changed dummy: ${file}`);
      locks[path.relative(root, file)] = track.sourceSha256;
    }
    selected.push({ caseId: item.caseId, population, corpusFile, sourceRoot: path.join(dummyRoot, 'media'),
      expectations: path.join(reviewDirectory, 'expectations.json'), sourceRelativeFolder: item.sourceRelativeFolder, tracks: item.tracks.length });
  }
  locks[path.relative(root, corpusFile)] = hash(corpusFile);
  locks[path.relative(root, path.join(reviewDirectory, 'expectations.json'))] = hash(path.join(reviewDirectory, 'expectations.json'));
}
if (selected.length !== 51 || new Set(selected.map(item => item.caseId)).size !== 51
  || selected.reduce((sum, item) => sum + item.tracks, 0) !== 539) throw Error('Selection changed');
const reviewedIds = read(path.join(fixtures, 'batch-suite.json')).cases.filter(item => item.scored).map(item => item.caseId);
if (reviewedIds.length !== selected.length || reviewedIds.some(id => !selected.some(item => item.caseId === id))) throw Error('Reviewed membership changed');
save(path.join(here, 'selection.json'), { profiles, cases: selected, binary, binaryHash, locks });
const providerManifest = path.join(here, 'providers/manifest.json');
importPools(path.join(fixtures, 'candidate-pools.json'), providerManifest);
const jobs = profiles.flatMap(profile => selected.map(item => ({ ...item, profile })));
const outcomes = [];
async function run(job) {
  if (hash(binary) !== binaryHash) throw Error('Native binary changed');
  const directory = path.join(here, job.profile, job.caseId);
  fs.mkdirSync(directory, { recursive: true });
  const log = fs.openSync(path.join(directory, 'native.log'), 'wx');
  const server = createService(providerManifest);
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  try {
    console.log('Starting', job.profile, job.caseId);
    const env = { ...process.env,
      SOUNDROBE_AUTO_TAG_EVAL_CORPUS: job.corpusFile, SOUNDROBE_AUTO_TAG_EVAL_EXPECTATIONS: job.expectations,
      SOUNDROBE_AUTO_TAG_EVAL_SOURCE_ROOT: job.sourceRoot, SOUNDROBE_AUTO_TAG_EVAL_ARTIFACT_DIR: directory,
      SOUNDROBE_AUTO_TAG_EVAL_RUN_ID: `${path.basename(here)}-${job.profile}-${job.caseId}`, SOUNDROBE_AUTO_TAG_EVAL_PROFILE: job.profile,
      SOUNDROBE_AUTO_TAG_EVAL_ARTISTS: '', SOUNDROBE_AUTO_TAG_EVAL_CASES: job.caseId,
      SOUNDROBE_AUTO_TAG_EVAL_MOCK_URL: `http://127.0.0.1:${server.address().port}` };
    const child = spawn(binary, ['--exact', 'commands::auto_tag::eval_tests::live_auto_tag_eval', '--ignored', '--nocapture'],
      { cwd: root, env, stdio: ['ignore', log, log] });
    const [exitCode, signal] = await once(child, 'exit');
    const inventory = await (await fetch(`${env.SOUNDROBE_AUTO_TAG_EVAL_MOCK_URL}/__fixtures`)).json();
    save(path.join(directory, 'provider-inventory-final.json'), { requests: inventory.requests, misses: inventory.misses });
    const outcome = { caseId: job.caseId, profile: job.profile, exitCode, signal,
      missingRoutes: inventory.misses.length, providerRequests: inventory.requests.length };
    save(path.join(directory, 'execution.json'), outcome);
    outcomes.push(outcome);
    console.log('Finished', job.profile, job.caseId, 'exit', exitCode, 'missing routes', outcome.missingRoutes);
  } finally {
    fs.closeSync(log);
    server.closeAllConnections();
    await new Promise(resolve => server.close(resolve));
  }
}
async function worker() { while (jobs.length) await run(jobs.shift()); }
Promise.all(Array.from({ length: 4 }, worker)).then(() => {
  save(path.join(here, 'executions.json'), outcomes);
  for (const [file, expected] of Object.entries(locks)) if (hash(path.join(root, file)) !== expected) throw Error(`Input changed during replay: ${file}`);
  if (hash(binary) !== binaryHash || outcomes.some(item => item.exitCode !== 0 || item.missingRoutes)) process.exitCode = 1;
}).catch(error => { console.error(error); process.exitCode = 1; });
