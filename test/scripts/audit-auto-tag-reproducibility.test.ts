// @vitest-environment node
import { execFileSync } from "node:child_process";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";

const repoRoot = path.resolve(__dirname, "../..");
const scriptPath = path.join(repoRoot, "scripts/audit_auto_tag_reproducibility.py");
const temporaryRoots: string[] = [];

afterEach(() => {
  for (const root of temporaryRoots.splice(0)) {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

describe("audit_auto_tag_reproducibility.py", () => {
  it("locks frozen responses and reports equivalence separately from identity drift", () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "soundrobe-repro-"));
    temporaryRoots.push(root);
    const fixtureRoot = path.join(root, "fixtures");
    const snapshotRoot = path.join(fixtureRoot, "snapshots");
    fs.mkdirSync(snapshotRoot, { recursive: true });
    const write = (file: string, value: unknown) => {
      fs.writeFileSync(file, JSON.stringify(value), "utf8");
      return crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex");
    };
    const local = path.join(fixtureRoot, "local.json");
    const response = path.join(snapshotRoot, "release.json");
    const localHash = write(local, { tracks: [{ title: "One" }] });
    const responseHash = write(response, { release_id: "release-1", tracks: [] });
    const pools = path.join(fixtureRoot, "candidate-pools.json");
    write(pools, {
      schemaVersion: 1,
      pools: [{
        poolId: "pool",
        localFixture: "local.json",
        localFixtureSha256: localHash,
        candidates: [{
          provider: "discogs",
          releaseId: "release-1",
          response: "snapshots/release.json",
          responseSha256: responseHash,
          expectation: "acceptable",
        }],
      }],
    });
    const equivalence = path.join(root, "equivalence.json");
    write(equivalence, {
      mediaMode: "synthetic_flac",
      source: "production reader",
      allLookupRequestsEquivalent: true,
      cases: [{ caseId: "case", requestEqual: true }],
    });
    const native = path.join(root, "results.json");
    write(native, {
      invocations: [
        { caseId: "case", phase: "cold", native: { providerAttempts: [{ status: "unavailable" }] } },
        { caseId: "case", phase: "warm", native: { providerAttempts: [] } },
      ],
      folderResults: [{
        caseId: "case",
        sourceRelativeFolder: "Artist/Album",
        coldIdentity: "release-old",
        warmIdentity: "release-new",
      }],
    });
    const output = path.join(root, "out");
    execFileSync("python3", [
      scriptPath,
      "--candidate-pools", pools,
      "--equivalence", equivalence,
      "--native-results", native,
      "--output-dir", output,
      "--run-id", "repro-test",
    ], { encoding: "utf8" });
    const result = JSON.parse(fs.readFileSync(path.join(output, "reproducibility.json"), "utf8"));
    expect(result).toMatchObject({
      candidatePoolCount: 1,
      frozenResponseCount: 1,
      candidatePoolHashesValid: true,
      reproducible: false,
      syntheticEquivalence: { caseCount: 1, allLookupRequestsEquivalent: true },
      nativeReplay: {
        invocationCount: 2,
        providerUnavailableCases: 1,
        reconciledAsFailedVerification: 1,
      },
    });
    expect(result.nativeReplay.identityInconsistencies).toEqual([
      expect.objectContaining({
        caseId: "case",
        coldIdentity: "release-old",
        warmIdentity: "release-new",
      }),
    ]);
    expect(fs.readFileSync(path.join(output, "reproducibility.md"), "utf8")).toContain(
      "Identity inconsistencies are reconciled as `failed_verification`",
    );
  });

  it("rejects a changed frozen response", () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "soundrobe-repro-invalid-"));
    temporaryRoots.push(root);
    const fixtures = path.join(root, "fixtures");
    fs.mkdirSync(path.join(fixtures, "snapshots"), { recursive: true });
    fs.writeFileSync(path.join(fixtures, "local.json"), "{}", "utf8");
    const response = path.join(fixtures, "snapshots/release.json");
    fs.writeFileSync(response, "{\"release\":\"changed\"}", "utf8");
    fs.writeFileSync(path.join(fixtures, "candidate-pools.json"), JSON.stringify({
      schemaVersion: 1,
      pools: [{
        poolId: "pool",
        localFixture: "local.json",
        localFixtureSha256: crypto.createHash("sha256").update("{}").digest("hex"),
        candidates: [{
          provider: "discogs",
          releaseId: "release-1",
          response: "snapshots/release.json",
          responseSha256: "0".repeat(64),
        }],
      }],
    }), "utf8");
    const equivalence = path.join(root, "equivalence.json");
    fs.writeFileSync(equivalence, JSON.stringify({ cases: [], allLookupRequestsEquivalent: true }), "utf8");
    expect(() => execFileSync("python3", [
      scriptPath,
      "--candidate-pools", path.join(fixtures, "candidate-pools.json"),
      "--equivalence", equivalence,
      "--output-dir", path.join(root, "out"),
    ], { encoding: "utf8", stdio: "pipe" })).toThrow(/SHA-256 mismatch/);
  });
});
