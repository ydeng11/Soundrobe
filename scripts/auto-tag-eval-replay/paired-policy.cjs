'use strict';

const crypto = require('node:crypto');
const fs = require('node:fs');
const path = require('node:path');

const receiptBlocks = {
  FIELD: ['ec27c234078c54d45759ed509f1d734956e074637ae9b8273947d64442a130c0', ''],
  HELPERS: ['10849b940feb66dcacb22d3ad376e2eb2779f30d330db854980c646e393010f0', ''],
  CAPTURE: ['960548b05042997193ee594bc0ef4eae4633a187e9797402d61a6fd3b53512df',
    '            album.discogs_release_receipt = Some(DiscogsReleaseReceipt {\n'
      + '                request_method: "GET".into(),\n'
      + '                request_path: format!("/{path}"),\n'
      + '                request_url: endpoint(\n'
      + '                    reqwest::Url::parse(&url).map_err(|_| "Invalid Discogs endpoint")?,\n'
      + '                ),\n'
      + '                response_url,\n'
      + '                resource_kind: "release".into(),\n'
      + '                resource_id: release_id.into(),\n'
      + '                response_status: status,\n'
      + '                response_sha256: format!("{:x}", Sha256::digest(&bytes)),\n'
      + '                response_body: String::from_utf8(bytes.to_vec())\n'
      + '                    .map_err(|_| "Discogs response is not UTF-8")?,\n'
      + '            });\n'],
  TEST: ['775d6120385dca0026144e6e2aeb0e1bd6cd23d526e4fcda1f7a52f9de596d08', ''],
};

const assistanceFlag = '--scripted-genre-assistance';
const querySignature = query => JSON.stringify([...new URLSearchParams(query ?? {}).entries()]
  .sort(([leftKey, leftValue], [rightKey, rightValue]) =>
    leftKey.localeCompare(rightKey) || leftValue.localeCompare(rightValue)));

function parseCompareOptions(args) {
  const rest = [...args];
  const scriptedAssistance = rest.includes(assistanceFlag);
  const remaining = rest.filter(arg => arg !== assistanceFlag);
  if (remaining.length > 1 || (scriptedAssistance && rest.filter(arg => arg === assistanceFlag).length !== 1)) {
    throw new Error('Expected one optional selection file and at most one assistance flag');
  }
  return { selection: remaining[0] ?? null, assistance: scriptedAssistance ? 'scripted' : 'disabled' };
}

function pairedBudget(assistance, evaluationJobs) {
  if (assistance === 'disabled') {
    return { assistance, modelRequests: 0, modelRequestsPerJob: 0,
      modelRequestCaps: { baseline: 0, candidate: 0 } };
  }
  if (assistance !== 'scripted' || !Number.isInteger(evaluationJobs) || evaluationJobs < 1) {
    throw new Error('Invalid paired assistance budget');
  }
  const capPerJob = 2; // One cold and one warm pass; both implementations receive the same cap.
  const perImplementation = evaluationJobs * capPerJob;
  return { assistance, modelRequests: perImplementation, modelRequestsPerJob: capPerJob,
    modelRequestCaps: { baseline: perImplementation, candidate: perImplementation } };
}

function pairedNativeInvocation(arm) {
  if (arm !== 'baseline' && arm !== 'candidate') throw new Error('Invalid paired arm');
  return {
    args: ['--exact', 'commands::auto_tag::eval_tests::live_auto_tag_eval', '--ignored', '--nocapture'],
    env: arm === 'candidate' ? { SOUNDROBE_AUTO_TAG_EVAL_PAIRED_ASSISTED_REVIEW: '1' } : {},
  };
}

function pairedRunContext(output, arm, job) {
  if (!['baseline', 'candidate'].includes(arm) || !job?.profile || !job?.caseId) {
    throw new Error('Invalid paired job context');
  }
  const directory = job.scenario
    ? path.join(output, arm, job.profile, job.caseId, 'supplemental', job.scenario)
    : path.join(output, arm, job.profile, job.caseId);
  const artifactDirectory = path.relative(output, directory).split(path.sep).join('/');
  const runId = [arm, job.profile, job.caseId, job.scenario].filter(Boolean).join('-');
  return { directory, runId, artifactDirectory };
}

function approvedManifestBodyPath(manifestPath, bodyFile, approvedRoot) {
  if (typeof bodyFile !== 'string' || !bodyFile || path.isAbsolute(bodyFile)) {
    throw new Error('Fixture bodyFile must be a relative path');
  }
  const root = fs.realpathSync(approvedRoot);
  const resolved = fs.realpathSync(path.resolve(path.dirname(manifestPath), bodyFile));
  const relative = path.relative(root, resolved);
  if (!relative || relative === '..' || relative.startsWith(`..${path.sep}`) || path.isAbsolute(relative)) {
    throw new Error('Fixture bodyFile escapes approved fixture root');
  }
  if (!fs.statSync(resolved).isFile()) throw new Error('Fixture bodyFile is not a regular file');
  return resolved;
}

function rebaseManifestBodyFiles(manifest, sourceManifestPath, targetManifestPath, approvedRoot) {
  if (!manifest || !Array.isArray(manifest.records)) throw new Error('Fixture manifest records required');
  const targetDirectory = path.dirname(targetManifestPath);
  return { ...manifest, records: manifest.records.map(record => {
    if (!record.bodyFile) return record;
    const sourceFile = approvedManifestBodyPath(sourceManifestPath, record.bodyFile, approvedRoot);
    const bodyFile = path.relative(targetDirectory, sourceFile);
    if (!bodyFile || path.isAbsolute(bodyFile)) throw new Error('Fixture bodyFile cannot be safely rebased');
    return { ...record, bodyFile };
  }) };
}

function pairedGenreServiceManifest(base, fixture, job) {
  if (!fixture || fixture.label !== 'synthetic_loopback_counterfactual'
      || !job || job.caseId !== fixture.caseId) return base;
  const releaseId = String(fixture.releaseId);
  const expectedRoute = { provider: 'discogs', path: `/releases/${releaseId}`, query: {} };
  if (JSON.stringify(fixture.route) !== JSON.stringify(expectedRoute)) {
    throw new Error('Synthetic genre fixture route changed');
  }
  const assisted = job.profile === fixture.profile;
  const body = assisted ? fixture.successReleaseResponse : fixture.providerPrecedenceReleaseResponse;
  if (!body || String(body.id) !== releaseId
      || (assisted && (body.genres?.length || body.styles?.length || body.notes !== fixture.sourceText))
      || (!assisted && (!body.genres?.length || body.notes))) {
    throw new Error('Synthetic genre fixture body does not match its declared profile');
  }
  const expectedQuery = querySignature(expectedRoute.query);
  const records = (base.records ?? []).filter(record => !(record.provider === expectedRoute.provider
    && record.path === expectedRoute.path && querySignature(record.query) === expectedQuery));
  records.push({ ...expectedRoute, status: 200, body });
  if (!assisted) return { ...base, records };
  if (job.scenario && job.scenario !== 'assistance_failure') {
    throw new Error('Unknown synthetic genre supplemental scenario');
  }
  return { ...base, records, scriptedModel: {
    maxRequests: 2,
    capturePayloads: true,
    pairedGenre: {
      artist: fixture.artist,
      album: fixture.album,
      tracks: body.tracklist.map(track => track.title),
      releaseId,
      responseBodySha256: crypto.createHash('sha256').update(JSON.stringify(body)).digest('hex'),
      text: fixture.sourceText,
      genre: fixture.genre,
    },
    ...(job.scenario === 'assistance_failure' ? { status: 503 } : {}),
    response: { genre: fixture.genre, confidence: 0.8 },
  } };
}

function replacePinnedBlock(source, name, [expectedDigest, replacement]) {
  const begin = `// BEGIN PAIRED_PROVIDER_RECEIPT_${name}`;
  const end = `// END PAIRED_PROVIDER_RECEIPT_${name}`;
  const begins = [...source.matchAll(new RegExp(`^[ \\t]*${begin}$`, 'gm'))];
  const ends = [...source.matchAll(new RegExp(`^[ \\t]*${end}$`, 'gm'))];
  if (begins.length === 0 && ends.length === 0) return source;
  if (begins.length !== 1 || ends.length !== 1 || begins[0].index >= ends[0].index) {
    throw new Error(`receipt allowlist block malformed: ${name}`);
  }
  const startLine = source.lastIndexOf('\n', begins[0].index) + 1;
  const bodyStart = source.indexOf('\n', begins[0].index) + 1;
  const endLine = source.lastIndexOf('\n', ends[0].index) + 1;
  const afterEnd = source.indexOf('\n', ends[0].index);
  const body = source.slice(bodyStart, endLine);
  const digest = crypto.createHash('sha256').update(body).digest('hex');
  if (digest !== expectedDigest) throw new Error(`receipt allowlist block changed: ${name}`);
  return source.slice(0, startLine) + replacement
    + source.slice(afterEnd < 0 ? source.length : afterEnd + 1);
}

function normalizeProviderSource(source) {
  const present = Object.keys(receiptBlocks).filter(name =>
    source.includes(`// BEGIN PAIRED_PROVIDER_RECEIPT_${name}`)
      || source.includes(`// END PAIRED_PROVIDER_RECEIPT_${name}`));
  if (present.length === 0) return source;
  if (present.length !== Object.keys(receiptBlocks).length) {
    throw new Error('receipt allowlist block set incomplete');
  }
  let normalized = source;
  for (const [name, block] of Object.entries(receiptBlocks)) {
    normalized = replacePinnedBlock(normalized, name, block);
  }
  return normalized;
}

function assertProviderSourceCompatible(baseline, candidate) {
  let normalizedBaseline;
  let normalizedCandidate;
  try {
    normalizedBaseline = normalizeProviderSource(baseline);
    normalizedCandidate = normalizeProviderSource(candidate);
  } catch (error) {
    throw new Error(`unapproved provider source difference: ${error.message}`);
  }
  if (normalizedBaseline !== normalizedCandidate) {
    throw new Error('unapproved provider source difference');
  }
}

module.exports = { assertProviderSourceCompatible, normalizeProviderSource, pairedBudget,
  pairedNativeInvocation, pairedRunContext, approvedManifestBodyPath, rebaseManifestBodyFiles,
  pairedGenreServiceManifest, parseCompareOptions };
