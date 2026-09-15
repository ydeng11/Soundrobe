// @vitest-environment node
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";

const repoRoot = path.resolve(__dirname, "../..");
const scriptPath = path.join(repoRoot, "scripts/audit_auto_tag_ground_truth.py");
const temporaryRoots: string[] = [];

afterEach(() => {
  for (const root of temporaryRoots.splice(0)) {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

describe("audit_auto_tag_ground_truth.py", () => {
  it("reconciles every case and keeps unreviewed cases explicitly unscored", () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "soundrobe-ground-truth-"));
    temporaryRoots.push(root);
    const writeJson = (name: string, value: unknown) => {
      const file = path.join(root, name);
      fs.writeFileSync(file, JSON.stringify(value), "utf8");
      return file;
    };
    const corpus = writeJson("corpus.json", {
      schemaVersion: 1,
      corpusVersion: "test-corpus",
      sourceRoot: "/tmp/source",
      cases: [
        {
          caseId: "match",
          artist: "Artist",
          releaseGroupId: "artist:group",
          sourceRelativeFolder: "Artist/Album",
          tracks: [{ trackNumber: 1 }, { trackNumber: 2 }],
        },
        {
          caseId: "pending",
          artist: "Artist",
          releaseGroupId: "artist:pending",
          sourceRelativeFolder: "Artist/Pending",
          tracks: [{ trackNumber: 1 }],
        },
      ],
    });
    const expectations = writeJson("expectations.json", {
      schemaVersion: 1,
      corpusVersion: "test-corpus",
      cases: [
        {
          caseId: "match",
          artist: "Artist",
          releaseGroupId: "artist:group",
          status: "verified_match",
          provenance: "reviewed fixture",
          rationale: "complete provider mapping",
          acceptableEditionIds: ["release-1"],
          rejectedHardNegativeIds: ["release-bad"],
          mapping: [
            { localTrack: 1, providerTrack: 1 },
            { localTrack: 2, providerTrack: 2 },
          ],
        },
        {
          caseId: "pending",
          artist: "Artist",
          releaseGroupId: "artist:pending",
          status: "unverified",
          provenance: "inventory-only",
          rationale: "provider mapping review pending",
          acceptableEditionIds: [],
          rejectedHardNegativeIds: [],
          mapping: [],
        },
      ],
    });
    const output = path.join(root, "out");
    expect(() => execFileSync("python3", [
      scriptPath,
      "--corpus", corpus,
      "--expectations", expectations,
      "--output-dir", output,
      "--run-id", "ground-truth-test",
    ], { encoding: "utf8", stdio: "pipe" })).toThrow();

    const result = JSON.parse(fs.readFileSync(path.join(output, "ground-truth.json"), "utf8"));
    expect(result).toMatchObject({
      caseCount: 2,
      trackCount: 3,
      scoredCaseCount: 1,
      unscoredCaseCount: 1,
      statusCounts: { unverified: 1, verified_match: 1 },
    });
    expect(result.cases).toEqual([
      expect.objectContaining({ caseId: "match", scored: true, mappingCount: 2 }),
      expect.objectContaining({
        caseId: "pending",
        scored: false,
        blockingReason: "provider-backed edition content and mapping review pending",
      }),
    ]);
    expect(result.inputSha256).toEqual({
      corpus: expect.stringMatching(/^[a-f0-9]{64}$/),
      expectations: expect.stringMatching(/^[a-f0-9]{64}$/),
    });
    expect(fs.readFileSync(path.join(output, "ground-truth.md"), "utf8")).toContain(
      "Every corpus case is represented exactly once",
    );
    expect(fs.readFileSync(path.join(output, "command.log"), "utf8")).toContain("status=incomplete");
  });

  it("rejects path escapes and incomplete reviewed mappings", () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "soundrobe-ground-truth-invalid-"));
    temporaryRoots.push(root);
    const writeJson = (name: string, value: unknown) => {
      const file = path.join(root, name);
      fs.writeFileSync(file, JSON.stringify(value), "utf8");
      return file;
    };
    const expectations = writeJson("expectations.json", {
      schemaVersion: 1,
      corpusVersion: "test-corpus",
      cases: [{
        caseId: "case",
        artist: "Artist",
        releaseGroupId: "artist:group",
        status: "verified_match",
        provenance: "reviewed",
        rationale: "mapping",
        acceptableEditionIds: ["release"],
        rejectedHardNegativeIds: [],
        mapping: [{ localTrack: 1, providerTrack: 1 }],
      }],
    });
    const output = path.join(root, "out");
    const escapedCorpus = writeJson("escaped-corpus.json", {
      schemaVersion: 1,
      corpusVersion: "test-corpus",
      cases: [{
        caseId: "case",
        artist: "Artist",
        releaseGroupId: "artist:group",
        sourceRelativeFolder: "../outside",
        tracks: [{ trackNumber: 1 }],
      }],
    });
    expect(() => execFileSync("python3", [
      scriptPath,
      "--corpus", escapedCorpus,
      "--expectations", expectations,
      "--output-dir", output,
    ], { encoding: "utf8", stdio: "pipe" })).toThrow(/escapes corpus root/);

    const incompleteCorpus = writeJson("incomplete-corpus.json", {
      schemaVersion: 1,
      corpusVersion: "test-corpus",
      cases: [{
        caseId: "case",
        artist: "Artist",
        releaseGroupId: "artist:group",
        sourceRelativeFolder: "Artist/Album",
        tracks: [{ trackNumber: 1 }, { trackNumber: 2 }],
      }],
    });
    expect(() => execFileSync("python3", [
      scriptPath,
      "--corpus", incompleteCorpus,
      "--expectations", expectations,
      "--output-dir", output,
    ], { encoding: "utf8", stdio: "pipe" })).toThrow(/mapping has 1 rows for 2 tracks/);
  });
});
