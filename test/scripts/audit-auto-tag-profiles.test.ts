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
    expect(() => execFileSync("python3", [
      scriptPath,
      "--corpus", corpus,
      "--expectations", expectations,
      "--output-dir", output,
      "--native", `assisted_without_ids=${native}`,
      "--run-id", "profile-test",
    ], { encoding: "utf8", stdio: "pipe" })).toThrow();
    const result = JSON.parse(fs.readFileSync(path.join(output, "profiles.json"), "utf8"));
    expect(result).toMatchObject({ caseCount: 2, trackCount: 3 });
    expect(result.profiles.folder_filename).toMatchObject({ caseCount: 2, trackCount: 3, nativeInvocations: 0, reviewedEligibleCases: 1 });
    expect(result.profiles.assisted_without_ids).toMatchObject({
      nativeInvocations: 1,
      expectedNativeInvocations: 4,
      nativeComplete: false,
      providerUnavailableInvocations: 1,
      correctMatches: 0,
      wrongMatches: 0,
      safeAbstentions: 0,
      coverage: 0,
      incompleteCases: 2,
      readbackFailures: 0,
      payloadFailures: 0,
    });
    expect(result.profiles.tagged_recovery.nativeInvocations).toBe(0);
    expect(fs.readFileSync(path.join(output, "profiles.md"), "utf8")).toContain("assisted_without_ids");
    expect(fs.readFileSync(path.join(output, "command.log"), "utf8")).toContain("status=incomplete");
  });

  it("reports reviewed outcomes and integrity failures for complete native phases", () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "soundrobe-profiles-complete-"));
    temporaryRoots.push(root);
    const corpus = path.join(root, "corpus.json");
    const expectations = path.join(root, "expectations.json");
    const cases = ["match", "abstain", "wrong", "incomplete", "integrity"].map((caseId) => ({
      caseId,
      tracks: [{}],
    }));
    fs.writeFileSync(corpus, JSON.stringify({
      corpusVersion: "test",
      profiles: {
        folder_filename: { purpose: "clean", strips: [] },
        assisted_without_ids: { purpose: "assisted", strips: [] },
        tagged_recovery: { purpose: "recovery", strips: [] },
      },
      cases,
    }), "utf8");
    fs.writeFileSync(expectations, JSON.stringify({ cases: [
      { caseId: "match", status: "verified_match", acceptableEditionIds: ["release-1"] },
      { caseId: "abstain", status: "verified_abstain", acceptableEditionIds: [] },
      { caseId: "wrong", status: "verified_match", acceptableEditionIds: ["release-1"], rejectedHardNegativeIds: ["hard-negative"] },
      { caseId: "incomplete", status: "unverified", acceptableEditionIds: [] },
      { caseId: "integrity", status: "verified_match", acceptableEditionIds: ["release-1"] },
    ] }), "utf8");
    const native = path.join(root, "results.json");
    const invocations = cases.flatMap(({ caseId }) => ["cold", "warm"].map((phase) => ({
      caseId,
      phase,
      classification: caseId === "abstain" ? "safe_abstention" : caseId === "incomplete" ? "incomplete" : caseId === "integrity" ? "failed_verification" : "confirmed_success",
      selectedIdentity: caseId === "match" ? "release-1" : caseId === "wrong" ? "hard-negative" : null,
      ...(caseId === "match" && phase === "cold" ? { native: { providerAttempts: [{ status: "unavailable" }] } } : {}),
      ...(caseId === "integrity" && phase === "cold" ? { readback: false, payloadUnchanged: false } : {}),
    })));
    fs.writeFileSync(native, JSON.stringify({
      invocations,
      folderResults: [
        { caseId: "match", classification: "confirmed_success", selectedIdentity: "release-1" },
        { caseId: "abstain", classification: "safe_abstention", selectedIdentity: null },
        { caseId: "wrong", classification: "confirmed_success", selectedIdentity: "hard-negative" },
        { caseId: "incomplete", classification: "incomplete", selectedIdentity: null },
        { caseId: "integrity", classification: "failed_verification", selectedIdentity: null },
      ],
    }), "utf8");
    const output = path.join(root, "out");
    execFileSync("python3", [
      scriptPath,
      "--corpus", corpus,
      "--expectations", expectations,
      "--output-dir", output,
      "--native", `folder_filename=${native}`,
      "--run-id", "profile-complete-test",
    ], { encoding: "utf8" });

    const result = JSON.parse(fs.readFileSync(path.join(output, "profiles.json"), "utf8"));
    expect(result.profiles.folder_filename).toMatchObject({
      nativeInvocations: 10,
      expectedNativeInvocations: 10,
      nativeComplete: true,
      correctMatches: 2,
      wrongMatches: 1,
      safeAbstentions: 1,
      coverage: 0.5,
      precision: 2 / 3,
      incompleteCases: 1,
      failedVerificationCases: 1,
      providerUnavailableInvocations: 1,
      mappingFailures: 1,
      readbackFailures: 1,
      payloadFailures: 1,
    });
    expect(fs.readFileSync(path.join(output, "command.log"), "utf8")).toContain("status=passed");
  });
});
