// @vitest-environment node
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";

const repoRoot = path.resolve(__dirname, "../..");
const scriptPath = path.join(repoRoot, "scripts/review_auto_tag_subset.py");
const corpusPath = path.join(repoRoot, "test/fixtures/tauri/auto-tag-eval/corpus.json");
const fixtureRoot = path.join(repoRoot, "test/fixtures/tauri/auto-tag-eval");
const manifestPath = path.join(fixtureRoot, "reviewed-subset.json");
const temporaryRoots: string[] = [];

afterEach(() => {
  for (const root of temporaryRoots.splice(0)) fs.rmSync(root, { recursive: true, force: true });
});

describe("review_auto_tag_subset.py", () => {
  it("replays the checked-in representative review and preserves unresolved rows", () => {
    const output = fs.mkdtempSync(path.join(os.tmpdir(), "soundrobe-subset-"));
    temporaryRoots.push(output);
    execFileSync("python3", [
      scriptPath,
      "--corpus", corpusPath,
      "--manifest", manifestPath,
      "--fixture-root", fixtureRoot,
      "--output-dir", output,
    ], { encoding: "utf8" });
    const result = JSON.parse(fs.readFileSync(path.join(output, "review.json"), "utf8"));
    expect(result).toMatchObject({
      caseCount: 30,
      trackCount: 320,
      providerSnapshotCount: 20,
      statusCounts: { verified_match: 20, unresolved: 10 },
      baseline: { exactTitleMatches: 190, trackCount: 243, completeCases: 3 },
      final: { strongMatches: 243, completeCases: 20 },
    });
    expect(result.cases.find((item: { caseId: string }) => item.caseId === "e8a8a8d6b27d")).toMatchObject({
      status: "verified_match",
      flags: ["normalized-title"],
    });
    expect(result.cases.find((item: { caseId: string }) => item.caseId === "4147560c89fd")).toMatchObject({
      providerTrackCount: 46,
      providerTrackPolicy: { kind: "selected_media", mediaPosition: "1" },
      unmatchedProviderTracks: expect.arrayContaining(["2-1", "3-1"]),
    });
    expect(result.cases.find((item: { caseId: string }) => item.caseId === "7f3eb9349ac2")).toMatchObject({
      providerTrackCount: 23,
      providerTrackPolicy: { kind: "allowed_extras", providerTracks: ["2-7"] },
      unmatchedProviderTracks: ["2-7"],
    });
    expect(result.cases.find((item: { caseId: string }) => item.caseId === "b7fbe4f8ccd1")).toMatchObject({
      status: "unresolved",
      flags: ["unresolved"],
    });
    expect(fs.readFileSync(path.join(output, "review.md"), "utf8")).toContain("Explicitly unresolved: 10");
    const expectations = JSON.parse(fs.readFileSync(path.join(output, "expectations.json"), "utf8"));
    expect(expectations).toMatchObject({
      schemaVersion: 1,
      corpusVersion: "2026-09-12.inventory-1",
      cases: expect.arrayContaining([
        expect.objectContaining({
          caseId: "0c9e8b505ea7",
          status: "verified_match",
          acceptableEditionIds: ["bf39b43f-2bf1-48ad-bdfe-d7608004df52"],
          matcherAttribution: true,
        }),
        expect.objectContaining({
          caseId: "7f3eb9349ac2",
          providerTrackCount: 23,
          providerTrackPolicy: { kind: "allowed_extras", providerTracks: ["2-7"] },
          unmatchedProviderTracks: ["2-7"],
        }),
        expect.objectContaining({
          caseId: "b7fbe4f8ccd1",
          status: "unresolved",
          acceptableEditionIds: [],
          rejectedHardNegativeIds: ["6020781"],
        }),
      ]),
    });
  });

  it("fails closed on a duration conflict instead of promoting a title-only match", () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "soundrobe-subset-invalid-"));
    temporaryRoots.push(root);
    const fixture = path.join(root, "fixtures");
    fs.mkdirSync(fixture, { recursive: true });
    fs.writeFileSync(path.join(fixture, "release.json"), JSON.stringify({
      artist: "Artist",
      tracks: [{ title: "Song", track_number: 1, length: 120000 }],
    }), "utf8");
    fs.writeFileSync(path.join(fixture, "corpus.json"), JSON.stringify({
      corpusVersion: "test",
      cases: [{ caseId: "case", artist: "Artist", tracks: [{ title: "Song", duration: 10 }] }],
    }), "utf8");
    fs.writeFileSync(path.join(fixture, "manifest.json"), JSON.stringify({
      schemaVersion: 1,
      cases: [{ caseId: "case", status: "verified_match", provider: "musicbrainz", releaseId: "release", snapshot: "release.json" }],
    }), "utf8");
    expect(() => execFileSync("python3", [
      scriptPath,
      "--corpus", path.join(fixture, "corpus.json"),
      "--manifest", path.join(fixture, "manifest.json"),
      "--fixture-root", fixture,
      "--output-dir", path.join(root, "out"),
    ], { encoding: "utf8", stdio: "pipe" })).toThrow(/duration-conflict/);
  });

  it("fails closed on malformed provider positions", () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "soundrobe-subset-invalid-position-"));
    temporaryRoots.push(root);
    const fixture = path.join(root, "fixtures");
    fs.mkdirSync(fixture, { recursive: true });
    fs.writeFileSync(path.join(fixture, "release.json"), JSON.stringify({
      artists: [{ name: "Artist" }],
      tracklist: [{ type_: "track", position: "malformed-1", title: "Song", duration: "0:10" }],
    }), "utf8");
    fs.writeFileSync(path.join(fixture, "corpus.json"), JSON.stringify({
      corpusVersion: "test",
      cases: [{ caseId: "case", artist: "Artist", tracks: [{ title: "Song", duration: 10 }] }],
    }), "utf8");
    fs.writeFileSync(path.join(fixture, "manifest.json"), JSON.stringify({
      schemaVersion: 1,
      cases: [{ caseId: "case", status: "verified_match", provider: "discogs", releaseId: "release", snapshot: "release.json" }],
    }), "utf8");
    expect(() => execFileSync("python3", [
      scriptPath,
      "--corpus", path.join(fixture, "corpus.json"),
      "--manifest", path.join(fixture, "manifest.json"),
      "--fixture-root", fixture,
      "--output-dir", path.join(root, "out"),
    ], { encoding: "utf8", stdio: "pipe" })).toThrow(/invalid provider positions/);
  });

  it("accepts only an explicitly reviewed artist alias without relaxing track evidence", () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "soundrobe-subset-alias-"));
    temporaryRoots.push(root);
    const payload = { artist: "盧廣仲", tracks: [{ title: "明仔載 (正式版)", track_number: 1, length: 212173 }] };
    const row = { caseId: "case", status: "verified_match", provider: "musicbrainz",
      releaseId: "release", snapshot: "release.json", artistAliases: ["盧廣仲"] };
    fs.writeFileSync(path.join(root, "release.json"), JSON.stringify(payload));
    fs.writeFileSync(path.join(root, "corpus.json"), JSON.stringify({ corpusVersion: "test", cases: [
      { caseId: "case", artist: "卢广仲", tracks: [{ title: "明仔載 (正式版)", duration: 210.152 }] },
    ] }));
    const manifest = { schemaVersion: 1, cases: [row] };
    const manifestFile = path.join(root, "manifest.json");
    fs.writeFileSync(manifestFile, JSON.stringify(manifest));
    const command = [scriptPath, "--corpus", path.join(root, "corpus.json"),
      "--manifest", manifestFile, "--fixture-root", root, "--output-dir", path.join(root, "out")];
    execFileSync("python3", command, { stdio: "pipe" });
    const result = JSON.parse(fs.readFileSync(path.join(root, "out/review.json"), "utf8"));
    expect(result.cases[0]).toMatchObject({ status: "verified_match", flags: ["reviewed-artist-alias"],
      mapping: [{ localTrack: 1, providerTrack: "1", durationDeltaSeconds: 2.021 }] });
    row.artistAliases = [];
    fs.writeFileSync(manifestFile, JSON.stringify(manifest));
    expect(() => execFileSync("python3", command, { stdio: "pipe" })).toThrow(/artist conflict/);
    row.artistAliases = ["另一位歌手"];
    fs.writeFileSync(manifestFile, JSON.stringify(manifest));
    expect(() => execFileSync("python3", command, { stdio: "pipe" })).toThrow(/artist conflict/);
    fs.writeFileSync(manifestFile, JSON.stringify({ ...manifest, cases: [
      { ...row, artistAliases: "不可以用字串包含盧廣仲" },
    ] }));
    expect(() => execFileSync("python3", command, { stdio: "pipe" })).toThrow(/artistAliases must be/);
    row.artistAliases = ["盧廣仲"];
    fs.writeFileSync(manifestFile, JSON.stringify(manifest));
    payload.tracks[0].title = "明仔載 (Live版)";
    fs.writeFileSync(path.join(root, "release.json"), JSON.stringify(payload));
    expect(() => execFileSync("python3", command, { stdio: "pipe" })).toThrow(/not complete/);
  });

  it("uses a frozen recording duration only when the release track has none", () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "soundrobe-subset-recording-duration-"));
    temporaryRoots.push(root);
    fs.writeFileSync(path.join(root, "release.json"), JSON.stringify({
      "artist-credit": [{ artist: { name: "Artist" } }], media: [{ position: 1, tracks: [
        { position: 1, title: "Song", length: null, recording: { length: 210000 } },
        { position: 2, title: "Song (Edit)", length: 120000, recording: { length: 210000 } },
      ] }],
    }));
    fs.writeFileSync(path.join(root, "corpus.json"), JSON.stringify({ corpusVersion: "test", cases: [
      { caseId: "case", artist: "Artist", tracks: [
        { title: "Song", duration: 208.155 }, { title: "Song (Edit)", duration: 120 },
      ] },
    ] }));
    fs.writeFileSync(path.join(root, "manifest.json"), JSON.stringify({ schemaVersion: 1, cases: [
      { caseId: "case", status: "verified_match", provider: "musicbrainz", releaseId: "release", snapshot: "release.json" },
    ] }));
    execFileSync("python3", [scriptPath, "--corpus", path.join(root, "corpus.json"),
      "--manifest", path.join(root, "manifest.json"), "--fixture-root", root,
      "--output-dir", path.join(root, "out")], { stdio: "pipe" });
    const result = JSON.parse(fs.readFileSync(path.join(root, "out/review.json"), "utf8"));
    expect(result.cases[0]).toMatchObject({ flags: [], mapping: [
      { localTrack: 1, durationDeltaSeconds: 1.845 }, { localTrack: 2, durationDeltaSeconds: 0 },
    ] });
  });

  it("replays the CJK and holdout reviews without blessing the unresolved soundtrack", () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "soundrobe-subset-expansion-"));
    temporaryRoots.push(root);
    const groups = [
      { manifest: "reviewed-billie-20261108.json", corpus: "corpus.json", matches: 6, tracks: 7, unresolved: 0 },
      { manifest: "reviewed-cjk-artist-20261108.json", corpus: "artist-batch-extension/corpus.json", matches: 2, tracks: 15, unresolved: 0 },
      { manifest: "reviewed-cjk-library-20261108.json", corpus: "library-sample/corpus.json", matches: 1, tracks: 12, unresolved: 1 },
    ];
    for (const group of groups) {
      const output = path.join(root, group.manifest);
      execFileSync("python3", [scriptPath, "--corpus", path.join(fixtureRoot, group.corpus),
        "--manifest", path.join(fixtureRoot, group.manifest), "--fixture-root", fixtureRoot,
        "--output-dir", output], { stdio: "pipe" });
      const result = JSON.parse(fs.readFileSync(path.join(output, "review.json"), "utf8"));
      expect(result.statusCounts).toEqual({ verified_match: group.matches, unresolved: group.unresolved });
      expect(result.final).toEqual({ strongMatches: group.tracks, completeCases: group.matches });
      for (const row of result.cases) {
        if (row.status === "unresolved") {
          expect(row.mapping).toEqual([]);
          expect(row.acceptableEditionIds).toEqual([]);
          continue;
        }
        expect(new Set(row.mapping.map((item: { providerTrack: string }) => item.providerTrack)).size)
          .toBe(row.trackCount);
        expect(row.mapping.every((item: { durationDeltaSeconds: number | null }) =>
          item.durationDeltaSeconds !== null && item.durationDeltaSeconds <= 2.021)).toBe(true);
      }
    }
    const manifest = JSON.parse(fs.readFileSync(path.join(fixtureRoot, groups[0].manifest), "utf8"));
    for (const caseId of ["0e7c8a578cea", "e61daee1518d"]) {
      const row = manifest.cases.find((item: { caseId: string }) => item.caseId === caseId);
      const wrong = manifest.cases.find((item: { releaseId: string }) => item.releaseId === row.hardNegativeIds[0]);
      const negativeManifest = path.join(root, "negative.json");
      fs.writeFileSync(negativeManifest, JSON.stringify({ ...manifest, cases: [
        { ...row, releaseId: wrong.releaseId, snapshot: wrong.snapshot },
      ] }));
      expect(() => execFileSync("python3", [scriptPath, "--corpus", corpusPath,
        "--manifest", negativeManifest, "--fixture-root", fixtureRoot,
        "--output-dir", path.join(root, "negative")], { stdio: "pipe" })).toThrow(/not complete/);
    }
  });
});
