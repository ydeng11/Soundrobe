// Real native process faults with disposable mock inputs only.
const fs = require('node:fs'), path = require('node:path'), assert = require('node:assert/strict');
const { spawn, execFileSync } = require('node:child_process');
const { once } = require('node:events');
const crypto = require('node:crypto');
const { rpc } = require('./mock-authority.cjs');
const read = file => JSON.parse(fs.readFileSync(file));
const save = (file, value) => fs.writeFileSync(file, JSON.stringify(value, null, 2) + '\n', { flag: 'wx' });
const delay = ms => new Promise(resolve => setTimeout(resolve, ms));
async function until(predicate, ms, label) {
  const end = performance.now() + ms;
  while (!predicate()) { assert.ok(performance.now() < end, label); await delay(5); }
}
function absent(group) {
  try { process.kill(-group, 0); return false; }
  catch (error) { return error.code === 'ESRCH'; }
}
async function main(output, binary, only) {
  output = path.resolve(output); binary = path.resolve(binary);
  assert.ok(path.dirname(output).startsWith(fs.realpathSync('.amp/in/artifacts') + path.sep));
  fs.mkdirSync(output);
  const digest = file => crypto.createHash('sha256').update(fs.readFileSync(file)).digest('hex');
  const locks = [binary, __filename, require.resolve('./run-bounded.cjs'), require.resolve('./mock-authority.cjs'),
    require.resolve('./run-supervised.cjs'), require.resolve('./finalization-control.cjs'),
    require.resolve('./bounded-storage.cjs'), require.resolve('./audit-supervised.py'),
    'test/fixtures/tauri/auto-tag-eval/supervised-requested-membership.json',
    ...execFileSync('rg', ['--files', 'src-tauri/src'], { encoding: 'utf8' }).trim().split('\n')]
    .map(file => ({ path: path.relative(process.cwd(), file), sha256: digest(file) }));
  save(path.join(output, 'source-and-tool-locks.json'), locks);
  const checkLocks = () => { for (const lock of locks) assert.equal(digest(lock.path), lock.sha256, lock.path); };
  const results = [];
  for (const control of ['broker-loss', 'supervisor-loss', 'receiver-loss', 'blocked-node',
    'stalled-storage', 'journal-lock', 'supervisor-suspend', 'broker-suspend', 'anchor-loss', 'anchor-running-loss', 'storage-denial']) {
    if (only && only !== control) continue;
    checkLocks();
    const root = path.join(output, control), log = fs.openSync(root + '.log', 'wx');
    const preparation = control === 'storage-denial' ? 300000 : ['blocked-node', 'supervisor-suspend'].includes(control) ? 3000 : 8000;
    let executable = binary, args = ['--exact',
      'infra::mock_run_authority::supervisor::ownership::native_supervision_process', '--ignored', '--nocapture'];
    if (control === 'anchor-running-loss') {
      const entry = path.join(output, 'anchor-running-entry');
      fs.writeFileSync(entry, `#!/bin/sh\nexport SOUNDROBE_NATIVE_CONTROL=anchor-running-loss SOUNDROBE_NATIVE_PREPARATION_MS=8000\nexec '${binary.replaceAll("'", "'\\''")}' "$@"\n`, { mode: 0o700, flag: 'wx' });
      executable = process.execPath;
      args = [require.resolve('./run-supervised.cjs'), root, entry];
    }
    const child = spawn(executable, args, {
      env: { PATH: process.env.PATH, SOUNDROBE_NATIVE_ROLE: 'receiver', SOUNDROBE_NATIVE_OUTPUT: root,
        SOUNDROBE_NATIVE_PREPARATION_MS: String(preparation), SOUNDROBE_NATIVE_CONTROL: control === 'receiver-loss' ? '' : control },
      stdio: ['ignore', log, log],
    });
    fs.closeSync(log);
    let status;
    const exited = once(child, 'exit').then(value => status = value);
    let pipe, locker;
    try {
      if (control === 'storage-denial') {
        await until(() => status, 5000, 'native storage-denial termination');
        await exited;
        const identity = read(path.join(root, 'group-identity.json'));
        await until(() => absent(identity.group), 1500, 'storage-denied execution group quiescence');
        assert.equal(status[0],78);
        assert.ok(!fs.existsSync(path.join(root,'must-not-create')));
        assert.ok(!fs.existsSync(path.join(root,'receiver-outcome.json')));
        const audit = JSON.parse(execFileSync('python3', ['-B', require.resolve('./audit-supervised.py'), root, '--incomplete'],
          {env:{PATH:process.env.PATH},encoding:'utf8'}));
        assert.equal(audit.unavailableFieldSlots,216);
        assert.equal(audit.validatedFieldSlots,0);
        assert.equal(audit.slots.length,216);
        const events = fs.readFileSync(root + '.log', 'utf8').split('\n')
          .filter(line => line.startsWith('SOUNDROBE_STORAGE_DENIAL '))
          .map(line => JSON.parse(line.slice('SOUNDROBE_STORAGE_DENIAL '.length)));
        assert.equal(events.length, 1, 'process loss alone is not quota-denial evidence');
        const configuration = read(path.join(root, 'native-supervision.json'));
        const denial = events[0];
        assert.equal(denial.generation, configuration.generation);
        assert.equal(denial.target, path.join(root, 'must-not-create'));
        assert.equal(denial.requestedGrowth, 0);
        assert.equal(denial.requestedWrites, 1024 * 1048576);
        assert.equal(denial.ordinaryWriteLimit, 1016 * 1048576);
        assert.ok(denial.requestedWrites + denial.ordinaryWritesReserved > denial.ordinaryWriteLimit);
        assert.equal(audit.storage.accounting, 'nonrefundable_prepaid_application_vfs_not_physical_io');
        assert.equal(new Set(audit.slots.map(slot => slot.id)).size, 216);
        assert.ok(audit.slots.every(slot => slot.observation === 'unavailable' && slot.effects === 'unknown'));
        assert.equal(configuration.preparationMs,300000);
        checkLocks();
        results.push({control,status,actualGroupAbsent:true,...audit});
        save(path.join(root,'control-result.json'),results.at(-1));
        continue;
      }
      const readiness = control === 'anchor-loss' ? 'anchor-preexec-blocked'
        : control === 'journal-lock' ? 'journal-driver-idle.json' : 'authority/ready.json';
      // Readiness must fit the existing preparation budget. Only the fault's
      // observed boundary starts the separate, unchanged containment bound.
      await until(() => fs.existsSync(path.join(root, readiness)) || status,
        control === 'anchor-running-loss' ? 30000 : preparation, 'native readiness boundary');
      assert.ok(control === 'anchor-running-loss' || !status, `early native exit: ${control}`);
      assert.ok(fs.existsSync(path.join(root, readiness)), 'required native boundary');
      const identity = read(path.join(root, 'group-identity.json'));
      const config = read(path.join(root, 'native-supervision.json'));
      assert.notEqual(identity.group, config.receiverGroup);
      assert.notEqual(identity.group, identity.supervisorPid);
      assert.ok(!absent(identity.group));
      if (control === 'stalled-storage') {
        const fifo = path.join(root, 'storage-fifo');
        execFileSync('/usr/bin/mkfifo', [fifo]);
        pipe = fs.openSync(fifo, fs.constants.O_RDWR | fs.constants.O_NONBLOCK);
        fs.writeFileSync(path.join(root, 'storage-release'), 'open FIFO');
        await until(() => fs.existsSync(path.join(root, 'storage-read-started')), 1000, 'actual blocking storage read');
        await delay(100);
        assert.ok(!status && !absent(identity.group));
      } else if (['blocked-node', 'anchor-running-loss'].includes(control)) {
        await until(() => fs.existsSync(path.join(root, 'blocked-node.json')) || status, 2000, 'observed wire before Node stall');
        assert.ok((control === 'anchor-running-loss' || !status) && fs.existsSync(path.join(root, 'blocked-node.json')));
        await delay(100);
        assert.ok((control === 'anchor-running-loss' || !status) && !absent(identity.group));
      } else if (control === 'journal-lock') {
        const authority = read(path.join(root, 'authority/ready.json'));
        const before = await rpc(authority.socket, authority.owner, { op: 'report' });
        assert.equal(before.ownerLive, true, 'owner lease committed before external lock');
        assert.deepEqual(before.sessions, {}, 'driver has not started preparation writers');
        save(path.join(root, 'journal-before-lock.json'), before);
        // A reserved writer lock blocks the real write transaction while
        // allowing the broker's independent report reader to remain healthy.
        const script = 'import sqlite3,sys,time; c=sqlite3.connect(sys.argv[1]); c.execute("BEGIN IMMEDIATE"); open(sys.argv[2],"w").write("locked"); time.sleep(20)';
        locker = spawn('python3', ['-B', '-c', script, path.join(root, 'authority/registry.sqlite'), path.join(root, 'journal-held')],
          { env: { PATH: process.env.PATH }, stdio: 'ignore' });
        await until(() => fs.existsSync(path.join(root, 'journal-held')), 1000, 'SQLite reserved writer lock');
        // The actual write transaction contends with the externally held lock.
        void rpc(authority.socket, authority.owner, { op: 'freeze' }).catch(() => {});
        await until(() => fs.existsSync(path.join(root, 'journal-entered')), 1000, 'contended journal entry');
      }
      const faultAt = performance.now();
      if (control === 'anchor-running-loss') {
        // Unknown containment may consume the original ten-second failure
        // grace. The finite orphan outlives it, proving return needs no EOF.
        await until(() => status, 11000, 'launcher returns despite surviving payload');
        assert.equal(status[0], 1);
        assert.equal(read(path.join(root, 'launcher-outcome.json')).receiverExit, 78);
        assert.ok(!absent(identity.group), 'unknown containment is not reported as group absence');
        assert.ok(!fs.existsSync(path.join(root, 'receiver-outcome.json')));
        assert.equal(read(path.join(root, 'requested-membership.json')).requestedFieldSlots, 216);
        results.push({ control, status, launcherReturnMs: performance.now() - faultAt,
          containmentAtReturn: 'unknown', requestedFieldSlots: 216, liveAuthority: false });
        // The deliberately finite orphan cleans itself up. No external signal.
        await until(() => absent(identity.group), 22000, 'finite orphan self-exit');
        checkLocks();
        save(path.join(root, 'control-result.json'), results.at(-1));
        continue;
      }
      if (control === 'receiver-loss') child.kill('SIGKILL');
      else if (!['blocked-node', 'anchor-loss'].includes(control)) fs.writeFileSync(path.join(root, 'fault.request'), control);
      if (control.endsWith('-suspend')) {
        const pid = control === 'supervisor-suspend' ? identity.supervisorPid : identity.brokerPid;
        await until(() => {
          try { return execFileSync('/bin/ps', ['-o', 'state=', '-p', String(pid)], { encoding: 'utf8' }).includes('T'); }
          catch { return false; }
        }, 500, 'actual owner suspension before cleanup');
      }
      await until(() => absent(identity.group), control === 'blocked-node' ? 3500 : 1500, 'actual group ESRCH');
      const quiescentMs = performance.now() - faultAt;
      await until(() => status, control === 'supervisor-suspend' ? 4500 : 2500, 'actual receiver exit');
      await exited;
      checkLocks();
      assert.ok(status[0] !== 0 || status[1], 'fault cannot become completion');
      assert.ok(!fs.existsSync(path.join(root, 'receiver-outcome.json')), 'no authoritative success');
      assert.equal(read(path.join(root, 'requested-membership.json')).requestedFieldSlots, 216);
      results.push({ control, status, quiescentMs, actualGroupAbsent: true, requestedFieldSlots: 216,
        receiverGroup: config.receiverGroup, executionGroup: identity.group, liveAuthority: false });
      save(path.join(root, 'control-result.json'), results.at(-1));
    } finally {
      if (pipe !== undefined) fs.closeSync(pipe);
      if (locker && locker.exitCode === null) { locker.kill('SIGKILL'); await once(locker, 'exit'); }
      // Only an owned unreaped direct child is signalled here, never a stored PGID.
      if (!status) {
        // Let the native receiver cancel/reap a suspended supervisor itself.
        try { await until(() => status, preparation + 11000, 'native failure cleanup'); }
        finally { if (!status) { child.kill('SIGKILL'); await exited; } }
      }
    }
  }
  save(path.join(output, 'results.json'), { controls: results, liveAuthority: false });
  console.log(JSON.stringify({ passed: results.length, output }));
}
if (require.main === module) main(...process.argv.slice(2)).catch(error => { console.error(error); process.exitCode = 1; });
