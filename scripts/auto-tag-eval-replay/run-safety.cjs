// Offline development safety matrix. Never use original media or live models.
// Usage: node scripts/auto-tag-eval-replay/run-safety.cjs NEW_OUTPUT BUILD_JSONL_OR_FROZEN_RUN [ORACLE] [JOB,...]
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const { spawn, execFileSync } = require('node:child_process');
const { once } = require('node:events');
const { createService, importPools } = require('../mock-provider-service.cjs');
const root = process.cwd();
const [outputArg, buildArg, oracleArg, jobArg] = process.argv.slice(2);
const output = path.resolve(outputArg), buildFile = path.resolve(buildArg);
const read = file => JSON.parse(fs.readFileSync(file));
const hash = file => crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');
const save = (file, value) => fs.writeFileSync(file, JSON.stringify(value, null, 2) + '\n', { flag: 'wx' });
fs.mkdirSync(output);
const fixtures = path.join(root, 'test/fixtures/tauri/auto-tag-eval');
const dummy = path.join(root, '.amp/in/eval-dummies/original');
const corpusFile = path.join(dummy, 'corpus.json');
const corpus = read(corpusFile);
const selectionFile = oracleArg ? path.resolve(oracleArg) : path.join(__dirname, 'safety-selection.json');
const selection = read(selectionFile);
const jobFilter = jobArg?.split(',');
if (jobFilter?.some(name => !selection.jobs.some(job => job.name === name))) throw Error('Unknown safety job');
const selectedJobs = selection.jobs.filter(job => !jobFilter || jobFilter.includes(job.name));
const metadataFile = path.join(__dirname, 'safety-model-inputs.json');
const metadata = read(metadataFile);
const negativeQueriesFile = path.join(__dirname, 'safety-negative-queries.json');
const negativeQueries = read(negativeQueriesFile);
fs.copyFileSync(selectionFile, path.join(output, 'selection.json'), fs.constants.COPYFILE_EXCL);
for (const file of [selectionFile, metadataFile, negativeQueriesFile]) {
  fs.copyFileSync(file, path.join(output, path.basename(file)), fs.constants.COPYFILE_EXCL);
}
const preserved = fs.statSync(buildFile).isDirectory() ? read(path.join(buildFile, 'snapshot.json')) : null;
const builds = preserved ? [] : fs.readFileSync(buildFile, 'utf8').trim().split('\n').map(JSON.parse);
const built = preserved ? path.join(buildFile, 'native-evaluator')
  : builds.find(item => item.reason === 'compiler-artifact' && item.target.name === 'soundrobe_lib' && item.executable)?.executable;
if (!built) throw Error('Native test executable missing');
if (preserved && (hash(built) !== preserved.binarySha256
    || hash(path.join(buildFile, 'source.tgz')) !== preserved.sourceArchiveSha256)) throw Error('Changed frozen binary/source');
const binary = path.join(output, 'native-evaluator');
fs.copyFileSync(built, binary, fs.constants.COPYFILE_EXCL);
const locks = Object.fromEntries(read(path.join(fixtures, 'batch-suite.json')).inputs.map(item =>
  [path.join('test/fixtures/tauri', item.path), item.sha256]));
locks[path.relative(root, corpusFile)] = hash(corpusFile);
locks[path.relative(root, selectionFile)] = hash(selectionFile);
locks[path.relative(root, metadataFile)] = hash(metadataFile);
locks[path.relative(root, negativeQueriesFile)] = hash(negativeQueriesFile);
const sourceFiles = execFileSync('rg', ['--files', 'src-tauri/src', 'src/components', 'src/shared'], { encoding: 'utf8' }).trim().split('\n')
  .concat(['src-tauri/Cargo.toml', 'src-tauri/Cargo.lock', 'data/provider-title-aliases.json',
    'scripts/mock-provider-service.cjs', 'scripts/review_auto_tag_subset.py',
    'scripts/auto-tag-eval-replay/run-safety.cjs', 'scripts/auto-tag-eval-replay/audit-safety.py',
    'scripts/auto-tag-eval-replay/compare-safety.py', 'scripts/auto-tag-eval-replay/test_safety_audit.py']);
const harnessLocks = Object.fromEntries(sourceFiles.map(file => [file, hash(file)]));
const sourceLocks = preserved ? preserved.sourceLocks : harnessLocks;
const expectations = {};
for (const name of new Set(Object.values(selection.evidence).map(item => item.manifest))) {
  const directory = path.join(output, 'review-' + path.basename(name, '.json'));
  execFileSync('python3', ['-B', 'scripts/review_auto_tag_subset.py', '--corpus', corpusFile,
    '--manifest', path.join(fixtures, name), '--fixture-root', fixtures, '--output-dir', directory], { stdio: 'pipe' });
  expectations[name] = path.join(directory, 'expectations.json');
}
for (const id of Object.keys(selection.evidence)) {
  const item = corpus.cases.find(item => item.caseId === id);
  for (const track of item.tracks) {
    const file = path.join(dummy, 'media', item.sourceRelativeFolder, track.relativePath);
    if (hash(file) !== track.sourceSha256) throw Error('Changed dummy input');
    locks[path.relative(root, file)] = track.sourceSha256;
  }
}
for (const [file, expected] of Object.entries(locks)) if (hash(file) !== expected) throw Error(`Changed input: ${file}`);
if (preserved) fs.copyFileSync(path.join(buildFile, 'source.tgz'), path.join(output, 'source.tgz'), fs.constants.COPYFILE_EXCL);
else execFileSync('tar', ['-czf', path.join(output, 'source.tgz'), ...sourceFiles], { cwd: root });
execFileSync('tar', ['-czf', path.join(output, 'harness.tgz'), ...sourceFiles], { cwd: root });
fs.writeFileSync(path.join(output, 'source.diff'), execFileSync('git', ['diff', '--', 'src-tauri', 'scripts', 'src/components']));
const manifestFile = path.join(output, 'providers.json');
importPools(path.join(fixtures, 'candidate-pools.json'), manifestFile);
const providers = read(manifestFile);
const selectedCases = [...new Set(selectedJobs.map(job => job.caseId))].map(id => corpus.cases.find(item => item.caseId === id));
const jobs = selectedJobs.flatMap(job => selection.profiles.map(profile => ({ ...job, profile })));
save(path.join(output, 'snapshot.json'), { schemaVersion: 1,
  requestedEvaluations: jobs.length * 2,
  requestedPhysicalFiles: jobs.reduce((sum, job) => sum + selectedCases.find(item => item.caseId === job.caseId).tracks.length * 2, 0),
  uniquePhysicalFiles: selectedCases.reduce((sum, item) => sum + item.tracks.length, 0),
  cases: selectedCases, locks, sourceLocks, harnessLocks, jobFilter,
  binarySourceCondition: preserved ? path.relative(root, buildFile) : 'current checkout',
  harnessArchiveSha256: hash(path.join(output, 'harness.tgz')), selectionSha256: hash(selectionFile),
  sourceArchiveSha256: hash(path.join(output, 'source.tgz')), binarySha256: hash(binary),
  gitHead: execFileSync('git', ['rev-parse', 'HEAD'], { encoding: 'utf8' }).trim(),
  versions: { node: process.version, python: execFileSync('python3', ['--version'], { encoding: 'utf8' }).trim(),
    rust: execFileSync('rustc', ['--version'], { encoding: 'utf8' }).trim() },
  budget: { modelRequestsPerProcess: 2, providerBudgets: 'unchanged production coordinator' },
  evidence: 'Scripted assistance and controlled provider visibility; zero grounded-assistance accuracy credit' });

// Deliberate counterfactual discovery wrappers, not new authentic captures.
// Only the incompatible release detail remains available. Artist identity stays
// real; other concrete identities return explicit unavailable responses.
function restrictToWrongRelease(records, wrong) {
  return records.map(record => {
    const body = read(path.resolve(output, record.bodyFile));
    let replacement;
    if (record.provider === 'musicbrainz') {
      if (record.path === '/release' || Array.isArray(body.releases)) {
        replacement = { ...body, releases: [wrong], 'release-count': 1, count: 1 };
      } else if (record.path.startsWith('/release/')) {
        replacement = record.path === `/release/${wrong.id}` ? wrong : { error: 'scripted_identity_unavailable' };
      } else if (record.path.startsWith('/recording') || record.path.startsWith('/release-group')) {
        replacement = { error: 'scripted_identity_unavailable' };
      }
    } else if (record.path.startsWith('/releases/') || record.path.startsWith('/masters/')) {
      replacement = { error: 'scripted_identity_unavailable' };
    } else if (Array.isArray(body.releases)) {
      replacement = { ...body, releases: [], pagination: { page: 1, pages: 1, items: 0 } };
    } else if (record.path === '/database/search' && record.query?.type !== 'artist') {
      replacement = { ...body, results: [], pagination: { page: 1, pages: 1, items: 0 } };
    }
    if (!replacement) return record;
    const { bodyFile, sha256, ...rest } = record;
    return { ...rest, body: replacement, status: replacement.error ? 404 : 200,
      source: { kind: replacement.error ? 'explicit_unavailable_evidence' : 'scripted_negative_discovery',
        original: record.source, wrongReleaseId: wrong.id, detailSha256: hash(path.join(fixtures,
          Object.values(selection.evidence).find(item => item.wrongSnapshot && read(path.join(fixtures, item.wrongSnapshot)).id === wrong.id).wrongSnapshot)) } };
  });
}

function withoutGenre(value) {
  if (Array.isArray(value)) return value.map(withoutGenre);
  if (!value || typeof value !== 'object') return value;
  return Object.fromEntries(Object.entries(value)
    .filter(([key]) => !['genre', 'genres', 'style', 'styles'].includes(key))
    .map(([key, item]) => [key, withoutGenre(item)]));
}

async function execute(job) {
  const { profile, scenario } = job;
  const directory = path.join(output, `${job.name}-${profile}`);
  fs.mkdirSync(directory);
  const manifest = path.join(directory, 'providers.json');
  const evidence = selection.evidence[job.caseId];
  const providerRecords = job.wrongOnly
    ? restrictToWrongRelease(providers.records, read(path.join(fixtures, evidence.wrongSnapshot))) : providers.records;
  if (job.wrongOnly) {
    for (const query of negativeQueries.records) {
      const wrong = read(path.join(fixtures, evidence.wrongSnapshot));
      providerRecords.push({ ...query, status: 200,
        body: query.provider === 'musicbrainz'
          ? { releases: [wrong], 'release-count': 1, 'release-offset': 0 }
          : { results: [], pagination: { page: 1, pages: 1, items: 0 } },
        source: { kind: 'scripted_negative_discovery', wrongReleaseId: wrong.id } });
    }
  }
  const records = providerRecords.map(record => {
    if (!record.bodyFile) return record;
    const bodyFile = path.relative(directory, path.resolve(output, record.bodyFile));
    if (!job.providerGenreAbsent || record.provider !== 'discogs') return { ...record, bodyFile };
    const original = read(path.join(directory, bodyFile));
    const body = withoutGenre(original);
    if (JSON.stringify(body) === JSON.stringify(original)) return { ...record, bodyFile };
    const { sha256, bodyFile: _, ...rest } = record;
    return { ...rest, body, source: { kind: 'scripted_genre_absence', original: record.source,
      originalBodyFile: bodyFile, originalSha256: sha256 ?? hash(path.join(directory, bodyFile)) } };
  });
  const album = job.caseId === '60ebec7ffadb' ? 'Guitar Songs'
    : job.caseId === '54e7923a3000' ? 'dont smile at me'
      : job.caseId === '07e09ba5552a' ? 'my future' : 'Lo vas a olvidar';
  const rejectedPayload = metadata.rejections[`${job.name}-${profile}`];
  const scriptedModel = rejectedPayload
    ? { maxRequests: 2, schemaName: 'TagCorrectionResponse', payload: rejectedPayload, response: {}, status: 400 }
    : job.name === 'unproven-remaster' && metadata.payloads[profile]
      ? { maxRequests: 2, schemaName: 'TagCorrectionResponse', payload: metadata.payloads[profile], response: metadata.response }
    : { maxRequests: 2, payload: { artist: 'Billie Eilish', album, tracks: evidence.titles ?? [] },
      response: { genre: 'Pop', confidence: scenario === 'rejected' ? 0.2 : 0.8 },
      status: scenario === 'failed' ? 400 : 200 };
  save(manifest, { ...providers, records, ...(scenario === 'disabled' ? {} : { scriptedModel }) });
  const inputFile = path.join(directory, 'input.json');
  save(inputFile, { genres: job.genres ?? {}, cancelAfterGenre: scenario === 'cancelled' });
  save(path.join(directory, 'job-lock.json'), { manifestSha256: hash(manifest), inputSha256: hash(inputFile) });
  const server = createService(manifest);
  server.listen(0, '127.0.0.1');
  await once(server, 'listening');
  const base = `http://127.0.0.1:${server.address().port}`;
  const log = fs.openSync(path.join(directory, 'native.log'), 'wx');
  try {
    const env = Object.fromEntries(['PATH', 'HOME', 'TMPDIR'].filter(key => process.env[key]).map(key => [key, process.env[key]]));
    Object.assign(env, { SOUNDROBE_AUTO_TAG_EVAL_CORPUS: corpusFile,
      SOUNDROBE_AUTO_TAG_EVAL_EXPECTATIONS: expectations[evidence.manifest],
      SOUNDROBE_AUTO_TAG_EVAL_SOURCE_ROOT: path.join(dummy, 'media'),
      SOUNDROBE_AUTO_TAG_EVAL_ARTIFACT_DIR: directory, SOUNDROBE_AUTO_TAG_EVAL_RUN_ID: `${job.name}-${profile}`,
      SOUNDROBE_AUTO_TAG_EVAL_PROFILE: profile, SOUNDROBE_AUTO_TAG_EVAL_CASES: job.caseId,
      SOUNDROBE_AUTO_TAG_EVAL_MOCK_URL: base, SOUNDROBE_AUTO_TAG_EVAL_SAFETY_INPUT: inputFile,
      SOUNDROBE_AUTO_TAG_EVAL_ASSISTANCE: scenario === 'disabled' ? 'disabled' : 'scripted' });
    const child = spawn(binary, ['--exact', 'commands::auto_tag::eval_tests::live_auto_tag_eval', '--ignored', '--nocapture'],
      { cwd: root, env, stdio: ['ignore', log, log] });
    const [exitCode, signal] = await once(child, 'exit');
    const inventory = await (await fetch(`${base}/__fixtures`)).json();
    save(path.join(directory, 'inventory.json'), inventory);
    const execution = { name: job.name, caseId: job.caseId, profile, exitCode, signal,
      providerRequests: inventory.requests.length, modelRequests: inventory.modelRequests.length, missingRoutes: inventory.misses.length };
    save(path.join(directory, 'execution.json'), execution);
    console.log(JSON.stringify(execution));
    return execution;
  } finally {
    fs.closeSync(log);
    server.closeAllConnections();
    await new Promise(resolve => server.close(resolve));
  }
}
async function run() {
  const executions = [];
  async function worker() { while (jobs.length) executions.push(await execute(jobs.shift())); }
  await Promise.all(Array.from({ length: 3 }, worker));
  save(path.join(output, 'executions.json'), executions);
  for (const [file, expected] of Object.entries({ ...locks, ...harnessLocks })) {
    if (hash(file) !== expected) throw Error(`Changed during replay: ${file}`);
  }
  if (executions.some(item => item.exitCode !== 0 || item.missingRoutes)) process.exitCode = 1;
}
run().catch(error => { console.error(error); process.exitCode = 1; });
