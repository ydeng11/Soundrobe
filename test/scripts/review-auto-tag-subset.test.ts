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
      providerSnapshotCount: 17,
      statusCounts: { verified_match: 17, unresolved: 13 },
      baseline: { exactTitleMatches: 153, trackCount: 197, completeCases: 3 },
      final: { strongMatches: 197, completeCases: 17 },
    });
    expect(result.cases.find((item: { caseId: string }) => item.caseId === "e8a8a8d6b27d")).toMatchObject({
      status: "verified_match",
      flags: ["normalized-title"],
    });
    expect(result.cases.find((item: { caseId: string }) => item.caseId === "b7fbe4f8ccd1")).toMatchObject({
      status: "unresolved",
      flags: ["unresolved"],
    });
    expect(fs.readFileSync(path.join(output, "review.md"), "utf8")).toContain("Explicitly unresolved: 13");
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
});
