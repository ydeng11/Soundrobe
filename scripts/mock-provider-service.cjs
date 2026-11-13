#!/usr/bin/env node
// Standalone test utility. No upstream requests or application settings changes.
const fs = require('./auto-tag-eval-replay/bounded-storage.cjs');
const path = require('node:path');
const http = require('node:http');
const crypto = require('node:crypto');
const { isDeepStrictEqual } = require('node:util');

const prefixes = { musicbrainz: '/musicbrainz/ws/2', discogs: '/discogs' };
const sensitive = /token|secret|password|authorization|api[-_]?key|^key$/i;
const sha256 = (bytes) => crypto.createHash('sha256').update(bytes).digest('hex');
const readJson = (file) => JSON.parse(fs.readFileSync(file, 'utf8'));

function queryKey(query) {
  return JSON.stringify([...new URLSearchParams(query).entries()]
    .filter(([key]) => !sensitive.test(key))
    .sort(([a, av], [b, bv]) => a.localeCompare(b) || av.localeCompare(bv)));
}

function validateUnavailableEvidence(record) {
  if (record.source?.kind === 'explicit_unavailable_evidence'
      && (!Number.isInteger(record.status) || record.status < 400 || record.status > 599)) {
    throw new Error('Explicit unavailable evidence requires a non-success HTTP status');
  }
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
    validateUnavailableEvidence({ ...record, status });
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

function createService(manifestPath, authority) {
  if (authority && typeof authority.onFatal !== 'function') throw Error('mock fixture requires a fatal lease callback');
  const records = loadRecords(manifestPath);
  const scripted = readJson(manifestPath).scriptedModel;
  if (scripted && (!Number.isInteger(scripted.maxRequests) || scripted.maxRequests < 1
      || scripted.maxRequests > 8 || !scripted.payload || !scripted.response)) {
    throw new Error('Scripted model requires a payload, response and allowance of 1–8 requests');
  }
  const schemaName = scripted?.schemaName ?? 'GenreFillResponse';
  if (scripted && !['GenreFillResponse', 'TagCorrectionResponse'].includes(schemaName)) {
    throw new Error('Unsupported scripted tagging schema');
  }
  const requests = [];
  const modelRequests = [];
  const wireArrivals = [];
  const misses = new Map();
  const compactInventory = () => {
    const compact = { schemaVersion: 1, compact: true, groundTruth: 'unreviewed_provider_evidence',
      records: records.map(({ bytes, ...record }) => ({ ...record, bytes: bytes.length })),
      requests, modelRequests,
      wireArrivals, upstreamForwardingAvailable: false, misses: [...misses.values()] };
    const bytes = Buffer.from(JSON.stringify(compact));
    if (bytes.length > 8388608) throw Error('compact_inventory_ceiling');
    return bytes;
  };
  let terminal = false;
  const denyIngress = async (reason) => {
    if (!authority) return;
    if (terminal) return;
    terminal = true;
    try {
      await require('./auto-tag-eval-replay/mock-authority.cjs').rpc(authority.socket, authority.service, {
        op: 'ingress_denied', reason,
      });
    } catch { authority.onFatal(); }
  };
  const server = http.createServer(async (request, response) => {
    const url = new URL(request.url, 'http://127.0.0.1');
    const wire = { method: request.method, route: url.pathname, status: null, state: 'receiving' };
    wireArrivals.push(wire);
    const send = (status, body, headers = {}) => {
      wire.status = status;
      wire.state = 'responded';
      const bytes = Buffer.isBuffer(body) ? body : Buffer.from(JSON.stringify(body));
      response.writeHead(status, { 'Content-Type': 'application/json', 'Content-Length': bytes.length, ...headers });
      response.end(bytes);
    };
    if (terminal) { request.resume(); return send(409, { error: 'mock_fixture_terminal' }); }
    let received = Buffer.alloc(0);
    try {
      for await (const chunk of request) {
        if (received.length + chunk.length > 65536) {
          wire.state = 'oversized'; await denyIngress('oversized'); request.destroy(); return;
        }
        received = Buffer.concat([received, chunk]);
      }
      if (authority) {
        if (terminal) return send(409, { error: 'mock_fixture_terminal' });
        const operation = request.headers['x-soundrobe-mock-operation'];
        if (!authority.service || typeof operation !== 'string') {
          await denyIngress('missing_operation'); return send(409, { error: 'mock_arrival_denied' });
        }
        wire.operation = operation;
        await require('./auto-tag-eval-replay/mock-authority.cjs').rpc(authority.socket, authority.service, {
          op: 'arrival', operation,
          request_sha256: sha256(Buffer.from(JSON.stringify([request.method, request.url, sha256(received)]))),
        });
      }
    } catch {
      wire.state = request.aborted ? 'aborted' : 'arrival_denied';
      await denyIngress(request.aborted ? 'aborted' : 'arrival_denied');
      return send(409, { error: 'mock_arrival_denied' });
    }
    if (scripted && request.method === 'POST' && url.pathname === '/model/chat/completions') {
      const bytes = received.toString();
        let body;
        try { body = JSON.parse(bytes); } catch { return send(400, { error: 'invalid_json' }); }
        const user = body.messages?.filter(message => message.role === 'user');
        let payload;
        try { payload = JSON.parse(user?.[0]?.content); } catch { /* Deny unsupported prompts. */ }
        if (schemaName === 'TagCorrectionResponse' && typeof payload?.full_path === 'string') {
          // Only the disposable evaluator prefix varies. The case, folder,
          // hints and complete physical-file payload must still match exactly.
          payload.full_path = payload.full_path.replace(
            /^\/private\/tmp\/soundrobe-auto-tag-eval-[0-9a-f-]{36}\/media\/(cold|warm)\//,
            '$EVAL_MEDIA/');
        }
        const allowed = (!authority || request.headers.authorization === 'Bearer MOCK_ONLY_SECRET_CANARY')
          && modelRequests.length < scripted.maxRequests
          && (!scripted.bodySha256 || sha256(Buffer.from(bytes)) === scripted.bodySha256)
          && body.model === (scripted.requestModel ?? 'scripted/genre-v1')
          && body.response_format?.json_schema?.name === schemaName
          && user?.length === 1 && isDeepStrictEqual(payload, scripted.payload);
        const status = allowed ? (scripted.status ?? 200) : 501;
        const result = status === 200 ? {
          model: scripted.responseModel ?? 'scripted/genre-v1', provider: scripted.provider ?? 'scripted-loopback-v1',
          choices: [{ finish_reason: 'stop', message: { content: JSON.stringify(scripted.response) } }],
          ...(scripted.usage === undefined ? {} : { usage: scripted.usage }),
        } : { error: allowed ? 'scripted_service_failure' : 'offline_model_fixture_missing' };
        if (scripted.privacyCanaries === true) result.diagnostic = 'MOCK_RAW_DIAGNOSTIC_CANARY';
        // Inventories never retain model bodies, parsed values or headers,
        // including the noncompact diagnostic endpoint and failure cases.
        modelRequests.push({ status, sha256: sha256(Buffer.from(bytes)), bytes: Buffer.byteLength(bytes),
          responseSha256: sha256(Buffer.from(JSON.stringify(result))), responseBytes: Buffer.byteLength(JSON.stringify(result)),
          ...(authority ? { mockCredentialPresented: request.headers.authorization === 'Bearer MOCK_ONLY_SECRET_CANARY' } : {}) });
        if (!allowed) misses.set(`model-${modelRequests.length}`, { provider: 'scripted_model', status });
        send(status, result, scripted.privacyCanaries === true ? { 'x-private-canary': 'MOCK_HEADER_SECRET_CANARY' } : {});
      return;
    }
    if (request.method !== 'GET') return send(405, { error: 'read_only_fixture_service' });
    if (url.pathname === '/__fixtures' || url.pathname === '/__fixtures/compact') {
      wire.status = 200;
      wire.state = 'responded';
      if (url.pathname === '/__fixtures/compact') {
        try { return send(200, compactInventory()); }
        catch { return send(507, { error: 'compact_inventory_ceiling' }); }
      }
      return send(200, { schemaVersion: 1, groundTruth: 'unreviewed_provider_evidence',
        records: records.map(({ bytes, ...record }) => ({ ...record, body: JSON.parse(bytes.toString()) })),
        requests, modelRequests, wireArrivals, upstreamForwardingAvailable: false, misses: [...misses.values()] });
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
  server.on('connect', async (request, socket) => {
    wireArrivals.push({ method: 'CONNECT', route: /^[a-z0-9.-]+:\d+$/i.test(request.url)
      ? request.url : '[invalid authority]', status: 403, state: 'denied_tunnel' });
    await denyIngress('connect');
    socket.end('HTTP/1.1 403 Forbidden\r\nConnection: close\r\nContent-Length: 0\r\n\r\n');
  });
  server.compactInventory = compactInventory;
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
      validateUnavailableEvidence(record);
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
