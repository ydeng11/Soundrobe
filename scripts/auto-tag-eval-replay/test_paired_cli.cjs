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
    assert.equal(report.budget.modelRequests, 0);
    assert.equal(report.budget.assistance, 'disabled');
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
    assert.equal(report.primaryEvaluationsPerImplementation, 30);
    assert.equal(report.supplementalAssistanceFailureEvaluationsPerImplementation, 0);
    assert.equal(report.budget.modelRequests, 0);
    assert.equal(report.budget.assistance, 'disabled');
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

test('opt-in paired assistance keeps equal per-job budgets and the full five-case denominator', () => {
  const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'paired-assisted-cli-'));
  try {
    const output = path.join(temp, 'run');
    const result = spawnSync(process.execPath, [cli, 'compare', output,
      path.join(temp, 'missing-before'), path.join(temp, 'missing-after'),
      path.join(__dirname, 'paired-safety-selection.json'), '--scripted-genre-assistance'], { encoding: 'utf8' });
    assert.equal(result.status, 1);
    const report = JSON.parse(fs.readFileSync(path.join(output, 'comparison.json')));
    assert.equal(report.requestedEvaluationsPerImplementation, 32);
    assert.equal(report.primaryEvaluationsPerImplementation, 30);
    assert.equal(report.supplementalAssistanceFailureEvaluationsPerImplementation, 2);
    assert.equal(report.budget.assistance, 'scripted');
    assert.equal(report.budget.modelRequestsPerJob, 2);
    assert.deepEqual(report.budget.modelRequestCaps, { baseline: 32, candidate: 32 });
    assert.equal(report.budget.modelRequests, 32);
    for (const arm of ['baseline', 'candidate']) {
      assert.equal(report[arm].skipped, 32);
      assert.equal(report[arm].requiredNoWriteControls, 6);
    }
    assert.equal(report.comparable, false);
  } finally {
    fs.rmSync(temp, { recursive: true, force: true });
  }
});

test('opt-in runs use the existing generic evaluator and scope candidate review instrumentation', () => {
  const { pairedNativeInvocation } = require('./paired-policy.cjs');
  assert.deepEqual(pairedNativeInvocation('baseline'), {
    args: ['--exact', 'commands::auto_tag::eval_tests::live_auto_tag_eval', '--ignored', '--nocapture'],
    env: {},
  });
  assert.deepEqual(pairedNativeInvocation('candidate'), {
    args: ['--exact', 'commands::auto_tag::eval_tests::live_auto_tag_eval', '--ignored', '--nocapture'],
    env: { SOUNDROBE_AUTO_TAG_EVAL_PAIRED_ASSISTED_REVIEW: '1' },
  });
  assert.throws(() => pairedNativeInvocation('unknown'), /paired arm/);
});

test('supplemental runner context pins scenario run ID and artifact directory', () => {
  const { pairedRunContext } = require('./paired-policy.cjs');
  const output = path.join(os.tmpdir(), 'paired-run');
  assert.deepEqual(pairedRunContext(output, 'baseline', {
    caseId: '60ebec7ffadb', profile: 'assisted_without_ids', scenario: 'assistance_failure',
  }), {
    directory: path.join(output, 'baseline', 'assisted_without_ids', '60ebec7ffadb',
      'supplemental', 'assistance_failure'),
    runId: 'baseline-assisted_without_ids-60ebec7ffadb-assistance_failure',
    artifactDirectory: 'baseline/assisted_without_ids/60ebec7ffadb/supplemental/assistance_failure',
  });
});

test('paired synthetic source fixture keeps one route and one model allowance across arms', () => {
  const { pairedGenreServiceManifest } = require('./paired-policy.cjs');
  const fixture = JSON.parse(fs.readFileSync(path.join(__dirname, 'synthetic-genre-source.json'), 'utf8'));
  const base = { schemaVersion: 1, records: [
    { provider: 'discogs', path: `/releases/${fixture.releaseId}`, query: {}, body: { id: Number(fixture.releaseId) } },
    { provider: 'musicbrainz', path: '/release', query: {}, body: {} },
  ] };
  const job = { caseId: fixture.caseId, profile: fixture.profile };
  const assisted = pairedGenreServiceManifest(base, fixture, job);
  assert.equal(assisted.records.filter(record => record.provider === 'discogs'
    && record.path === `/releases/${fixture.releaseId}`).length, 1);
  assert.deepEqual(assisted.records.find(record => record.provider === 'discogs'
    && record.path === `/releases/${fixture.releaseId}`).body, fixture.successReleaseResponse);
  assert.equal(assisted.scriptedModel.maxRequests, 2);
  assert.deepEqual(assisted.scriptedModel.pairedGenre.tracks,
    fixture.successReleaseResponse.tracklist.map(track => track.title));
  assert.equal(assisted.scriptedModel.response.genre, fixture.genre);

  const failure = pairedGenreServiceManifest(base, fixture,
    { ...job, scenario: 'assistance_failure' });
  assert.equal(failure.scriptedModel.maxRequests, assisted.scriptedModel.maxRequests);
  assert.equal(failure.scriptedModel.status, 503);
  assert.deepEqual(failure.records, assisted.records);

  const precedence = pairedGenreServiceManifest(base, fixture,
    { caseId: fixture.caseId, profile: 'folder_filename' });
  const precedenceBody = precedence.records.find(record => record.provider === 'discogs'
    && record.path === `/releases/${fixture.releaseId}`).body;
  assert.deepEqual(precedenceBody, fixture.providerPrecedenceReleaseResponse);
  assert.equal(precedence.scriptedModel, undefined);
  assert.deepEqual(pairedGenreServiceManifest(base, fixture,
    { caseId: 'f7756354af37', profile: fixture.profile }), base);
});

test('nested paired manifest loads bodies from the actual imported candidate pools', async () => {
  const { pairedGenreServiceManifest, rebaseManifestBodyFiles } = require('./paired-policy.cjs');
  const { createService, importPools } = require('../mock-provider-service.cjs');
  const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'paired-body-path-'));
  try {
    const basePath = path.join(temp, 'providers.json');
    importPools(path.join(__dirname, '../../test/fixtures/tauri/auto-tag-eval/candidate-pools.json'), basePath);
    const base = JSON.parse(fs.readFileSync(basePath, 'utf8'));
    const source = JSON.parse(fs.readFileSync(path.join(__dirname, 'synthetic-genre-source.json'), 'utf8'));
    const job = { caseId: source.caseId, profile: source.profile };
    const nested = path.join(temp, 'job-fixtures', 'job.json');
    fs.mkdirSync(path.dirname(nested));
    const nestedManifest = rebaseManifestBodyFiles(
      pairedGenreServiceManifest(base, source, job), basePath, nested,
      path.join(__dirname, '../../test/fixtures/tauri'));
    fs.writeFileSync(nested, JSON.stringify(nestedManifest));
    const expected = base.records.find(record => record.provider === 'discogs' && record.bodyFile);
    assert.ok(expected, 'candidate-pools import must retain a captured Discogs bodyFile');
    const originalBodyPath = path.resolve(path.dirname(basePath), expected.bodyFile);
    const originalBodyBytes = fs.readFileSync(originalBodyPath);
    const expectedBody = JSON.parse(originalBodyBytes.toString('utf8'));
    const rebased = nestedManifest.records.find(record => record.path === expected.path
      && record.provider === expected.provider);
    assert.equal(fs.realpathSync(path.resolve(path.dirname(nested), rebased.bodyFile)),
      fs.realpathSync(originalBodyPath));
    assert.equal(rebased.sha256, expected.sha256);
    const service = createService(nested);
    try {
      await new Promise((resolve, reject) => {
        service.once('error', reject);
        service.listen(0, '127.0.0.1', resolve);
      });
      const response = await fetch(`http://127.0.0.1:${service.address().port}/discogs${expected.path}`);
      assert.equal(response.status, 200);
      assert.deepEqual(await response.json(), expectedBody);
      assert.deepEqual(fs.readFileSync(originalBodyPath), originalBodyBytes,
        'rebasing must leave the read-only source body unchanged');
    } finally {
      await new Promise(resolve => service.close(resolve));
    }
  } finally {
    fs.rmSync(temp, { recursive: true, force: true });
  }
});

test('nested manifest rebasing rejects body files outside the approved fixture root', () => {
  const { rebaseManifestBodyFiles } = require('./paired-policy.cjs');
  const temp = fs.mkdtempSync(path.join(os.tmpdir(), 'paired-body-traversal-'));
  try {
    const approved = path.join(temp, 'fixtures');
    const output = path.join(temp, 'run');
    fs.mkdirSync(approved);
    fs.mkdirSync(output);
    const outside = path.join(temp, 'secret.json');
    fs.writeFileSync(outside, '{}');
    const basePath = path.join(output, 'providers.json');
    const nestedPath = path.join(output, 'jobs', 'job.json');
    assert.throws(() => rebaseManifestBodyFiles({ schemaVersion: 1,
      records: [{ provider: 'discogs', path: '/releases/42', bodyFile: '../secret.json', sha256: 'a'.repeat(64) }],
    }, basePath, nestedPath, approved), /escapes approved fixture root/);
  } finally {
    fs.rmSync(temp, { recursive: true, force: true });
  }
});

test('provider source lock permits only the pinned receipt addition and rejects policy or unknown edits', () => {
  const { assertProviderSourceCompatible } = require('./paired-policy.cjs');
  const baseline = spawnSync('git', ['show', 'HEAD:src-tauri/src/state/providers.rs'], {
    cwd: path.resolve(__dirname, '../..'), encoding: 'utf8' });
  assert.equal(baseline.status, 0, baseline.stderr);
  const candidate = fs.readFileSync(path.resolve(__dirname, '../../src-tauri/src/state/providers.rs'), 'utf8');
  assert.doesNotThrow(() => assertProviderSourceCompatible(baseline.stdout, candidate));
  assert.throws(() => assertProviderSourceCompatible(baseline.stdout,
    candidate.replace('const AUTO_TAG_MAX_PROVIDER_ATTEMPTS: u8 = 3;',
      'const AUTO_TAG_MAX_PROVIDER_ATTEMPTS: u8 = 4;')), /unapproved provider source difference/);
  assert.throws(() => assertProviderSourceCompatible(baseline.stdout,
    candidate.replace('Duration::from_secs(15)', 'Duration::from_secs(16)')), /unapproved provider source difference/);
  assert.throws(() => assertProviderSourceCompatible(baseline.stdout,
    candidate.replace('Synthetic loopback source: this record is described as dream pop.',
      'Synthetic loopback source: this record is described as noisy pop.')), /receipt allowlist block changed/);
});
