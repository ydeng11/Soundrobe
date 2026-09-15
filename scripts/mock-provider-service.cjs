#!/usr/bin/env node
// Standalone test utility. No upstream requests or application settings changes.
const fs = require('node:fs');
const path = require('node:path');
const http = require('node:http');
const crypto = require('node:crypto');

const prefixes = { musicbrainz: '/musicbrainz/ws/2', discogs: '/discogs' };
const sensitive = /token|secret|password|authorization|api[-_]?key|^key$/i;
const sha256 = (bytes) => crypto.createHash('sha256').update(bytes).digest('hex');
const readJson = (file) => JSON.parse(fs.readFileSync(file, 'utf8'));

function queryKey(query) {
  return JSON.stringify([...new URLSearchParams(query).entries()]
    .filter(([key]) => !sensitive.test(key))
    .sort(([a, av], [b, bv]) => a.localeCompare(b) || av.localeCompare(bv)));
}

function loadRecords(manifestPath) {
  const manifest = readJson(manifestPath);
  if (manifest.schemaVersion !== 1 || !Array.isArray(manifest.records)) {
    throw new Error('Expected schemaVersion 1 and records array');
  }
  const records = manifest.records.map((record) => {
    if (!prefixes[record.provider] || !/^\/[\w/.-]+\/?$/.test(record.path)) {
      throw new Error('Invalid provider or fixture path');
    }
    const status = record.status ?? 200;
    if (!Number.isInteger(status) || status < 200 || status > 599) throw new Error('Invalid status');
    let bytes;
    if (record.bodyFile) {
      bytes = fs.readFileSync(path.resolve(path.dirname(manifestPath), record.bodyFile));
      if (!record.sha256 || sha256(bytes) !== record.sha256) throw new Error('Snapshot hash mismatch');
      JSON.parse(bytes.toString('utf8'));
    } else {
      if (record.body === undefined) throw new Error('Fixture body required');
      bytes = Buffer.from(JSON.stringify(record.body));
    }
    const query = queryKey(record.query ?? {});
    if ([...new URLSearchParams(record.query ?? {}).keys()].some((key) => sensitive.test(key))) {
      throw new Error('Credential fields are forbidden in fixture queries');
    }
    const headers = {};
    for (const [key, value] of Object.entries(record.headers ?? {})) {
      if (!/^(retry-after|x-ratelimit-(limit|remaining|used))$/i.test(key)) {
        throw new Error('Only retry/rate-limit response headers are supported');
      }
      if (/[\r\n]/.test(String(value))) throw new Error('Invalid response header');
      headers[key] = String(value);
    }
    return { provider: record.provider, path: record.path, query,
      status, headers, bytes, sha256: sha256(bytes), source: record.source ?? null };
  });
  for (let i = 0; i < records.length; i++) {
    for (const other of records.slice(i + 1)) {
      const record = records[i];
      if (record.provider === other.provider && record.path === other.path && record.query === other.query) {
        throw new Error(`Fixture routes overlap: ${record.provider}${record.path}`);
      }
    }
  }
  return records;
}

function createService(manifestPath) {
  const records = loadRecords(manifestPath);
  const requests = [];
  const misses = new Map();
  const server = http.createServer((request, response) => {
    const url = new URL(request.url, 'http://127.0.0.1');
    const send = (status, body, headers = {}) => {
      const bytes = Buffer.isBuffer(body) ? body : Buffer.from(JSON.stringify(body));
      response.writeHead(status, { 'Content-Type': 'application/json', 'Content-Length': bytes.length, ...headers });
      response.end(bytes);
    };
    if (request.method !== 'GET') return send(405, { error: 'read_only_fixture_service' });
    if (url.pathname === '/__fixtures') {
      return send(200, { schemaVersion: 1, groundTruth: 'unreviewed_provider_evidence',
        records: records.map(({ bytes, ...record }) => ({ ...record, body: JSON.parse(bytes.toString()) })),
        requests, misses: [...misses.values()] });
    }
    const provider = Object.keys(prefixes).find((key) => url.pathname.startsWith(`${prefixes[key]}/`));
    const fixturePath = provider ? url.pathname.slice(prefixes[provider].length) : url.pathname;
    const query = queryKey(url.searchParams);
    const record = records.find((item) => item.provider === provider && item.path === fixturePath
      && item.query === query);
    const entry = { provider: provider ?? 'unknown', path: fixturePath, query: JSON.parse(query), status: record?.status ?? 501 };
    requests.push(entry);
    if (!record) {
      misses.set(JSON.stringify(entry), entry);
      return send(501, { error: 'offline_fixture_missing', request: entry });
    }
    send(record.status, record.bytes, record.headers);
  });
  return server;
}

// Import only raw provider payloads. Normalized ProviderAlbum candidates are
// deliberately excluded: they are not valid upstream API responses.
function importPools(poolPath, outputPath) {
  const pools = readJson(poolPath);
  const records = new Map();
  const skipped = [];
  for (const pool of pools.pools) {
    for (const candidate of pool.candidates) {
      const file = path.resolve(path.dirname(poolPath), candidate.response);
      const bytes = fs.readFileSync(file);
      if (sha256(bytes) !== candidate.responseSha256) throw new Error('Candidate snapshot hash mismatch');
      const body = JSON.parse(bytes.toString());
      const raw = candidate.provider === 'musicbrainz' ? Array.isArray(body.media) : Array.isArray(body.tracklist);
      if (!raw) { skipped.push({ provider: candidate.provider, releaseId: candidate.releaseId, reason: 'not_raw_provider_response' }); continue; }
      if (String(body.id) !== String(candidate.releaseId)) throw new Error('Snapshot provider ID mismatch');
      const route = candidate.provider === 'musicbrainz' ? `/release/${candidate.releaseId}` : `/releases/${candidate.releaseId}`;
      const query = candidate.provider === 'musicbrainz'
        ? { fmt: 'json', inc: 'recordings+artist-credits+labels+url-rels' }
        : {};
      const record = { provider: candidate.provider, path: route, query,
        bodyFile: path.relative(path.dirname(path.resolve(outputPath)), file), sha256: sha256(bytes),
        source: { kind: 'captured_release_detail', releaseId: String(candidate.releaseId) } };
      addImportedRecord(records, record, 'provider pool');
    }
  }
  const discoveryPath = path.join(path.dirname(poolPath), 'provider-discovery.json');
  if (fs.existsSync(discoveryPath)) {
    const discovery = readJson(discoveryPath);
    if (discovery.schemaVersion !== 1 || !Array.isArray(discovery.records)) {
      throw new Error('Expected discovery schemaVersion 1 and records array');
    }
    for (const sourceRecord of discovery.records) {
      if (!sourceRecord.bodyFile) throw new Error('Discovery fixture bodyFile required');
      const file = path.resolve(path.dirname(discoveryPath), sourceRecord.bodyFile);
      const bytes = fs.readFileSync(file);
      if (!sourceRecord.sha256 || sha256(bytes) !== sourceRecord.sha256) {
        throw new Error('Discovery snapshot hash mismatch');
      }
      JSON.parse(bytes.toString());
      const record = { ...sourceRecord,
        bodyFile: path.relative(path.dirname(path.resolve(outputPath)), file),
        sha256: sha256(bytes) };
      addImportedRecord(records, record, 'discovery');
    }
  }
  fs.mkdirSync(path.dirname(path.resolve(outputPath)), { recursive: true });
  fs.writeFileSync(outputPath, JSON.stringify({ schemaVersion: 1, groundTruth: 'unreviewed_provider_evidence',
    records: [...records.values()], skipped }, null, 2) + '\n');
}

function addImportedRecord(records, record, sourceLabel) {
  const key = `${record.provider}${record.path}|${queryKey(record.query ?? {})}`;
  const existing = records.get(key);
  const responseMetadataKey = (value) => JSON.stringify({
    status: value?.status ?? 200,
    headers: Object.entries(value?.headers ?? {})
      .map(([name, header]) => [name.toLowerCase(), String(header)])
      .sort(([a], [b]) => a.localeCompare(b)),
  });
  const isUnavailableEvidence = (value) => value?.status === 404
    && value.source?.kind === 'explicit_unavailable_evidence';
  const isCapturedReleaseDetail = (value) => (value?.status ?? 200) === 200
    && value.source?.kind === 'captured_release_detail';
  const responseConflicts = existing
    && (existing.sha256 !== record.sha256
      || responseMetadataKey(existing) !== responseMetadataKey(record));
  if (responseConflicts
      && isUnavailableEvidence(existing) && isCapturedReleaseDetail(record)) {
    records.set(key, record);
    return;
  }
  if (responseConflicts
      && isCapturedReleaseDetail(existing) && isUnavailableEvidence(record)) {
    return;
  }
  if (responseConflicts) {
    throw new Error(`Conflicting ${sourceLabel} snapshot for ${record.provider}${record.path}`);
  }
  records.set(key, record);
}

module.exports = { createService, importPools };

if (require.main === module) {
  try {
    const args = process.argv.slice(2);
    if (args[0] === 'import-pools' && args.length === 3) {
      importPools(path.resolve(args[1]), path.resolve(args[2]));
    } else if (args[0] === 'serve' && (args.length === 2 || args.length === 3)) {
      const server = createService(path.resolve(args[1]));
      const port = Number(args[2] ?? 0);
      if (!Number.isInteger(port) || port < 0 || port > 65535) throw new Error('Invalid port');
      server.on('error', (error) => { console.error(error.message); process.exitCode = 1; });
      server.listen(port, '127.0.0.1', () => {
        const base = `http://127.0.0.1:${server.address().port}`;
        console.log(JSON.stringify({ musicbrainz: `${base}${prefixes.musicbrainz}`, discogs: `${base}${prefixes.discogs}`, inventory: `${base}/__fixtures` }));
      });
      for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => { server.closeAllConnections(); server.close(); });
    } else {
      throw new Error('Usage: mock-provider-service.cjs import-pools POOLS OUTPUT | serve MANIFEST [PORT]');
    }
  } catch (error) { console.error(error.message); process.exitCode = 1; }
}
