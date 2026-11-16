// Launches native mock ownership before hashing, fixtures or evaluator setup.
const fs = require('node:fs');
const path = require('node:path');
const crypto = require('node:crypto');
const { spawnSync } = require('node:child_process');

function validateOutcome(status, config, receipt) {
  if (status.error || status.status !== 0 || status.signal || status.pid !== config.receiverPid
      || receipt?.state !== 'complete' || receipt.authority !== 'native_witness_and_timely_supervisor_exit'
      || receipt.generation !== config.generation || receipt.receiverPid !== status.pid
      || receipt.receiverGroup !== config.receiverGroup || receipt.liveAuthority !== false
      || receipt.requestedFieldSlots !== 216 || receipt.controlOnly !== false
      || !['supervisorExitObservedAt', 'acceptedAt', 'deadline', 'executionGroup', 'supervisorGroup', 'receiverGroup']
        .every(k => Number.isSafeInteger(receipt[k]) && receipt[k] > 0)
      || receipt.supervisorExitObservedAt > receipt.acceptedAt || receipt.acceptedAt >= receipt.deadline
      || receipt.executionGroup === receipt.receiverGroup || receipt.executionGroup === receipt.supervisorGroup
      || ![receipt.resultSha256, receipt.evidenceSha256].every(s => /^[a-f0-9]{64}$/.test(s ?? ''))) {
    throw Error('native completion unavailable; staging is not recoverable success');
  }
  return receipt;
}

function readSmall(file) {
  if (fs.statSync(file).size > 65536) throw Error('native outcome byte ceiling');
  return fs.readFileSync(file);
}

function main(output, executable, rehearsalPlan) {
  if (!output || !executable || ![4, 5].includes(process.argv.length)) throw Error('Usage: run-supervised.cjs NEW_OUTPUT TEST_EXECUTABLE [PUBLIC_REHEARSAL_PLAN]');
  output = path.resolve(output);
  // No source/input/config reads precede native ownership and its clock.
  const status = spawnSync(path.resolve(executable), ['--exact',
    'infra::mock_run_authority::supervisor::ownership::native_supervision_process', '--ignored', '--nocapture'], {
    cwd: process.cwd(), env: { PATH: process.env.PATH, SOUNDROBE_NATIVE_ROLE: 'receiver', SOUNDROBE_NATIVE_OUTPUT: output,
      ...(rehearsalPlan ? { SOUNDROBE_MODEL_REHEARSAL_PLAN: path.resolve(rehearsalPlan) } : {}) },
    // Descendants must not own a pipe whose EOF gates launcher completion.
    // Native exit and the receiver outcome, never captured output, decide it.
    stdio: 'ignore',
  });
  const outcome = archiveReceiverOutcome(output, executable, status);
  if (outcome.state !== 'complete') process.exitCode = 1;
  console.log(JSON.stringify(outcome));
}

// Internal launcher helper: main supplies spawnSync's observed exit, never argv.
// The native owner archives its own waited Child directly in Rust.
function archiveReceiverOutcome(output, executable, observedStatus) {
  output = path.resolve(output);
  const status = observedStatus;
  let outcome = { state: 'incomplete', requestedFieldSlots: 216, liveAuthority: false,
    receiverExit: status.status, receiverSignal: status.signal, recovery: 'no_staging_promotion_no_resume',
    validatedFieldSlots: 0, unavailableFieldSlots: 216, interruptedEffects: 'unknown' };
  try {
    if (status.error || status.status !== 0 || status.signal) throw Error('native receiver unsuccessful');
    const config = JSON.parse(readSmall(path.join(output, 'native-supervision.json')));
    const bytes = readSmall(path.join(output, 'receiver-outcome.json'));
    const accepted = validateOutcome(status, config, JSON.parse(bytes));
    outcome = { ...outcome, state: 'complete', generation: accepted.generation, receiverPid: status.pid,
      receiverOutcomeSha256: crypto.createHash('sha256').update(bytes).digest('hex'),
      authority: accepted.authority, actualReceiverExitObserved: true,
      validatedFieldSlots: 216, unavailableFieldSlots: 0 };
  } catch { /* Invalid or unavailable evidence remains incomplete. */ }
  if (fs.existsSync(output)) {
    try {
      const config = JSON.parse(readSmall(path.join(output, 'native-supervision.json')));
      require('./bounded-storage.cjs').writeLauncherOutcome(path.resolve(executable), output, config.generation, outcome);
    } catch {
      // No unmetered fallback. Lost recording remains incomplete, including
      // when a receiver accepted but its archival acknowledgment cannot fit.
      outcome = { ...outcome, state: 'incomplete', validatedFieldSlots: 0, unavailableFieldSlots: 216 };
    }
  }
  return outcome;
}
module.exports = { validateOutcome, archiveReceiverOutcome };
if (require.main === module) {
  try {
    main(...process.argv.slice(2));
  }
  catch { console.error('mock supervised launch failed; incomplete, never resume'); process.exitCode = 1; }
}
