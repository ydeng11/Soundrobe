// Explicit native test-broker IPC. No credentials, endpoint discovery or resume.
const fs = require('./bounded-storage.cjs');
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
  const hosted = receipt?.testPolicy === 'disabled-hosted-boundary-v1;native-file;native-response-observation;no-live;no-retry;no-repair'
    && receipt.bindings?.modelRehearsal?.offlineFixture === true;
  const binding = receipt?.bindings?.modelRehearsal;
  const plan = receipt?.bindings?.modelRehearsalPlan;
  if (Object.hasOwn(plan ?? {}, 'nativePrepared')) throw Error('native prepared runs are not accepted by the mock receipt validator');
  const http = Object.hasOwn(binding ?? {}, 'offlineHttp') || Object.hasOwn(plan ?? {}, 'offlineHttp');
  if (http && (!hosted || binding.offlineHttp !== true || plan?.offlineHttp !== true
      || !plan.offlineHosted || typeof plan.offlineHosted !== 'object' || Array.isArray(plan.offlineHosted))) {
    throw Error('offline HTTP mode missing or mismatched');
  }
  const operations = Object.values(receipt?.operations ?? {});
  const native = hosted ? operations.filter(o => o.category === 'model' && o.arrived === false
    && o.response?.observation === (http ? 'native_http_response' : 'offline_injected_native_response')
    && (http ? o.response.http?.transport === 'offline_local_http' && o.response.http.version === 1 : !Object.hasOwn(o.response, 'http'))
    && o.response.remoteArrivalVerified === false) : [];
  if (hosted && (native.length !== 1 || operations.filter(o => o.category === 'model').length !== 1
      || operations.length !== receipt.counters?.reserved || receipt.counters?.model !== 1)) {
    throw Error('offline native observation missing or ambiguous');
  }
  if (!receipt || !expected || receipt.state !== state || receipt.consumptions !== 1
      || receipt.denied !== 0 || receipt.ownerLive !== false || receipt.liveAuthority !== false
      || !expected.checkpoint || !expected.runId
      || ['runId', 'checkpoint', 'membership', 'bindings', 'counters', 'sessions', 'preparations', 'operations']
        .some(key => !isDeepStrictEqual(receipt[key], expected[key]))
      || Object.keys(receipt.sessions).length !== 12
      || receipt.counters.reserved !== receipt.counters.arrived + native.length || receipt.counters.reserved !== receipt.counters.completed
      || operations.some(o => !o.dispatched || (!o.arrived && !native.includes(o)) || !o.completed)) {
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
// Explicit offline signing fixture: RFC 8032 public test-vector seed. This
// authenticates test plumbing only and MUST NOT be provisioned as user trust.
function testApproval(report) {
  const crypto = require('node:crypto');
  const canonical = value => value === null || typeof value !== 'object' ? JSON.stringify(value)
    : Array.isArray(value) ? '[' + value.map(canonical).join(',') + ']'
      : '{' + Object.keys(value).sort().map(k => JSON.stringify(k) + ':' + canonical(value[k])).join(',') + '}';
  const policy = report.testPolicy ?? 'offline-retained-evidence-v1;loopback-scripted-genre-v1;no-live;no-retry;no-repair';
  const rehearsal = policy === 'offline-model-rehearsal-v1;separate-verifier;fake-credential;loopback-only;no-live;no-retry;no-repair';
  const hosted = policy === 'disabled-hosted-boundary-v1;native-file;native-response-observation;no-live;no-retry;no-repair';
  if (Object.hasOwn(report.bindings?.modelRehearsalPlan ?? {}, 'nativePrepared')) throw Error('native prepared approval cannot use the public signer');
  if (hosted && report.bindings?.modelRehearsal?.offlineFixture !== true) throw Error('hosted public signer requires explicit fixture trust');
  if (!rehearsal && !hosted && policy !== 'offline-retained-evidence-v1;loopback-scripted-genre-v1;no-live;no-retry;no-repair') throw Error('Unknown offline signing policy');
  const scope = { runId: report.runId, checkpoint: report.checkpoint, bindings: report.bindings,
    membership: report.membership, preparations: report.preparations, policy };
  const payload = { version: 1, runId: report.runId, checkpoint: report.checkpoint,
    scopeSha256: crypto.createHash('sha256').update(canonical(scope)).digest('hex'),
    nonce: crypto.randomUUID(), expiresUnixSeconds: Math.floor(Date.now() / 1000) + 1200 };
  const key = crypto.createPrivateKey({ format: 'der', type: 'pkcs8', key: Buffer.from(
    '302e020100300506032b657004220420' + (hosted ? '2b'.repeat(32) : rehearsal ? '2a'.repeat(32) : '9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60'), 'hex') });
  return { payload, signature: [...crypto.sign(null, Buffer.from(canonical(payload)), key)] };
}
async function approveFrozen(authority, report, retain) {
  const plan = report?.bindings?.modelRehearsalPlan;
  if (!Object.hasOwn(plan ?? {}, 'nativePrepared')) {
    const approval = testApproval(report);
    await retain('test-fixture-approval.json', approval);
    return authority.command({ op: 'approve', approval });
  }
  const frozen = structuredClone(report);
  const binding = frozen.bindings?.modelRehearsal, boundary = binding?.boundary;
  const policy = 'disabled-hosted-boundary-v1;native-file;native-response-observation;no-live;no-retry;no-repair';
  const digest = value => typeof value === 'string' && /^[a-f0-9]{64}$/.test(value);
  const noModelActivity = value => value.credentialLookups === 0 && value.actualFakeLookups === 0
    && value.counters?.model === 0 && value.operations && typeof value.operations === 'object'
    && !Array.isArray(value.operations) && Object.values(value.operations).every(o => o?.category !== 'model');
  if (plan.nativePrepared !== true || frozen.state !== 'frozen' || frozen.consumptions !== 0
      || typeof frozen.runId !== 'string' || !frozen.runId || !digest(frozen.checkpoint)
      || !frozen.membership || !frozen.preparations || !noModelActivity(frozen)
      || frozen.testPolicy !== policy || binding?.policy !== policy
      || frozen.liveAuthority !== false || binding.liveAuthority !== false
      || ['offlineFixture', 'offlineHosted', 'offlineHttp', 'offlineResponse'].some(key =>
        Object.hasOwn(plan, key) || Object.hasOwn(binding, key))
      || boundary?.allowanceUsdCents !== 500
      || ['hardBillingCap', 'billingRiskAccepted', 'liveDispatchAvailable', 'readyForExecution']
        .some(key => boundary?.[key] !== false)) throw Error('native approval coordination scope invalid');
  let deadline = Infinity;
  const scope = async () => {
    const value = await authority.command({ op: 'scope' });
    if (!Number.isSafeInteger(value?.remainingMs) || value.remainingMs <= 0) throw Error('native approval scope unavailable or exhausted');
    deadline = Math.min(deadline, performance.now() + value.remainingMs);
    const remaining = deadline - performance.now();
    if (remaining <= 0) throw Error('native approval scope exhausted');
    return remaining;
  };
  let status;
  for (;;) {
    await scope();
    status = await authority.command({ op: 'owner_approval_status' });
    const keys = status?.state === 'pending' ? ['checkpoint', 'liveAuthority', 'state']
      : ['approvalSha256', 'checkpoint', 'liveAuthority', 'state'];
    if (!status || !['pending', 'ready'].includes(status.state)
        || !isDeepStrictEqual(Object.keys(status).sort(), keys)
        || status.checkpoint !== frozen.checkpoint || status.liveAuthority !== false
        || (status.state === 'ready' && !digest(status.approvalSha256))) throw Error('native owner approval status invalid or drifted');
    if (status.state === 'ready') break;
    const remaining = deadline - performance.now();
    if (remaining <= 0) throw Error('native approval scope exhausted');
    await new Promise(resolve => setTimeout(resolve, Math.min(25, remaining)));
  }
  await scope();
  const consumed = await authority.command({ op: 'approve_owner_file' });
  const current = await authority.command({ op: 'report' });
  if (consumed?.consumed !== true || !Number.isSafeInteger(consumed.expiresUnixSeconds)
      || consumed.expiresUnixSeconds <= Math.floor(Date.now() / 1000)
      || !current || current.state !== 'executing' || current.consumptions !== 1
      || current.liveAuthority !== false || current.testPolicy !== frozen.testPolicy
      || !noModelActivity(current) || current.approvalSha256 !== status.approvalSha256
      || current.approvalExpires !== consumed.expiresUnixSeconds
      || ['runId', 'checkpoint', 'bindings', 'membership', 'preparations'].some(key =>
        !isDeepStrictEqual(current[key], frozen[key]))) throw Error('native approval coordination receipt mismatched');
  const receipt = { componentCoordinationOnly: true, liveAuthority: false, humanPresenceVerified: false,
    qualityCertificate: false, successCertificate: false,
    ...Object.fromEntries(['runId', 'checkpoint', 'bindings', 'membership', 'preparations', 'approvalSha256',
      'approvalExpires', 'consumptions', 'state', 'credentialLookups', 'actualFakeLookups', 'counters']
      .map(key => [key, current[key]])) };
  await retain('owner-approval-coordination.json', receipt);
  return receipt;
}
module.exports = { rpc, lease, start, validateCompleteReceipt, validateProvisionalReceipt, testApproval, approveFrozen };
