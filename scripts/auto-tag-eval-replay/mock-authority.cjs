// Explicit native test-broker IPC. No credentials, endpoint discovery or resume.
const fs = require('node:fs');
const path = require('node:path');
const net = require('node:net');
const { spawn } = require('node:child_process');
const { once } = require('node:events');
const { isDeepStrictEqual } = require('node:util');

function validateCompleteReceipt(receipt, expected) {
  return validateReceipt(receipt, expected, 'complete');
}

function validateProvisionalReceipt(receipt, expected) {
  if (!/^[a-f0-9]{64}$/.test(receipt?.resultSha256 ?? '') || receipt.resultSha256 !== expected?.resultSha256
      || !receipt.bindings?.supervision?.generation) throw Error('mock staging digest missing or mismatched');
  return validateReceipt(receipt, expected, 'provisional_complete');
}

function validateReceipt(receipt, expected, state) {
  if (!receipt || !expected || receipt.state !== state || receipt.consumptions !== 1
      || receipt.denied !== 0 || receipt.ownerLive !== false || receipt.liveAuthority !== false
      || !expected.checkpoint || !expected.runId
      || ['runId', 'checkpoint', 'membership', 'bindings', 'counters', 'sessions', 'preparations', 'operations']
        .some(key => !isDeepStrictEqual(receipt[key], expected[key]))
      || Object.keys(receipt.sessions).length !== 12
      || receipt.counters.reserved !== receipt.counters.arrived || receipt.counters.reserved !== receipt.counters.completed
      || Object.values(receipt.operations).some(o => !o.dispatched || !o.arrived || !o.completed)) {
    throw Error('mock final receipt missing, terminal or mismatched; incomplete, never resume');
  }
  return receipt;
}

function rpc(socket, handle, command) {
  return new Promise((resolve, reject) => {
    const stream = net.createConnection(socket);
    let bytes = Buffer.alloc(0), settled = false;
    const finish = (error, value) => {
      if (settled) return;
      settled = true; clearTimeout(timer); stream.destroy();
      if (error) reject(error); else resolve(value);
    };
    const timer = setTimeout(() => finish(Error('mock admission unknown; never retry')), 9000);
    stream.on('connect', () => stream.write(JSON.stringify({ handle, command }) + '\n'));
    stream.on('error', () => finish(Error('mock authority unavailable; never retry')));
    stream.on('end', () => finish(Error('mock admission unknown; never retry')));
    stream.on('data', chunk => {
      bytes = Buffer.concat([bytes, chunk]);
      if (bytes.length > 1048576) return finish(Error('mock IPC ceiling'));
      if (!bytes.includes(10)) return;
      try {
        const result = JSON.parse(bytes.toString());
        if (result.ok !== true) return finish(Error(result.code || 'mock authority denied'));
        finish(null, result.value);
      } catch { finish(Error('mock IPC invalid')); }
    });
  });
}

function lease(socket, handle, role) {
  return new Promise((resolve, reject) => {
    const stream = net.createConnection(socket);
    let bytes = Buffer.alloc(0), ready = false, pending;
    let timer = setTimeout(() => { stream.destroy(); reject(Error('mock lease admission unknown')); }, 9000);
    stream.on('connect', () => stream.write(JSON.stringify({ handle, command: { op: 'lease', role } }) + '\n'));
    const lost = () => {
      clearTimeout(timer);
      if (!ready) reject(Error('mock lease unavailable'));
      if (pending) pending.reject(Error('mock lease settlement unknown'));
    };
    stream.on('error', lost); stream.on('end', lost);
    stream.on('data', chunk => {
      bytes = Buffer.concat([bytes, chunk]);
      if (bytes.length > 65536) { stream.destroy(); lost(); return; }
      if (!bytes.includes(10)) return;
      try {
        const result = JSON.parse(bytes.toString()); bytes = Buffer.alloc(0);
        if (result.ok !== true) throw Error('mock lease denied');
        clearTimeout(timer);
        if (!ready) {
          ready = true;
          resolve({ async release() {
            try {
              await new Promise((resolve, reject) => {
                if (pending) return reject(Error('duplicate mock lease release'));
                pending = { resolve, reject };
                timer = setTimeout(() => reject(Error('mock lease settlement unknown')), 9000);
                stream.write('release\n');
              });
            } finally { clearTimeout(timer); stream.destroy(); }
          }, abandon() { stream.destroy(); } });
        } else if (pending) { pending.resolve(); }
      } catch { stream.destroy(); lost(); }
    });
  });
}

async function start(binary, output, membership, bindings, fault, supervision) {
  const directory = path.join(output, 'authority');
  fs.mkdirSync(directory, { mode: 0o700 });
  const bootstrap = path.join(directory, 'bootstrap.json');
  fs.writeFileSync(bootstrap, JSON.stringify({ membership, bindings, ...(fault ? { fault } : {}) }), { flag: 'wx', mode: 0o600 });
  let child, exited;
  if (!supervision) {
    const log = fs.openSync(path.join(directory, 'process.log'), 'wx');
    child = spawn(binary, ['--exact', 'infra::mock_run_authority::broker::mock_authority_process', '--ignored', '--nocapture'], {
      cwd: process.cwd(), env: { PATH: process.env.PATH, SOUNDROBE_MOCK_AUTHORITY_BOOTSTRAP: bootstrap },
      stdio: ['ignore', log, log],
    });
    fs.closeSync(log);
    exited = once(child, 'exit');
  }
  const ready = path.join(directory, 'ready.json');
  const descriptor = await new Promise((resolve, reject) => {
    const watcher = fs.watch(directory, () => {
      if (fs.existsSync(ready)) finish(null, JSON.parse(fs.readFileSync(ready)));
    });
    const timer = setTimeout(() => finish(Error('mock broker startup unavailable')), 20000);
    const finish = (error, value) => {
      watcher.close(); clearTimeout(timer);
      if (error) reject(error); else resolve(value);
    };
    exited?.then(() => { if (!fs.existsSync(ready)) finish(Error('mock broker exited before readiness')); });
    if (fs.existsSync(ready)) finish(null, JSON.parse(fs.readFileSync(ready)));
  });
  if (descriptor.mode !== 'durable_mock_only') throw Error('mock broker mode mismatch');
  const ownerLease = await lease(descriptor.socket, descriptor.owner, 'owner');
  return { ...descriptor, directory, pid: child?.pid,
    command: command => rpc(descriptor.socket, descriptor.owner, command),
    async settle(failed, expected) {
      if (failed) try { await rpc(descriptor.socket, descriptor.owner, { op: 'stop' }); } catch { /* Unknown is terminal; no retries. */ }
      let releaseError;
      try { await ownerLease.release(); } catch (error) { ownerLease.abandon(); releaseError = error; }
      if (supervision) {
        if (releaseError) throw releaseError;
        const receipt = await rpc(descriptor.socket, descriptor.owner, { op: 'report' });
        return failed ? receipt : validateProvisionalReceipt(receipt, expected);
      }
      let timer;
      let status;
      try {
        status = await Promise.race([exited, new Promise((_, reject) => {
          timer = setTimeout(() => { child.kill('SIGKILL'); reject(Error('mock broker drain unavailable')); }, 12000);
        })]);
      } finally { clearTimeout(timer); }
      const report = path.join(directory, 'final.json');
      const receipt = fs.existsSync(report) ? JSON.parse(fs.readFileSync(report)) : null;
      if (!failed) {
        if (releaseError || status[0] !== 0 || status[1]) throw Error('mock broker settlement unknown; incomplete');
        return validateCompleteReceipt(receipt, expected);
      }
      return receipt;
    },
  };
}
module.exports = { rpc, lease, start, validateCompleteReceipt, validateProvisionalReceipt };
