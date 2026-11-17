const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const os = require('node:os');
const path = require('node:path');
const { spawnSync } = require('node:child_process');
const cli = path.join(__dirname, 'run-paired.cjs');

test('missing frozen executables retain every requested evaluation in a report', () => {
  const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'paired-cli-'));
  try {
    const output = path.join(temp, 'run');
    const result = spawnSync(process.execPath, [cli, 'compare', output,
      path.join(temp, 'missing-before'), path.join(temp, 'missing-after')], { encoding: 'utf8' });
    assert.equal(result.status, 1);
    assert.ok(fs.existsSync(path.join(output, 'comparison.json')), result.stderr);
    const report = JSON.parse(fs.readFileSync(path.join(output, 'comparison.json')));
    assert.equal(report.requestedEvaluationsPerImplementation, 12);
    assert.equal(report.baseline.skipped, 12);
    assert.equal(report.candidate.skipped, 12);
    assert.equal(report.comparable, false);
    assert.match(report.setupFailures.join('\n'), /missing-before/);
    const bytes = fs.readFileSync(path.join(output, 'comparison.json'));
    const retry = spawnSync(process.execPath, [cli, 'compare', output, 'absent', 'absent']);
    assert.equal(retry.status, 1);
    assert.deepEqual(fs.readFileSync(path.join(output, 'comparison.json')), bytes);
  } finally {
    fs.rmSync(temp, { recursive: true, force: true });
  }
});

test('expanded safety selection retains thirty slots and six unavailable no-write controls per arm', () => {
  const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'paired-safety-cli-'));
  try {
    const output = path.join(temp, 'run');
    const result = spawnSync(process.execPath, [cli, 'compare', output,
      path.join(temp, 'missing-before'), path.join(temp, 'missing-after'),
      path.join(__dirname, 'paired-safety-selection.json')], { encoding: 'utf8' });
    assert.equal(result.status, 1);
    const report = JSON.parse(fs.readFileSync(path.join(output, 'comparison.json')));
    assert.equal(report.requestedEvaluationsPerImplementation, 30);
    for (const arm of ['baseline', 'candidate']) {
      assert.equal(report[arm].skipped, 30);
      assert.equal(report[arm].requiredNoWriteControls, 6);
      assert.equal(report[arm].passedNoWriteControls, 0);
    }
    assert.equal(report.comparable, false);
    assert.equal(report.improvements, 0);
  } finally {
    fs.rmSync(temp, { recursive: true, force: true });
  }
});
