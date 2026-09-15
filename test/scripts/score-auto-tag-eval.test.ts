// @vitest-environment node
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";

const repoRoot = path.resolve(__dirname, "../..");
const scriptPath = path.join(repoRoot, "scripts/score_auto_tag_eval.py");
const temporaryRoots: string[] = [];

afterEach(() => {
  for (const root of temporaryRoots.splice(0)) {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

describe("score_auto_tag_eval.py", () => {
  it("scores reviewed cases and separates recovery from matcher attribution", () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "soundrobe-score-eval-"));
    temporaryRoots.push(root);
    const writeJson = (name: string, value: unknown) => {
      const file = path.join(root, name);
      fs.writeFileSync(file, JSON.stringify(value), "utf8");
      return file;
    };
    const corpus = writeJson("corpus.json", {
      corpusVersion: "test-corpus",
      cases: [
        { caseId: "match", releaseGroupId: "group-match" },
        { caseId: "abstain", releaseGroupId: "group-abstain" },
        { caseId: "wrong", releaseGroupId: "group-wrong" },
        { caseId: "unverified", releaseGroupId: "group-unverified" },
      ],
    });
    const expectations = writeJson("expectations.json", {
      cases: [
        {
          caseId: "match",
          status: "verified_match",
          acceptableEditionIds: ["release-1"],
          rejectedHardNegativeIds: [],
          matcherAttribution: true,
        },
        {
          caseId: "abstain",
          status: "verified_abstain",
          acceptableEditionIds: [],
          rejectedHardNegativeIds: [],
        },
        {
          caseId: "wrong",
          status: "verified_match",
          acceptableEditionIds: ["release-2"],
          rejectedHardNegativeIds: ["hard-negative"],
        },
        {
          caseId: "unverified",
          status: "unverified",
          acceptableEditionIds: [],
          rejectedHardNegativeIds: [],
        },
      ],
    });
    const results = writeJson("results.json", {
      folderResults: [
        {
          caseId: "match",
          classification: "confirmed_success",
          selectedIdentity: "release-1",
        },
        { caseId: "abstain", classification: "safe_abstention", selectedIdentity: null },
        {
          caseId: "wrong",
          classification: "confirmed_success",
          selectedIdentity: "hard-negative",
        },
        {
          caseId: "unverified",
          classification: "failed_verification",
          coldIdentity: "provider-old",
          warmIdentity: "provider-new",
          sourceRelativeFolder: "artist/unreviewed",
          coldClassification: "incomplete",
          warmClassification: "unresolved",
        },
      ],
      invocations: [
        {
          caseId: "match",
          phase: "cold",
          classification: "incomplete",
          selectedIdentity: "release-1",
          selectedTrackEvidence: { evidence: ["GuardedTitle"] },
          native: { providerAttempts: [{ status: "unavailable" }] },
        },
        {
          caseId: "match",
          phase: "warm",
          classification: "confirmed_success",
          selectedIdentity: "release-1",
          selectedTrackEvidence: { evidence: ["GuardedTitle"] },
          native: { providerAttempts: [] },
        },
        { caseId: "abstain", phase: "cold", classification: "safe_abstention", selectedIdentity: null },
        { caseId: "abstain", phase: "warm", classification: "safe_abstention", selectedIdentity: null },
        { caseId: "wrong", phase: "cold", classification: "confirmed_success", selectedIdentity: "old" },
        { caseId: "wrong", phase: "warm", classification: "confirmed_success", selectedIdentity: "hard-negative" },
      ],
    });
    const reviewedTruth = writeJson("reviewed-truth.json", {
      source: "relapse",
      status: "verified_match",
      acceptableEditionIds: ["36441795"],
      rejectedHardNegativeIds: ["16649340"],
      mapping: [{ localTrack: 1, providerTrack: 1 }, { localTrack: 2, providerTrack: 2 }],
      baseline: { strongTitleMatches: 1, positionOnlyMatches: 1 },
      postFix: { strongMatches: 2, positionOnlyMatches: 0, guardedTitleMatches: 1 },
    });
    const output = path.join(root, "out");
    execFileSync("python3", [
      scriptPath,
      "--corpus", corpus,
      "--expectations", expectations,
      "--results", results,
      "--reviewed-truth", reviewedTruth,
      "--output-dir", output,
      "--run-id", "score-test",
    ], { encoding: "utf8" });

    const score = JSON.parse(fs.readFileSync(path.join(output, "score.json"), "utf8"));
    expect(score.scored).toMatchObject({
      eligibleCases: 3,
      correct: 2,
      wrongMatch: 1,
      unresolved: 0,
      incomplete: 0,
      providerUnavailableCases: 1,
      warmRecoveryCases: 1,
      coldWarmIdentityInconsistencies: 1,
      nativeMatcherAttributionCases: 1,
    });
    expect(score.scored.diagnostic).toMatchObject({
      providerUnavailableCases: 1,
      warmRecoveryCases: 1,
      guardedTitleInvocationRecords: 2,
      coldWarmIdentityInconsistencies: [
        expect.objectContaining({
          caseId: "unverified",
          coldIdentity: "provider-old",
          warmIdentity: "provider-new",
        }),
      ],
    });
    expect(score.scored.coverage).toBeCloseTo(2 / 3);
    expect(score.scored.cases).toEqual([
      expect.objectContaining({ caseId: "abstain", outcome: "correct" }),
      expect.objectContaining({ caseId: "match", outcome: "correct", selectedIdentity: "release-1" }),
      expect.objectContaining({ caseId: "wrong", outcome: "wrong_match", selectedIdentity: "hard-negative" }),
    ]);
    expect(score.inputSha256).toEqual({
      corpus: expect.stringMatching(/^[a-f0-9]{64}$/),
      expectations: expect.stringMatching(/^[a-f0-9]{64}$/),
      results: expect.stringMatching(/^[a-f0-9]{64}$/),
      reviewedTruth: expect.stringMatching(/^[a-f0-9]{64}$/),
    });
    expect(score.reviewedTruth.matcherAttribution).toBe(true);
    expect(score.reviewedTruth.delta.strongEvidenceMatches).toBe(1);
    expect(fs.readFileSync(path.join(output, "score.md"), "utf8")).toContain(
      "Unreviewed corpus cases remain outside precision and coverage",
    );
  });
});
