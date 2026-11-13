// Synthetic finalization producer, not a tagging/scoring run or accuracy evidence.
// Only a native final-* control may select this fixture. All owner/session
// transitions use the real broker; no registry rows or completion files are forged.
const fs = require('./bounded-storage.cjs'), path = require('node:path'), crypto = require('node:crypto');
const { start, rpc, lease, testApproval } = require('./mock-authority.cjs');
const read = file => JSON.parse(fs.readFileSync(file));
const digest = bytes => crypto.createHash('sha256').update(bytes).digest('hex');
async function main(root, binary) {
  const supervision = read(path.join(root, 'native-supervision.json'));
  if (!supervision.control.startsWith('final-') || process.env.SOUNDROBE_NATIVE_MANAGED !== '1') throw Error('native control only');
  const membership = read(path.join(root, 'requested-membership.json'));
  const executable = digest(fs.readFileSync(binary));
  const authority = await start(binary, root, membership, { executable, supervision }, undefined, supervision);
  const sessions = [];
  for (const stage of ['preparation', 'execution']) {
    if (stage === 'execution') {
      await authority.command({ op: 'freeze' });
      await authority.command({ op: 'approve', approval: testApproval(await authority.command({ op: 'report' })) });
    }
    for (const { name: job } of membership.evaluations) {
      const session = await authority.command({ op: 'register', job, stage,
        endpoint: 'http://127.0.0.1:1234', manifest_sha256: digest('synthetic finalization fixture') });
      const native = await lease(authority.socket, session.client, 'native');
      const fixture = await lease(authority.socket, session.service, 'fixture');
      await rpc(authority.socket, session.client, { op: 'attach', executable_sha256: executable,
        preparation_sha256: stage === 'execution' ? digest(job) : null });
      if (stage === 'preparation') {
        await rpc(authority.socket, session.client, { op: 'seal_inputs' });
        await rpc(authority.socket, session.client, { op: 'prepared', preparation_sha256: digest(job), request_sha256: null });
      } else await rpc(authority.socket, session.client, { op: 'finish', observation_sha256: digest('synthetic observation') });
      await native.release();
      await rpc(authority.socket, session.client, { op: 'close' });
      await fixture.release();
      sessions.push({ job, stage });
    }
  }
  fs.writeFileSync(path.join(root, 'results.json'), JSON.stringify({ requestedFieldSlots: 216,
    controlOnly: true, sessions, groundedAssistanceAccuracyCredit: 0 }), { flag: 'wx' });
  await authority.command({ op: 'stage_result', result_sha256: digest(fs.readFileSync(path.join(root, 'results.json'))) });
  const report = await authority.command({ op: 'report' });
  await authority.settle(false, report);
  fs.writeFileSync(path.join(root, 'final-control-produced.json'), JSON.stringify({ sessions: sessions.length,
    requestedFieldSlots: report.membership.requestedFieldSlots, controlOnly: true }), { flag: 'wx' });
}
main(...process.argv.slice(2)).then(() => process.exit(0)).catch(error => { console.error(error); process.exit(78); });
