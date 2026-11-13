// Offline process/wire controls. Inputs and all outputs stay in local artifacts.
const fs = require('node:fs'), path = require('node:path'), crypto = require('node:crypto');
const assert = require('node:assert/strict');
const http = require('node:http');
const { spawn, fork, execFileSync } = require('node:child_process');
const { once } = require('node:events');
const { start, rpc, lease, validateCompleteReceipt } = require('./mock-authority.cjs');
const { failureEnvelope } = require('./run-bounded.cjs');
const sha = bytes => crypto.createHash('sha256').update(bytes).digest('hex');
const save = (p, value) => fs.writeFileSync(p, JSON.stringify(value, null, 2) + '\n', { flag: 'wx' });
const read = p => JSON.parse(fs.readFileSync(p));
const body = Buffer.from('{"model":"scripted/genre-v1","control":true}');

function recovered(directory) {
  return JSON.parse(execFileSync('python3', ['-B', '-c',
    `import sqlite3,json,sys,hashlib
c=sqlite3.connect("file:"+sys.argv[1]+"?mode=ro",uri=True,timeout=.1)
c.execute("BEGIN")
row=c.execute("SELECT state FROM authority WHERE id=1").fetchone()
assert row is not None, "initialization incomplete"
state=json.loads(row[0])
if state["bindings"] is None:
    binding=c.execute("SELECT value FROM authority_bindings WHERE id=1").fetchone()
    assert binding is not None, "immutable bindings unavailable"
    assert hashlib.sha256(binding[0].encode()).hexdigest()==state["bindings_sha256"], "immutable bindings corrupt"
    state["bindings"]=json.loads(binding[0])
else:
    assert state.get("bindings_sha256") is None, "mixed binding representations"
print(json.dumps(state))`,
    path.join(directory, 'authority/registry.sqlite')], { encoding: 'utf8', timeout: 3000 }));
}
async function child(input) {
  const value = read(input);
  process.send('ready'); await once(process, 'message');
  let result;
  try {
    result = value.command ? await rpc(value.socket, value.handle, value.command)
      : { status: (await fetch(value.url, { method: value.model ? 'POST' : 'GET',
        headers: { 'x-soundrobe-mock-session': value.handle },
        ...(value.model ? { body } : {}), signal: AbortSignal.timeout(10000) })).status };
    save(value.output, { ok: true, result });
  } catch { save(value.output, { ok: false }); }
  process.disconnect();
}
async function race(directory, values) {
  const children = await Promise.all(values.map(async (value, n) => {
    const input = path.join(directory, `child-${n}.json`), output = path.join(directory, `child-${n}-result.json`);
    save(input, { ...value, output });
    const log = fs.openSync(path.join(directory, `child-${n}.log`), 'wx');
    const process = fork(__filename, ['--child', input], { env: { PATH: global.process.env.PATH }, stdio: ['ignore', log, log, 'ipc'] });
    fs.closeSync(log);
    const exit = once(process, 'exit');
    assert.equal((await once(process, 'message'))[0], 'ready');
    return { process, exit, output };
  }));
  for (const c of children) c.process.send('go');
  await Promise.all(children.map(c => c.exit));
  return children.map(c => read(c.output));
}
async function main(output, binary, membershipFile, only) {
  output = path.resolve(output); binary = path.resolve(binary);
  const anchor = fs.realpathSync('.amp/in/artifacts');
  assert.ok(output.startsWith(anchor + path.sep));
  fs.mkdirSync(output);
  const membership = read(membershipFile), executable = sha(fs.readFileSync(binary));
  assert.equal(membership.requestedFieldSlots, 216);
  const locks = [binary, membershipFile, __filename, require.resolve('./mock-authority.cjs'), require.resolve('./run-bounded.cjs'),
    require.resolve('../mock-provider-service.cjs'), ...execFileSync('rg', ['--files', 'src-tauri/src'], { encoding: 'utf8' }).trim().split('\n')]
    .map(file => ({ path: path.relative(process.cwd(), file), sha256: sha(fs.readFileSync(file)) }));
  save(path.join(output, 'source-and-tool-locks.json'), locks);
  const checkLocks = () => { for (const lock of locks) assert.equal(sha(fs.readFileSync(lock.path)), lock.sha256, `changed frozen control source: ${lock.path}`); };
  const results = [];
  async function run(name, action, fault) {
    if (only && only !== name) return;
    checkLocks();
    const directory = path.join(output, name); fs.mkdirSync(directory);
    save(path.join(directory, 'requested-membership.json'), membership);
    const authority = await start(binary, directory, membership, { executable, sourcesAndTools: locks }, fault);
    const wires = [], servers = [], sessions = [];
    let wireAction;
    async function register(job, stage, maintained = false) {
      let service;
      const handler = async (request, response) => {
        const chunks = [];
        for await (const chunk of request) chunks.push(chunk);
        const bytes = Buffer.concat(chunks), operation = request.headers['x-soundrobe-mock-operation'];
        wires.push({ operation, method: request.method, path: request.url, bodySha256: sha(bytes) });
        if (wireAction === 'kill_before_arrival') { process.kill(authority.pid, 'SIGKILL'); response.destroy(); return; }
        try {
          await rpc(authority.socket, wireAction?.service ?? service, { op: 'arrival', operation,
            request_sha256: sha(Buffer.from(JSON.stringify([request.method, request.url, sha(bytes)]))) });
          if (wireAction === 'kill_after_arrival') { process.kill(authority.pid, 'SIGKILL'); response.destroy(); return; }
          response.end('{}');
        } catch { response.writeHead(409); response.end('{}'); }
      };
      let fixtureLease;
      const fixtureAuthority = { socket: authority.socket, onFatal: () => fixtureLease?.abandon() };
      const manifest = path.join(directory, `${job}-${stage}-service.json`);
      if (maintained) save(manifest, { schemaVersion: 1, records: [] });
      const server = maintained ? require('../mock-provider-service.cjs').createService(manifest, fixtureAuthority) : http.createServer(handler);
      server.listen(0, '127.0.0.1'); await once(server, 'listening'); servers.push(server);
      const session = await authority.command({ op: 'register', job, stage,
        endpoint: `http://127.0.0.1:${server.address().port}`, manifest_sha256: sha(Buffer.from('fixed control fixture')) });
      service = session.service;
      fixtureAuthority.service = service;
      fixtureLease = await lease(authority.socket, service, 'fixture');
      const nativeLease = await lease(authority.socket, session.client, 'native');
      sessions.push({ nativeLease, fixtureLease });
      const command = command => rpc(authority.socket, session.client, command);
      await command({ op: 'attach', executable_sha256: executable,
        preparation_sha256: stage === 'execution' ? sha(Buffer.from(job)) : null });
      if (stage === 'preparation') await command({ op: 'seal_inputs' });
      return { ...session, command, fixtureLease, nativeLease, server, fixtureAuthority, async finish() {
        await command(stage === 'preparation' ? { op: 'prepared', preparation_sha256: sha(Buffer.from(job)),
          request_sha256: job === 'assistance-crowd' ? sha(body) : null }
          : { op: 'finish', observation_sha256: sha(Buffer.from('control observation')) });
        await nativeLease.release(); await command({ op: 'close' });
        server.closeAllConnections(); await new Promise(resolve => server.close(resolve)); await fixtureLease.release();
      } };
    }
    async function prepare(providerWire = false) {
      for (const row of membership.evaluations) {
        const session = await register(row.name, 'preparation');
        if (providerWire && row.name === 'disabled-crowd') assert.equal(await request(session), 200);
        await session.finish();
      }
      return (await authority.command({ op: 'freeze' })).checkpoint;
    }
    async function request(session, model = false) {
      return (await fetch(authority.gateway + (model ? '/model/chat/completions' : '/musicbrainz/ws/2/release/control'), {
        method: model ? 'POST' : 'GET', headers: { 'x-soundrobe-mock-session': session.client },
        ...(model ? { body } : {}), signal: AbortSignal.timeout(10000),
      })).status;
    }
    try {
      await action({ authority, directory, register, prepare, request, wires,
        setWireAction: action => { wireAction = action; } });
      try { await authority.settle(true); } catch { /* Crash controls intentionally have no final broker reply. */ }
      const stored = recovered(directory);
      const operations = Object.values(stored.operations);
      if (fault) {
        assert.equal(operations.length, fault === 'before_reservation_commit' ? 0 : 1);
        assert.equal(operations.filter(o => o.dispatched).length, fault === 'after_dispatch_admission' ? 1 : 0);
        assert.equal(operations.filter(o => o.arrived || o.completed).length, 0);
      }
      if (name === 'kill_before_arrival' || name === 'kill_after_arrival') {
        assert.equal(operations.length, 1); assert.equal(operations[0].dispatched, true);
        assert.equal(operations[0].arrived, name === 'kill_after_arrival'); assert.equal(operations[0].completed, false);
      }
      const registry = path.join(directory, 'authority/registry.sqlite');
      const beforeRestart = sha(fs.readFileSync(registry));
      const restartLog = fs.openSync(path.join(directory, 'restart-denied.log'), 'wx');
      const restart = spawn(binary, ['--exact', 'infra::mock_run_authority::broker::mock_authority_process', '--ignored', '--nocapture'], {
        env: { PATH: process.env.PATH, SOUNDROBE_MOCK_AUTHORITY_BOOTSTRAP: path.join(directory, 'authority/bootstrap.json') }, stdio: ['ignore', restartLog, restartLog],
      });
      fs.closeSync(restartLog);
      assert.equal((await once(restart, 'exit'))[0], 101, 'restarted an existing identity');
      assert.equal(sha(fs.readFileSync(registry)), beforeRestart, 'restart modified the journal');
      save(path.join(directory, 'recovered-read-only.json'), stored);
      const envelope = failureEnvelope(directory);
      assert.equal(envelope.requestedFieldSlots, 216); assert.equal(envelope.serviceFailureSlots, 216);
      save(path.join(directory, 'failure-envelope.json'), envelope);
      save(path.join(directory, 'wire-arrivals.json'), wires);
      checkLocks();
      results.push({ name, wires: wires.length, operations: Object.keys(stored.operations).length,
        modelReservations: Object.values(stored.operations).filter(o => o.category === 'model').length,
        storedState: stored.state, requestedFieldSlots: 216, failureSlots: 216,
        recovery: 'read_only_incomplete_no_resume', remainingAllowance: 0 });
    } finally {
      for (const session of sessions) { session.nativeLease.abandon(); session.fixtureLease.abandon(); }
      for (const server of servers) { server.closeAllConnections(); server.close(); }
    }
  }
  await run('simultaneous-consumption', async ({ authority, directory, prepare, wires }) => {
    const checkpoint = await prepare();
    const outcomes = await race(directory, [0, 1].map(() => ({ socket: authority.socket, handle: authority.owner,
      command: { op: 'consume', fake_checkpoint_sha256: checkpoint } })));
    assert.equal(outcomes.filter(o => o.ok).length, 1); assert.equal(wires.length, 0);
    assert.equal((await authority.command({ op: 'report' })).consumptions, 1);
  });
  await run('competing-model-children', async ({ authority, directory, prepare, register, wires }) => {
    const checkpoint = await prepare(); await authority.command({ op: 'consume', fake_checkpoint_sha256: checkpoint });
    const session = await register('assistance-crowd', 'execution');
    await race(directory, [0, 1].map(() => ({ url: authority.gateway + '/model/chat/completions', handle: session.client, model: true })));
    assert.ok(wires.length <= 1);
    const report = await authority.command({ op: 'report' });
    assert.equal(report.counters.model, 1); assert.equal(report.state, 'terminal');
  });
  await run('replacement-no-reset-and-stale-handle', async ({ authority, prepare, register, request, wires }) => {
    const checkpoint = await prepare(true); await authority.command({ op: 'consume', fake_checkpoint_sha256: checkpoint });
    const session = await register('disabled-crowd', 'execution');
    assert.equal(await request(session), 200); await session.finish();
    assert.equal(await request(session), 409); assert.equal(wires.length, 2);
    assert.equal((await authority.command({ op: 'report' })).counters.provider, 2);
  });
  await run('forged-handle-stops-subsequent-work', async ({ register, request, wires }) => {
    const session = await register('disabled-crowd', 'preparation');
    assert.equal(await request({ client: '00000000-0000-0000-0000-000000000000' }), 409);
    assert.equal(await request(session), 409); assert.equal(wires.length, 0);
  });
  await run('fixture-disconnect-revokes', async ({ authority, register, request, wires }) => {
    const session = await register('disabled-crowd', 'preparation'); session.fixtureLease.abandon();
    const deadline = Date.now() + 2000;
    while ((await authority.command({ op: 'report' })).state !== 'terminal') assert.ok(Date.now() < deadline, 'fixture disconnect not observed');
    assert.equal(await request(session), 409); assert.equal(wires.length, 0);
    assert.equal((await authority.command({ op: 'report' })).state, 'terminal');
  });
  await run('orphan-fixture-ingress-is-terminal', async ({ authority, register, request, wires }) => {
    const session = await register('disabled-crowd', 'preparation', true);
    const response = await fetch(`http://127.0.0.1:${session.server.address().port}/__fixtures/compact`);
    assert.equal(response.status, 409);
    assert.equal((await authority.command({ op: 'report' })).state, 'terminal', 'orphan ingress did not stop authority');
    assert.equal(await request(session), 409);
    const snapshot = JSON.parse(session.server.compactInventory());
    assert.equal(snapshot.wireArrivals.length, 1); wires.push(...snapshot.wireArrivals);
  });
  await run('rebound-service-cannot-acknowledge-another-job', async ({ authority, register, request, wires, setWireAction }) => {
    const a = await register('disabled-crowd', 'preparation');
    const b = await register('disabled-billie', 'preparation');
    setWireAction({ service: b.service });
    assert.equal(await request(a), 409); assert.equal(await request(b), 409);
    assert.equal(wires.length, 1);
    const report = await authority.command({ op: 'report' });
    assert.equal(report.counters.reserved, 1); assert.equal(report.counters.arrived, 0);
  });
  await run('failed-ingress-notification-revokes-fixture-lease', async ({ authority, register, request, wires }) => {
    const session = await register('disabled-crowd', 'preparation', true);
    session.fixtureAuthority.socket = path.join(authority.directory, 'never-listened.sock');
    assert.equal((await fetch(`http://127.0.0.1:${session.server.address().port}`)).status, 409);
    session.fixtureAuthority.socket = authority.socket;
    const deadline = Date.now() + 2000;
    while ((await authority.command({ op: 'report' })).state !== 'terminal') assert.ok(Date.now() < deadline, 'notification failure did not revoke fixture');
    assert.equal(await request(session), 409);
    const snapshot = JSON.parse(session.server.compactInventory());
    assert.equal(snapshot.wireArrivals.length, 1); wires.push(...snapshot.wireArrivals);
  });
  for (const reason of ['connect', 'oversized', 'aborted']) {
    await run(`fixture-${reason}-is-terminal`, async ({ authority, register, request, wires }) => {
      const session = await register('disabled-crowd', 'preparation', true);
      const url = `http://127.0.0.1:${session.server.address().port}`;
      if (reason === 'connect') {
        assert.equal(await new Promise(resolve => {
          const req = http.request(url, { method: 'CONNECT', path: 'no-upstream.invalid:443' });
          req.on('connect', (response, socket) => { socket.destroy(); resolve(response.statusCode); });
          req.on('error', () => resolve(0)); req.end();
        }), 403);
      } else if (reason === 'oversized') {
        await assert.rejects(fetch(url, { method: 'POST', body: Buffer.alloc(65537), signal: AbortSignal.timeout(10000) }));
      } else {
        const req = http.request(url, { method: 'POST', headers: { 'content-length': '32' } });
        req.on('error', () => {}); req.write('partial');
        const deadline = Date.now() + 2000;
        while (!JSON.parse(session.server.compactInventory()).wireArrivals.length) {
          assert.ok(Date.now() < deadline, 'partial request never reached fixture'); await new Promise(resolve => setImmediate(resolve));
        }
        req.destroy();
      }
      const deadline = Date.now() + 2000;
      while ((await authority.command({ op: 'report' })).state !== 'terminal') assert.ok(Date.now() < deadline, 'ingress denial not terminal');
      assert.equal(await request(session), 409);
      const report = await authority.command({ op: 'report' });
      assert.equal(report.counters.reserved, 0); assert.equal(report.ingressDenials[0].reason, reason);
      const snapshot = JSON.parse(session.server.compactInventory());
      assert.equal(snapshot.wireArrivals.length, 1); wires.push(...snapshot.wireArrivals);
    });
  }
  await run('storage-lock-denies-without-refund-or-resume', async ({ directory, register, request, wires }) => {
    const session = await register('disabled-crowd', 'preparation');
    const holder = spawn('python3', ['-B', '-c',
      'import sqlite3,sys,os; c=sqlite3.connect(sys.argv[1],isolation_level=None); c.execute("BEGIN EXCLUSIVE"); c.execute("UPDATE authority SET state=state"); print("locked",flush=True); sys.stdin.read(1); os._exit(86)',
      path.join(directory, 'authority/registry.sqlite')], { stdio: ['pipe', 'pipe', 'pipe'] });
    const exited = once(holder, 'exit');
    assert.equal((await once(holder.stdout, 'data'))[0].toString().trim(), 'locked');
    const outcomes = await Promise.all([0, 1].map(async () => { try { return await request(session); } catch { return 'unknown'; } }));
    assert.ok(outcomes.every(s => s !== 200));
    holder.stdin.end('x'); assert.equal((await exited)[0], 86);
    let after;
    try { after = await request(session); } catch { after = 'unknown'; }
    assert.notEqual(after, 200); assert.equal(wires.length, 0);
  });
  await run('storage-unavailable-denies-without-new-registry', async ({ directory, register, request, wires }) => {
    const session = await register('disabled-crowd', 'preparation');
    const registry = path.join(directory, 'authority/registry.sqlite');
    fs.renameSync(registry, registry + '.unavailable');
    let outcome;
    try { try { outcome = await request(session); } catch { outcome = 'unknown'; }
      assert.equal(fs.existsSync(registry), false, 'silently created another registry');
    } finally { fs.renameSync(registry + '.unavailable', registry); }
    assert.notEqual(outcome, 200);
    try { outcome = await request(session); } catch { outcome = 'unknown'; }
    assert.notEqual(outcome, 200); assert.equal(wires.length, 0);
  });
  await run('post-publish-crash-remains-incomplete', async ({ authority, prepare, register }) => {
    const checkpoint = await prepare(); await authority.command({ op: 'consume', fake_checkpoint_sha256: checkpoint });
    for (const row of membership.evaluations) await (await register(row.name, 'execution')).finish();
    const expected = await authority.command({ op: 'report' });
    await authority.command({ op: 'publish' });
    process.kill(authority.pid, 'SIGKILL');
    await assert.rejects(authority.settle(false, expected), 'missing final receipt was accepted');
  });
  await run('zero-call-completion-and-final-receipt-controls', async ({ authority, prepare, register, wires }) => {
    const checkpoint = await prepare(); await authority.command({ op: 'consume', fake_checkpoint_sha256: checkpoint });
    for (const row of membership.evaluations) await (await register(row.name, 'execution')).finish();
    const expected = await authority.command({ op: 'report' });
    await authority.command({ op: 'publish' });
    const receipt = await authority.settle(false, expected);
    assert.equal(receipt.state, 'complete'); assert.equal(wires.length, 0);
    assert.throws(() => validateCompleteReceipt(null, expected));
    for (const key of ['state', 'runId', 'checkpoint', 'membership', 'bindings', 'counters', 'sessions', 'preparations', 'operations', 'consumptions', 'ownerLive']) {
      assert.throws(() => validateCompleteReceipt({ ...receipt, [key]: null }, expected), `accepted changed ${key}`);
    }
  });
  for (const fault of ['before_reservation_commit', 'after_reservation_commit', 'after_dispatch_admission']) {
    await run(fault, async ({ register, request, wires }) => {
      const session = await register('disabled-crowd', 'preparation');
      await assert.rejects(request(session)); assert.equal(wires.length, 0);
    }, fault);
  }
  for (const action of ['kill_before_arrival', 'kill_after_arrival']) {
    await run(action, async ({ register, request, wires, setWireAction }) => {
      const session = await register('disabled-crowd', 'preparation'); setWireAction(action);
      await assert.rejects(request(session)); assert.equal(wires.length, 1);
    });
  }
  save(path.join(output, 'results.json'), { executableSha256: executable, results,
    powerLossOrPhysicalFlushProved: false, liveAuthority: false });
  console.log(JSON.stringify(results));
}
module.exports = { recovered };
if (require.main === module) {
  if (process.argv[2] === '--child') child(process.argv[3]).catch(error => { console.error(error); process.exitCode = 1; });
  else main(...process.argv.slice(2)).catch(error => { console.error(error); process.exitCode = 1; });
}
