const { test } = require('node:test');
const assert = require('node:assert/strict');
const { validatePlan } = require('./run-rehearsal.cjs');
const packet = { profiles: ['folder_filename', 'assisted_without_ids', 'tagged_recovery'],
  cases: [13, 11, 11, 9, 16].map((tracks, i) => ({ caseId: String(i), tracks })) };
const plan = () => ({ assistance: 'disabled', modelRequestsMaximum: 0,
  jobs: packet.cases.flatMap(c => packet.profiles.map(profile => ({ ...c, profile }))) });
test('runs exactly the frozen five-case profile matrix', () => {
  assert.doesNotThrow(() => validatePlan(plan(), packet));
});
test('runs an explicitly locked one-case three-profile diagnostic selection', () => {
  const selected = { ...packet, cases: [packet.cases[2]] };
  const bounded = { ...plan(), jobs: plan().jobs.filter(j => j.caseId === '2') };
  assert.doesNotThrow(() => validatePlan(bounded, selected));
  for (const cases of [[], [packet.cases[2], packet.cases[2]]]) {
    assert.throws(() => validatePlan(bounded, { ...selected, cases }));
  }
});
test('rejects omitted, duplicate, substituted or extra jobs', () => {
  for (const mutate of [p => p.jobs.pop(), p => p.jobs[1] = p.jobs[0],
    p => p.jobs[0].caseId = 'other', p => p.jobs.push(p.jobs[0]),
    p => p.jobs[0].tracks = 14]) {
    const p = plan(); mutate(p); assert.throws(() => validatePlan(p, packet));
  }
});
test('profile names never enable assistance or model budgets', () => {
  for (const change of [{ assistance: 'scripted' }, { assistance: 'live' },
    { assistance: undefined }, { modelRequestsMaximum: 1 }]) {
    assert.throws(() => validatePlan({ ...plan(), ...change }, packet));
  }
});
