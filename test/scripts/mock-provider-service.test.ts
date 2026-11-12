// @vitest-environment node
import { createRequire } from "node:module";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import http from "node:http";
import { afterEach, expect, it } from "vitest";

const require = createRequire(import.meta.url);
const { createService, importPools } = require("../../scripts/mock-provider-service.cjs");
const roots: string[] = [];
const servers: any[] = [];
afterEach(async () => {
  await Promise.all(servers.splice(0).map((server) => new Promise<void>((resolve) => {
    server.closeAllConnections();
    server.close(() => resolve());
  })));
  roots.splice(0).forEach((root) => fs.rmSync(root, { recursive: true, force: true }));
});

async function start(records: unknown[]) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "soundrobe-provider-mock-"));
  roots.push(root);
  const manifest = path.join(root, "manifest.json");
  fs.writeFileSync(manifest, JSON.stringify({ schemaVersion: 1, records }));
  const server = createService(manifest);
  servers.push(server);
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  return { server, base: `http://127.0.0.1:${server.address().port}` };
}

it("replays exact search pages for both providers without treating missing evidence as no match", async () => {
  const { base } = await start([
    { provider: "musicbrainz", path: "/release", query: { query: "artist:Enya", offset: "0" }, body: { releases: [{ id: "mb" }] } },
    { provider: "discogs", path: "/database/search", query: { artist: "Enya", page: "1" }, body: { results: [{ id: 1 }] } },
  ]);
  expect(await (await fetch(`${base}/musicbrainz/ws/2/release?offset=0&query=artist%3AEnya&token=private`)).json()).toEqual({ releases: [{ id: "mb" }] });
  expect(await (await fetch(`${base}/discogs/database/search?page=1&artist=Enya`)).json()).toEqual({ results: [{ id: 1 }] });
  expect((await fetch(`${base}/discogs/database/search?page=2&artist=Enya`)).status).toBe(501);
});

it("preserves recorded errors and publishes sanitized misses for extending the dataset", async () => {
  const { base } = await start([{ provider: "discogs", path: "/releases/1", body: { message: "rate limit" }, status: 429, headers: { "Retry-After": "0" } }]);
  const response = await fetch(`${base}/discogs/releases/1`);
  expect(response.status).toBe(429);
  expect(response.headers.get("retry-after")).toBe("0");
  await fetch(`${base}/discogs/releases/2?token=secret&key=private`, { headers: { Authorization: "secret" } });
  const stats = await (await fetch(`${base}/__fixtures`)).json();
  expect(stats.misses).toHaveLength(1);
  expect(JSON.stringify(stats)).not.toMatch(/secret|private/);
  expect(stats.groundTruth).toBe("unreviewed_provider_evidence");
});

it("accounts for denied methods and CONNECT arrivals without forwarding or retaining credentials", async () => {
  const { base } = await start([]);
  expect((await fetch(`${base}/model/chat/completions`, { method: "POST", body: "{}" })).status).toBe(405);
  const tunnelStatus = await new Promise<number>((resolve) => {
    const request = http.request(base, { method: "CONNECT", path: "never-forward.invalid:443",
      headers: { Authorization: "Bearer private" } });
    request.on("connect", (response, socket) => { socket.destroy(); resolve(response.statusCode ?? 0); });
    request.on("error", () => resolve(0));
    request.end();
  });
  expect(tunnelStatus).toBe(403);
  const stats = await (await fetch(`${base}/__fixtures`)).json();
  expect(stats.wireArrivals.map((row: { method: string }) => row.method)).toEqual(["POST", "CONNECT", "GET"]);
  expect(stats.wireArrivals.map((row: { status: number }) => row.status)).toEqual([405, 403, 200]);
  expect(stats.upstreamForwardingAvailable).toBe(false);
  expect(JSON.stringify(stats)).not.toContain("private");
});

it.each([undefined, { prompt_tokens: 7, completion_tokens: 3, total_tokens: 10 }])(
  "denies model calls by default and preserves explicitly scripted usage %j", async (usage) => {
  const { base } = await start([]);
  expect((await fetch(`${base}/model/chat/completions`, { method: "POST", body: "{}" })).status).toBe(405);
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "soundrobe-model-mock-"));
  roots.push(root);
  const manifest = path.join(root, "manifest.json");
  const payload = { artist: "Billie Eilish", album: "Ocean Eyes", tracks: ["ocean eyes"] };
  fs.writeFileSync(manifest, JSON.stringify({ schemaVersion: 1, records: [], scriptedModel: {
    maxRequests: 1, payload, response: { genre: "Pop", confidence: 0.8 }, usage,
  } }));
  const server = createService(manifest);
  servers.push(server);
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const url = `http://127.0.0.1:${server.address().port}`;
  const request = { model: "scripted/genre-v1", messages: [{ role: "user", content: JSON.stringify(payload) }],
    response_format: { json_schema: { name: "GenreFillResponse" } } };
  const post = (body: unknown) => fetch(`${url}/model/chat/completions`, {
    method: "POST", body: JSON.stringify(body), headers: { Authorization: "Bearer private" },
  });
  expect((await post(request)).status).toBe(200);
  expect((await post(request)).status).toBe(501);
  const stats = await (await fetch(`${url}/__fixtures`)).json();
  expect(stats.modelRequests).toHaveLength(2);
  expect(stats.modelRequests[0].body).toEqual(request);
  expect(stats.modelRequests[0].response.choices[0].message.content).toBe(JSON.stringify({ genre: "Pop", confidence: 0.8 }));
  expect(stats.modelRequests[0].response.usage).toEqual(usage);
  expect(stats.misses).toHaveLength(1);
  expect(JSON.stringify(stats)).not.toContain("private");
});

it("replays metadata rejection controls without accepting arbitrary schemas, paths, or file sets", async () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "soundrobe-metadata-model-mock-"));
  roots.push(root);
  const manifest = path.join(root, "manifest.json");
  const payload = { full_path: "$EVAL_MEDIA/case/Enya/Album", current_tracks: [{ index: 0, filename: "Remastered" }] };
  fs.writeFileSync(manifest, JSON.stringify({ schemaVersion: 1, records: [], scriptedModel: {
    maxRequests: 4, schemaName: "TagCorrectionResponse", payload, response: { confidence: 0.99, tracks: [] },
  } }));
  const server = createService(manifest);
  servers.push(server);
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const url = `http://127.0.0.1:${server.address().port}`;
  const actual = { ...payload, full_path: "/private/tmp/soundrobe-auto-tag-eval-12345678-1234-1234-1234-123456789abc/media/cold/case/Enya/Album" };
  const post = (input: unknown, schema = "TagCorrectionResponse") => fetch(`${url}/model/chat/completions`, {
    method: "POST", body: JSON.stringify({ model: "scripted/genre-v1", messages: [{ role: "user", content: JSON.stringify(input) }],
      response_format: { json_schema: { name: schema } } }),
  });
  expect((await post(actual)).status).toBe(200);
  expect((await post({ ...actual, full_path: "/Users/real/Music/Enya/Album" })).status).toBe(501);
  expect((await post({ ...actual, current_tracks: [] })).status).toBe(501);
  expect((await post(actual, "ArbitraryTool" )).status).toBe(501);
  const stats = await (await fetch(`${url}/__fixtures`)).json();
  expect(stats.misses).toHaveLength(3);
});

it("compacts bodies without losing arrivals, misses or exact serialized model binding", async () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "soundrobe-compact-mock-"));
  roots.push(root);
  const manifest = path.join(root, "manifest.json");
  const payload = { artist: "Catalog artist", tracks: ["Catalog title"] };
  const body = { model: "scripted/genre-v1", messages: [{ role: "user", content: JSON.stringify(payload) }],
    response_format: { json_schema: { name: "GenreFillResponse" } }, temperature: 0.2 };
  const serialized = JSON.stringify(body);
  const digest = crypto.createHash("sha256").update(serialized).digest("hex");
  fs.writeFileSync(manifest, JSON.stringify({ schemaVersion: 1,
    records: [{ provider: "discogs", path: "/releases/1", body: { large: "x".repeat(9000000) } }],
    scriptedModel: { maxRequests: 2, payload, bodySha256: digest, response: { genre: "Diagnostic", confidence: 0.8 } } }));
  const server = createService(manifest);
  servers.push(server);
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const base = `http://127.0.0.1:${server.address().port}`;
  const post = (text: string) => fetch(`${base}/model/chat/completions`, { method: "POST", body: text });
  expect((await post(JSON.stringify({ ...body, temperature: 1 }))).status).toBe(501);
  expect((await post(serialized)).status).toBe(200);
  await fetch(`${base}/discogs/releases/missing`);
  const response = await fetch(`${base}/__fixtures/compact`);
  const raw = await response.text();
  expect(response.status).toBe(200);
  expect(Buffer.byteLength(raw)).toBeLessThan(8388608);
  const stats = JSON.parse(raw);
  expect(stats.records[0]).toMatchObject({ bytes: 9000012, sha256: expect.any(String) });
  expect(stats.records[0]).not.toHaveProperty("body");
  expect(stats.modelRequests.map((r: { status: number }) => r.status)).toEqual([501, 200]);
  expect(stats.modelRequests[1]).toMatchObject({ sha256: digest, bytes: Buffer.byteLength(serialized) });
  expect(stats.modelRequests[1]).not.toHaveProperty("body");
  expect(stats.modelRequests[1]).not.toHaveProperty("response");
  expect(stats.misses).toHaveLength(2);
  expect(stats.wireArrivals).toHaveLength(4);
});

it("imports locked raw detail snapshots from existing pools for offline review", async () => {
  const manifest = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "soundrobe-import-")), "manifest.json");
  roots.push(path.dirname(manifest));
  importPools(path.resolve("test/fixtures/tauri/auto-tag-eval/candidate-pools.json"), manifest);
  const server = createService(manifest);
  servers.push(server);
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const base = `http://127.0.0.1:${server.address().port}`;
  expect((await (await fetch(`${base}/discogs/releases/1459867`)).json()).id).toBe(1459867);
  const relapse = await fetch(`${base}/discogs/releases/36441795`);
  expect(relapse.status).toBe(200);
  expect((await relapse.json()).tracklist).toHaveLength(22);
  expect((await (await fetch(`${base}/musicbrainz/ws/2/release/627377a9-be56-4c45-a56d-9ae941546ef0?inc=recordings%2Bartist-credits%2Blabels%2Burl-rels&fmt=json`)).json()).id).toBe("627377a9-be56-4c45-a56d-9ae941546ef0");
  const exactMusicBrainz = await fetch(`${base}/musicbrainz/ws/2/release/4f45e662-f5fa-44af-ba11-f202ee324df8?inc=recordings%2Bartist-credits%2Blabels%2Burl-rels&fmt=json`);
  expect((await exactMusicBrainz.json()).id).toBe("4f45e662-f5fa-44af-ba11-f202ee324df8");
  expect((await fetch(`${base}/musicbrainz/ws/2/release/4f45e662-f5fa-44af-ba11-f202ee324df8?inc=recordings&fmt=json`)).status).toBe(501);
  expect((await (await fetch(`${base}/musicbrainz/ws/2/artist/?query=artist%3A%22Enya%22&fmt=json&limit=5`)).json()).artists[0].id).toBe("4967c0a1-b9f3-465e-8440-4598fd9fc33c");
  expect((await fetch(`${base}/discogs/releases/1459867?unexpected=query`)).status).toBe(501);
  const artistReleases = await fetch(`${base}/discogs/artists/9807/releases?page=1&per_page=100&sort=year&sort_order=desc`);
  expect(artistReleases.status).toBe(200);
  expect((await artistReleases.json()).pagination).toMatchObject({ page: 1, per_page: 100 });
  expect((await fetch(`${base}/musicbrainz/ws/2/release?query=anything`)).status).toBe(501);
  const celts = await fetch(`${base}/musicbrainz/ws/2/release/2c67f46d-1a66-3d71-9e0c-f696297063b9?fmt=json&inc=recordings%2Bartist-credits%2Blabels%2Burl-rels`);
  expect(celts.status).toBe(200);
  const celtsBody = await celts.json();
  expect(celtsBody).toMatchObject({ id: "2c67f46d-1a66-3d71-9e0c-f696297063b9", title: "The Celts" });
  expect(celtsBody.media[0].tracks).toHaveLength(15);
  const single = await fetch(`${base}/discogs/releases/21035242`);
  expect(single.status).toBe(200);
  const singleBody = await single.json();
  expect(singleBody).toMatchObject({ id: 21035242, title: "Noche De Paz - The Celts" });
  expect(singleBody.tracklist).toHaveLength(4);
  const alternate = await fetch(`${base}/discogs/releases/1600135`);
  expect(alternate.status).toBe(200);
  expect((await alternate.json()).id).toBe(1600135);
  expect((await fetch(`${base}/discogs/releases/21035242?unexpected=query`)).status).toBe(501);
  const exileQuery = new URLSearchParams({ fmt: "json", limit: "10", query: 'artist:"Enya" AND release:"Exile   Five Inch CD Single"' });
  const exile = await fetch(`${base}/musicbrainz/ws/2/release?${exileQuery}`);
  expect(exile.status).toBe(200);
  expect((await exile.json()).releases).toEqual([]);
  exileQuery.set("query", 'artist:"Enya" AND release:"Exile Five Inch CD Single"');
  expect((await fetch(`${base}/musicbrainz/ws/2/release?${exileQuery}`)).status).toBe(501);
  const celtsSearch = await fetch(`${base}/discogs/database/search?page=1&per_page=10&q=Enya+The+Celts&type=release`);
  expect(celtsSearch.status).toBe(200);
  const celtsSearchBody = await celtsSearch.json();
  expect(celtsSearchBody.results).toContainEqual(expect.objectContaining({ id: 6961860, title: "Enya - The Celts" }));
  expect(celtsSearchBody.pagination).toMatchObject({ page: 1, per_page: 10 });
  const stormsQuery = new URLSearchParams({ country: "United States", page: "1", per_page: "10", q: "Enya Storms In Africa   Single, Promo", type: "release" });
  expect((await fetch(`${base}/discogs/database/search?${stormsQuery}`)).status).toBe(200);
  const stormsDetail = await fetch(`${base}/musicbrainz/ws/2/release/4f45e662-f5fa-44af-ba11-f202ee324df8?fmt=json&inc=recordings%2Bartist-credits%2Blabels%2Burl-rels`);
  expect(stormsDetail.status).toBe(200);
  expect(await stormsDetail.json()).toMatchObject({
    relations: expect.arrayContaining([expect.objectContaining({
      type: "discogs", url: expect.objectContaining({ resource: "https://www.discogs.com/release/1459867" }),
    })]),
  });
  const exileDetail = await fetch(`${base}/discogs/releases/3420328`);
  expect(exileDetail.status).toBe(200);
  expect((await exileDetail.json()).id).toBe(3420328);
  const dreamsDetail = await fetch(`${base}/musicbrainz/ws/2/release/76969f4f-cb44-453b-af6f-cc48a68d2a2a?fmt=json&inc=recordings%2Bartist-credits%2Blabels%2Burl-rels`);
  expect(dreamsDetail.status).toBe(200);
  expect((await dreamsDetail.json()).id).toBe("76969f4f-cb44-453b-af6f-cc48a68d2a2a");
  const dreamsQuery = new URLSearchParams({ fmt: "json", limit: "10", query: 'artist:"Enya" AND release:"A Box of Dreams"' });
  const dreamsSearch = await fetch(`${base}/musicbrainz/ws/2/release?${dreamsQuery}`);
  expect(dreamsSearch.status).toBe(200);
  expect(Array.isArray((await dreamsSearch.json()).releases)).toBe(true);
  dreamsQuery.set("limit", "5");
  expect((await fetch(`${base}/musicbrainz/ws/2/release?${dreamsQuery}`)).status).toBe(501);
});

it("replays current artist browsing and pagination without substituting placeholder artist IDs", async () => {
  const manifest = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "soundrobe-browse-")), "manifest.json");
  roots.push(path.dirname(manifest));
  importPools(path.resolve("test/fixtures/tauri/auto-tag-eval/candidate-pools.json"), manifest);
  const server = createService(manifest);
  servers.push(server);
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const base = `http://127.0.0.1:${server.address().port}`;
  const eminem = await (await fetch(`${base}/musicbrainz/ws/2/artist/?query=artist%3A%22Eminem%22&fmt=json&limit=5`)).json();
  expect(eminem.artists[0]).toMatchObject({ id: "b95ce3ff-3d05-4e87-9e01-c97b66af13d4", name: "Eminem" });
  const enya = await (await fetch(`${base}/discogs/database/search?q=Enya&type=artist&per_page=5`)).json();
  expect(enya.results[0]).toMatchObject({ id: 9807, title: "Enya" });
  for (const artist of ["4967c0a1-b9f3-465e-8440-4598fd9fc33c", "b95ce3ff-3d05-4e87-9e01-c97b66af13d4",
    "5df62a88-cac9-490a-b62c-c7c88f4020f4", "f46bd570-5768-462e-b84c-c7c993bbf47e", "33ca19f4-18c8-4411-98df-ac23890ce9f5"]) {
    let offset = 0;
    let total = 1;
    while (offset < total) {
      const query = new URLSearchParams({ artist, limit: "100", offset: String(offset), fmt: "json", inc: "artist-credits+media+release-groups" });
      const response = await fetch(`${base}/musicbrainz/ws/2/release?${query}`);
      expect(response.status).toBe(200);
      const body = await response.json();
      expect(body["release-offset"]).toBe(offset);
      expect(body["release-count"]).toBeGreaterThan(0);
      expect(body.releases.length).toBeGreaterThan(0);
      expect(body.releases[0]).toHaveProperty("release-group.id");
      offset += body.releases.length;
      total = body["release-count"];
    }
    expect(offset).toBe(total);
  }
  for (const artist of ["9807", "38661"]) {
    let pages = 1;
    for (let page = 1; page <= pages; page++) {
      const response = await fetch(`${base}/discogs/artists/${artist}/releases?page=${page}&per_page=100&sort=year&sort_order=desc`);
      expect(response.status).toBe(200);
      const body = await response.json();
      expect(body.pagination).toMatchObject({ page, per_page: 100 });
      expect(body.releases.length).toBeGreaterThan(0);
      expect(body.pagination.pages).toBeGreaterThan(1);
      if (page === 1) pages = body.pagination.pages;
      expect(body.pagination.pages).toBe(pages);
    }
    expect((await fetch(`${base}/discogs/artists/${artist}/releases?page=2&per_page=25&sort=year&sort_order=desc`)).status).toBe(501);
  }
});

it("fails on overlapping routes and tampered snapshot evidence", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "soundrobe-invalid-mock-"));
  roots.push(root);
  const manifest = path.join(root, "manifest.json");
  const record = { provider: "discogs", path: "/releases/1", body: {} };
  fs.writeFileSync(manifest, JSON.stringify({ schemaVersion: 1, records: [record, record] }));
  expect(() => createService(manifest)).toThrow(/overlap/);
  fs.writeFileSync(path.join(root, "body.json"), "{}");
  fs.writeFileSync(manifest, JSON.stringify({ schemaVersion: 1, records: [{ ...record, body: undefined, bodyFile: "body.json", sha256: "bad" }] }));
  expect(() => createService(manifest)).toThrow(/hash/);
  fs.writeFileSync(manifest, JSON.stringify({ schemaVersion: 1, records: [{ ...record, query: [["token", "secret"]] }] }));
  expect(() => createService(manifest)).toThrow(/Credential/);
});

it("requires an explicit non-success status for unavailable evidence", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "soundrobe-invalid-unavailable-"));
  roots.push(root);
  const manifest = path.join(root, "manifest.json");
  fs.writeFileSync(manifest, JSON.stringify({ schemaVersion: 1, records: [{
    provider: "musicbrainz",
    path: "/release",
    body: { releases: [] },
    source: { kind: "explicit_unavailable_evidence" },
  }] }));
  expect(() => createService(manifest)).toThrow(/non-success HTTP status/);

  const body = JSON.stringify({ releases: [] });
  const bodyPath = path.join(root, "body.json");
  fs.writeFileSync(bodyPath, body);
  const sha256 = crypto.createHash("sha256").update(body).digest("hex");
  const poolsPath = path.join(root, "candidate-pools.json");
  fs.writeFileSync(poolsPath, JSON.stringify({ pools: [] }));
  fs.writeFileSync(path.join(root, "provider-discovery.json"), JSON.stringify({
    schemaVersion: 1,
    records: [{
      provider: "musicbrainz",
      path: "/release",
      bodyFile: "body.json",
      sha256,
      source: { kind: "explicit_unavailable_evidence" },
    }],
  }));
  expect(() => importPools(poolsPath, path.join(root, "imported.json")))
    .toThrow(/non-success HTTP status/);
});

it("rejects imported response metadata conflicts even when bodies match", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "soundrobe-conflicting-mock-"));
  roots.push(root);
  const body = JSON.stringify({ id: "1", tracklist: [] });
  const bodyPath = path.join(root, "body.json");
  fs.writeFileSync(bodyPath, body);
  const sha256 = crypto.createHash("sha256").update(body).digest("hex");
  const poolsPath = path.join(root, "candidate-pools.json");
  fs.writeFileSync(poolsPath, JSON.stringify({
    pools: [{ candidates: [{
      provider: "discogs",
      releaseId: "1",
      response: "body.json",
      responseSha256: sha256,
      expectation: "acceptable",
    }] }],
  }));
  fs.writeFileSync(path.join(root, "provider-discovery.json"), JSON.stringify({
    schemaVersion: 1,
    records: [{
      provider: "discogs",
      path: "/releases/1",
      status: 429,
      bodyFile: "body.json",
      sha256,
      source: { kind: "rate_limit" },
    }],
  }));

  expect(() => importPools(poolsPath, path.join(root, "manifest.json")))
    .toThrow(/Conflicting discovery snapshot/);
});
