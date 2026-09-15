// @vitest-environment node
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";

const repoRoot = path.resolve(__dirname, "../..");
const scriptPath = path.join(repoRoot, "scripts/audit_auto_tag_profiles.py");
const temporaryRoots: string[] = [];

afterEach(() => {
  for (const root of temporaryRoots.splice(0)) fs.rmSync(root, { recursive: true, force: true });
});

describe("audit_auto_tag_profiles.py", () => {
  it("reports all three deterministic profiles and separates partial native work", () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "soundrobe-profiles-"));
    temporaryRoots.push(root);
    const corpus = path.join(root, "corpus.json");
    const expectations = path.join(root, "expectations.json");
    fs.writeFileSync(corpus, JSON.stringify({
      corpusVersion: "test",
      profiles: {
        folder_filename: { purpose: "clean", strips: ["tags", "providerIds"] },
        assisted_without_ids: { purpose: "assisted", strips: ["providerIds"] },
        tagged_recovery: { purpose: "recovery", strips: [] },
      },
      cases: [
        { caseId: "one", tracks: [{}, {}] },
        { caseId: "two", tracks: [{}] },
      ],
    }), "utf8");
    fs.writeFileSync(expectations, JSON.stringify({ cases: [{ caseId: "one", status: "verified_match" }] }), "utf8");
    const native = path.join(root, "cold.jsonl");
    fs.writeFileSync(native, JSON.stringify({ caseId: "one", classification: "incomplete", native: { providerAttempts: [{ status: "unavailable" }] } }) + "\n", "utf8");
    const output = path.join(root, "out");
    execFileSync("python3", [
      scriptPath,
      "--corpus", corpus,
      "--expectations", expectations,
      "--output-dir", output,
      "--native", `assisted_without_ids=${native}`,
      "--run-id", "profile-test",
    ], { encoding: "utf8" });
    const result = JSON.parse(fs.readFileSync(path.join(output, "profiles.json"), "utf8"));
    expect(result).toMatchObject({ caseCount: 2, trackCount: 3 });
    expect(result.profiles.folder_filename).toMatchObject({ caseCount: 2, trackCount: 3, nativeInvocations: 0, reviewedEligibleCases: 1 });
    expect(result.profiles.assisted_without_ids).toMatchObject({ nativeInvocations: 1, expectedNativeInvocations: 4, nativeComplete: false, providerUnavailableInvocations: 1 });
    expect(result.profiles.tagged_recovery.nativeInvocations).toBe(0);
    expect(fs.readFileSync(path.join(output, "profiles.md"), "utf8")).toContain("assisted_without_ids");
  });
});
