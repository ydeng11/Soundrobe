// Same evaluator/environment/strict mock as maintained run-safety and run-native.
const fs = require('node:fs'), path = require('node:path'), crypto = require('node:crypto');
const {spawn} = require('node:child_process');
const {once} = require('node:events');
const root = process.cwd(), here = __dirname;
const {createService} = require(path.join(root, 'scripts/mock-provider-service.cjs'));
const read = p => JSON.parse(fs.readFileSync(p));
const save = (p, x) => fs.writeFileSync(p, JSON.stringify(x, null, 2), {flag: 'wx'});
const hash = p => crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const frozen = read(path.join(here, 'freeze.json'));
const binary = path.join(here, 'native-evaluator');
if (hash(binary) !== frozen.binarySha256) throw Error('Unbound native binary');
const prior = path.join(root, frozen.prior);
const providers = read(path.join(prior, 'providers/manifest.json'));
const records = providers.records.map(r => r.bodyFile
  ? {...r, bodyFile: path.resolve(prior, 'providers', r.bodyFile)} : r);
const pilot = process.argv.includes('--pilot');
const names = new Set([
  'gap-accepted-assisted_without_ids-8452b6f2b3ea',
  'gap-accepted-tagged_recovery-d587d60846f0',
  'gap-cancelled-folder_filename-e8a8a8d6b27d',
  'hold-accepted-assisted_without_ids-f7756354af37',
  'hold-accepted-folder_filename-0e7c8a578cea',
]);
const jobs = frozen.jobs.filter(j => !pilot || names.has(j.name));
const output = path.join(here, pilot ? 'pilot' : 'runs');
fs.mkdirSync(output);
const pending = [...jobs], executions = [];
async function execute(job) {
  const directory = path.join(output, job.name);
  fs.mkdirSync(directory);
  const manifest = path.join(directory, 'providers.json');
  save(manifest, {...providers, records, ...(job.scenario === 'disabled' ? {} : {scriptedModel: job.script})});
  const input = path.join(directory, 'input.json');
  save(input, {genres: job.genres, cancelAfterGenre: job.scenario === 'cancelled'});
  save(path.join(directory, 'job-lock.json'), {manifestSha256: hash(manifest), inputSha256: hash(input),
    frozenSelectionSha256: hash(path.join(here, 'freeze.json')), driverSha256: hash(__filename)});
  const service = createService(manifest);
  service.listen(0, '127.0.0.1');
  await once(service, 'listening');
  const log = fs.openSync(path.join(directory, 'native.log'), 'wx');
  try {
    const env = Object.fromEntries(['PATH', 'HOME', 'TMPDIR'].filter(k => process.env[k]).map(k => [k, process.env[k]]));
    Object.assign(env, {SOUNDROBE_AUTO_TAG_EVAL_CORPUS: job.case.corpusFile,
      SOUNDROBE_AUTO_TAG_EVAL_EXPECTATIONS: job.case.expectations,
      SOUNDROBE_AUTO_TAG_EVAL_SOURCE_ROOT: job.case.sourceRoot,
      SOUNDROBE_AUTO_TAG_EVAL_ARTIFACT_DIR: directory,
      SOUNDROBE_AUTO_TAG_EVAL_RUN_ID: job.name,
      SOUNDROBE_AUTO_TAG_EVAL_PROFILE: job.profile,
      SOUNDROBE_AUTO_TAG_EVAL_CASES: job.case.caseId, SOUNDROBE_AUTO_TAG_EVAL_ARTISTS: '',
      SOUNDROBE_AUTO_TAG_EVAL_DONOR_TRACE: '1', SOUNDROBE_AUTO_TAG_EVAL_SAFETY_INPUT: input,
      SOUNDROBE_AUTO_TAG_EVAL_MOCK_URL: `http://127.0.0.1:${service.address().port}`,
      SOUNDROBE_AUTO_TAG_EVAL_ASSISTANCE: job.scenario === 'disabled' ? 'disabled' : 'scripted'});
    const child = spawn(binary, ['--exact', 'commands::auto_tag::eval_tests::live_auto_tag_eval', '--ignored', '--nocapture'],
      {cwd: root, env, stdio: ['ignore', log, log]});
    const [exitCode, signal] = await once(child, 'exit');
    const inventory = await (await fetch(`${env.SOUNDROBE_AUTO_TAG_EVAL_MOCK_URL}/__fixtures`)).json();
    save(path.join(directory, 'provider-inventory-final.json'), {requests: inventory.requests,
      modelRequests: inventory.modelRequests, misses: inventory.misses});
    const result = {name: job.name, exitCode, signal, providerRequests: inventory.requests.length,
      modelRequests: inventory.modelRequests.length, misses: inventory.misses.length};
    save(path.join(directory, 'execution.json'), result);
    executions.push(result);
    console.log(JSON.stringify(result));
  } finally {
    fs.closeSync(log); service.closeAllConnections(); await new Promise(resolve => service.close(resolve));
  }
}
(async () => {
  async function worker() {while (pending.length) await execute(pending.shift());}
  await Promise.all(Array.from({length: 4}, worker));
  save(path.join(here, pilot ? 'pilot-executions.json' : 'executions.json'), executions);
  if (executions.some(r => r.exitCode || r.signal || r.misses)) process.exitCode = 1;
})().catch(error => {console.error(error); process.exitCode = 1;});
