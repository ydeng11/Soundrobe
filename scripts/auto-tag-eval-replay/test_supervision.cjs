const { test } = require('node:test');
const assert = require('node:assert/strict');
const { validateProvisionalReceipt } = require('./mock-authority.cjs');

const receipt = () => ({ state: 'provisional_complete', consumptions: 1, denied: 0,
  ownerLive: false, liveAuthority: false, runId: 'fresh-run', checkpoint: 'frozen',
  membership: { requestedFieldSlots: 216 }, bindings: { supervision: { generation: 'fresh-generation' } },
  counters: { reserved: 1, arrived: 1, completed: 1 }, preparations: {},
  resultSha256: 'a'.repeat(64),
  sessions: Object.fromEntries(Array.from({ length: 12 }, (_, i) => [String(i), { closed: true }])),
  operations: { one: { dispatched: true, arrived: true, completed: true } } });

test('explicitly released staging is valid evidence, never complete authority', () => {
  const value = receipt();
  assert.doesNotThrow(() => validateProvisionalReceipt(value, structuredClone(value)));
  for (const mutate of [r => r.state = 'complete', r => r.ownerLive = true,
    r => r.membership.requestedFieldSlots--, r => r.resultSha256 = 'b'.repeat(64),
    r => r.operations.one.completed = false, r => r.bindings.supervision.generation = 'other']) {
    const changed = structuredClone(value); mutate(changed);
    assert.throws(() => validateProvisionalReceipt(changed, value));
  }
});

test('recovery records observed receiver success, never staged files or eventual supervisor exit', () => {
  const { validateOutcome } = require('./run-supervised.cjs');
  const config = { generation: 'fresh', receiverPid: 123, receiverGroup: 321 };
  const status = { pid: 123, status: 0, signal: null };
  const value = { state: 'complete', authority: 'native_witness_and_timely_supervisor_exit',
    generation: 'fresh', receiverPid: 123, receiverGroup: 321, supervisorGroup: 987,
    executionGroup: 789, supervisorExitObservedAt: 190, acceptedAt: 191, deadline: 200,
    requestedFieldSlots: 216, liveAuthority: false, controlOnly: false,
    evidenceSha256: 'a'.repeat(64), resultSha256: 'b'.repeat(64) };
  assert.doesNotThrow(() => validateOutcome(status, config, value));
  for (const mutate of [r => r.state = 'provisional_complete', r => r.generation = 'old',
    r => r.acceptedAt = 200, r => r.supervisorExitObservedAt = 201,
    r => r.supervisorExitObservedAt = null, r => r.executionGroup = 321,
    r => r.resultSha256 = null, r => r.controlOnly = true, r => delete r.controlOnly]) {
    const changed = structuredClone(value); mutate(changed);
    assert.throws(() => validateOutcome(status, config, changed));
  }
  for (const change of [{ status: 78 }, { signal: 'SIGKILL' }, { pid: 124 }, { error: Error('unknown') }]) {
    assert.throws(() => validateOutcome({ ...status, ...change }, config, value));
  }
});

test('launcher returns incomplete without waiting for inherited descendant pipes', async () => {
  const fs = require('node:fs'), path = require('node:path');
  const { spawn } = require('node:child_process');
  const { once } = require('node:events');
  const root = fs.mkdtempSync(path.resolve('.amp/in/launcher-pipe-'));
  const executable = path.join(root, 'fixture.cjs');
  // Finite disposable descendant: no cleanup signal or stored PID is needed.
  const exiting = path.join(root, 'exiting.json');
  fs.writeFileSync(executable, `#!${process.execPath}\nrequire('node:child_process').spawn(process.execPath,
    ['-e', 'setTimeout(() => {}, 2500)'], {stdio: 'inherit'});
    require('node:fs').writeFileSync(${JSON.stringify(exiting)}, String(Date.now())); process.exit(78);\n`, { mode: 0o700 });
  const child = spawn(process.execPath, [require.resolve('./run-supervised.cjs'), path.join(root, 'output'), executable],
    { env: { PATH: process.env.PATH }, stdio: 'ignore' });
  const [status] = await once(child, 'exit');
  const elapsed = Date.now() - Number(fs.readFileSync(exiting, 'utf8'));
  try {
    assert.equal(status, 1);
    assert.ok(elapsed < 1500, `descendant pipe delayed incomplete return: ${elapsed}ms`);
  } finally {
    fs.rmSync(root, { recursive: true });
  }
});
