// @vitest-environment node
import { createRequire } from "node:module";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
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

it("imports locked raw detail snapshots from existing pools for offline review", async () => {
  const manifest = path.join(fs.mkdtempSync(path.join(os.tmpdir(), "soundrobe-import-")), "manifest.json");
  roots.push(path.dirname(manifest));
  importPools(path.resolve("test/fixtures/tauri/auto-tag-eval/candidate-pools.json"), manifest);
  const server = createService(manifest);
  servers.push(server);
  await new Promise<void>((resolve) => server.listen(0, "127.0.0.1", resolve));
  const base = `http://127.0.0.1:${server.address().port}`;
  expect((await (await fetch(`${base}/discogs/releases/1459867`)).json()).id).toBe(1459867);
  // A normalized candidate cannot stand in for an upstream release response;
  // this ID is explicitly locked as unavailable rather than silently omitted.
  const unavailable = await fetch(`${base}/discogs/releases/36441795`);
  expect(unavailable.status).toBe(404);
  expect((await unavailable.json()).reason).toBe("provider_evidence_not_captured");
  expect((await (await fetch(`${base}/musicbrainz/ws/2/release/627377a9-be56-4c45-a56d-9ae941546ef0?inc=recordings&fmt=json`)).json()).id).toBe("627377a9-be56-4c45-a56d-9ae941546ef0");
  expect((await (await fetch(`${base}/musicbrainz/ws/2/release/4f45e662-f5fa-44af-ba11-f202ee324df8?inc=recordings&fmt=json`)).json()).id).toBe("4f45e662-f5fa-44af-ba11-f202ee324df8");
  expect((await (await fetch(`${base}/musicbrainz/ws/2/artist/?query=artist%3A%22Enya%22&fmt=json&limit=5`)).json()).artists[0].id).toBe("4967c0a1-b9f3-465e-8440-4598fd9fc33c");
  const artistReleases = await fetch(`${base}/discogs/artists/9807/releases?page=1&per_page=100&sort=year&sort_order=desc`);
  expect(artistReleases.status).toBe(200);
  expect((await artistReleases.json()).releases).toEqual([]);
  expect((await fetch(`${base}/musicbrainz/ws/2/release?query=anything`)).status).toBe(501);
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
