// Bounded scheduling of the existing offline native evaluator.
const fs = require('node:fs'), path = require('node:path'), crypto = require('node:crypto');
const { spawn } = require('node:child_process');
const { once } = require('node:events');
const { createService } = require('../mock-provider-service.cjs');
const read = p => JSON.parse(fs.readFileSync(p));
const hash = p => crypto.createHash('sha256').update(fs.readFileSync(p)).digest('hex');
const save = (p, value) => fs.writeFileSync(p, JSON.stringify(value, null, 2) + '\n', { flag: 'wx' });

function validatePlan(plan, packet) {
  if (plan.assistance !== 'disabled' || plan.modelRequestsMaximum !== 0) throw Error('Assistance must be disabled');
  const expected = packet.cases.flatMap(c => packet.profiles.map(profile => `${c.caseId}/${profile}/${c.tracks}`)).sort();
  const actual = plan.jobs.map(j => `${j.caseId}/${j.profile}/${j.tracks}`).sort();
  if (!packet.cases.length || new Set(packet.cases.map(c => c.caseId)).size !== packet.cases.length
      || JSON.stringify(packet.profiles) !== JSON.stringify(['folder_filename', 'assisted_without_ids', 'tagged_recovery'])
      || JSON.stringify(actual) !== JSON.stringify(expected)) throw Error('Locked case/profile membership changed');
}

async function main(output) {
  const root = process.cwd();
  output = path.resolve(output);
  const planFile = path.join(output, 'execution-plan.json'), plan = read(planFile);
  validatePlan(plan, read(plan.packetSelection));
  const check = () => {
    for (const [file, expected] of Object.entries(plan.locks)) {
      if (hash(path.resolve(root, file)) !== expected) throw Error(`Changed binding: ${file}`);
    }
    if (hash(plan.binary) !== plan.binarySha256) throw Error('Changed executable');
  };
  check();
  const providers = read(plan.providers);
  if (providers.scriptedModel) throw Error('Model scripts are forbidden');
  const records = providers.records.map(r => ({ ...r, bodyFile: path.resolve(path.dirname(plan.providers), r.bodyFile) }));
  const pending = [...plan.jobs], outcomes = [];
  fs.mkdirSync(path.join(output, 'runs')); // Never append to a prior execution.
  async function execute(job) {
    check();
    const directory = path.join(output, 'runs', `${job.profile}-${job.caseId}`);
    fs.mkdirSync(directory);
    const manifest = path.join(directory, 'providers.json');
    save(manifest, { ...providers, records });
    save(path.join(directory, 'job-lock.json'), { planSha256: hash(planFile), manifestSha256: hash(manifest),
      driverSha256: hash(__filename), assistance: 'disabled', modelRequestsMaximum: 0 });
    const service = createService(manifest);
    service.listen(0, '127.0.0.1'); await once(service, 'listening');
    const log = fs.openSync(path.join(directory, 'native.log'), 'wx');
    try {
      const env = Object.fromEntries(['PATH', 'HOME', 'TMPDIR'].filter(k => process.env[k]).map(k => [k, process.env[k]]));
      Object.assign(env, { SOUNDROBE_AUTO_TAG_EVAL_CORPUS: job.corpusFile,
        SOUNDROBE_AUTO_TAG_EVAL_EXPECTATIONS: job.expectations, SOUNDROBE_AUTO_TAG_EVAL_SOURCE_ROOT: job.sourceRoot,
        SOUNDROBE_AUTO_TAG_EVAL_ARTIFACT_DIR: directory, SOUNDROBE_AUTO_TAG_EVAL_RUN_ID: `five-disabled-${job.profile}-${job.caseId}`,
        SOUNDROBE_AUTO_TAG_EVAL_PROFILE: job.profile, SOUNDROBE_AUTO_TAG_EVAL_CASES: job.caseId,
        SOUNDROBE_AUTO_TAG_EVAL_ARTISTS: '', SOUNDROBE_AUTO_TAG_EVAL_DONOR_TRACE: '1',
        SOUNDROBE_AUTO_TAG_EVAL_ASSISTANCE: 'disabled',
        SOUNDROBE_AUTO_TAG_EVAL_MOCK_URL: `http://127.0.0.1:${service.address().port}` });
      save(path.join(directory, 'evaluation-env.json'), Object.fromEntries(Object.entries(env).filter(([k]) => k.startsWith('SOUNDROBE_'))));
      const child = spawn(plan.binary, ['--exact', 'commands::auto_tag::eval_tests::live_auto_tag_eval', '--ignored', '--nocapture'],
        { cwd: root, env, stdio: ['ignore', log, log] });
      const [exitCode, signal] = await once(child, 'exit');
      const inventory = await (await fetch(`${env.SOUNDROBE_AUTO_TAG_EVAL_MOCK_URL}/__fixtures`)).json();
      save(path.join(directory, 'provider-inventory-final.json'), { requests: inventory.requests,
        modelRequests: inventory.modelRequests, misses: inventory.misses });
      const result = { caseId: job.caseId, profile: job.profile, exitCode, signal,
        providerRequests: inventory.requests.length, modelRequests: inventory.modelRequests.length, misses: inventory.misses.length };
      save(path.join(directory, 'execution.json'), result); outcomes.push(result);
      console.log(JSON.stringify(result));
    } finally {
      fs.closeSync(log); service.closeAllConnections(); await new Promise(resolve => service.close(resolve));
    }
  }
  async function worker() { while (pending.length) await execute(pending.shift()); }
  await Promise.all(Array.from({ length: 3 }, worker));
  save(path.join(output, 'executions.json'), outcomes);
  check();
  if (outcomes.length !== plan.jobs.length || outcomes.some(r => r.exitCode !== 0 || r.signal || r.modelRequests || r.misses)) process.exitCode = 1;
}
module.exports = { validatePlan };
if (require.main === module) main(process.argv[2]).catch(error => { console.error(error); process.exitCode = 1; });
