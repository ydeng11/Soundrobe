// @vitest-environment node
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";

const repoRoot = path.resolve(__dirname, "../..");
const scriptPath = path.join(repoRoot, "scripts/collect_auto_tag_evidence.py");
const temporaryRoots: string[] = [];

afterEach(() => {
  for (const root of temporaryRoots.splice(0)) fs.rmSync(root, { recursive: true, force: true });
});

function inputs(root: string) {
  const corpus = path.join(root, "corpus.json");
  const expectations = path.join(root, "expectations.json");
  const cases = ["one", "two", "three"].map((caseId) => ({
    caseId,
    tracks: [{ title: caseId }],
  }));
  fs.writeFileSync(corpus, JSON.stringify({ corpusVersion: "test", cases }), "utf8");
  fs.writeFileSync(expectations, JSON.stringify({
    corpusVersion: "test",
    cases: [
      { caseId: "one", status: "verified_match" },
      { caseId: "two", status: "unverified" },
      { caseId: "three", status: "unverified" },
    ],
  }), "utf8");
  return { corpus, expectations };
}

function run(args: string[], env?: NodeJS.ProcessEnv) {
  return JSON.parse(execFileSync("python3", [scriptPath, ...args], { encoding: "utf8", env }));
}

describe("collect_auto_tag_evidence.py", () => {
  it("plans only unscored cases into bounded resumable batches", () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "soundrobe-evidence-plan-"));
    temporaryRoots.push(root);
    const { corpus, expectations } = inputs(root);
    const reviewedManifest = path.join(root, "reviewed.json");
    fs.writeFileSync(reviewedManifest, JSON.stringify({
      cases: [{ caseId: "two", status: "verified_match" }],
    }), "utf8");
    const output = path.join(root, "out");
    expect(run([
      "--corpus", corpus,
      "--expectations", expectations,
      "--reviewed-manifest", reviewedManifest,
      "--output-dir", output,
      "--batch-size", "1",
    ])).toMatchObject({
      caseCount: 1,
      completeCaseCount: 0,
      pendingCaseCount: 1,
      plannedBatchCount: 1,
      runBatchCount: 0,
    });
    const queue = JSON.parse(fs.readFileSync(path.join(output, "queue.json"), "utf8"));
    expect(queue.batches.map((batch: { caseIds: string[] }) => batch.caseIds)).toEqual([["three"]]);
    expect(JSON.parse(fs.readFileSync(path.join(output, "state.json"), "utf8"))).toMatchObject({
      profile: "folder_filename",
      caseIds: ["three"],
      pendingCaseIds: ["three"],
    });
  });

  it("retains provider-complete phases and queues only missing cases", () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "soundrobe-evidence-resume-"));
    temporaryRoots.push(root);
    const { corpus, expectations } = inputs(root);
    const output = path.join(root, "out");
    const batch = path.join(output, "batch-001");
    fs.mkdirSync(batch, { recursive: true });
    const record = (phase: string) => fs.writeFileSync(path.join(batch, `${phase}.jsonl`), JSON.stringify({
      caseId: "two",
      phase,
      outcome: "needs_review",
      native: { providerAttempts: [{ provider: "discogs", status: "no_match" }] },
    }) + "\n", "utf8");
    record("cold");
    record("warm");
    expect(run([
      "--corpus", corpus,
      "--expectations", expectations,
      "--output-dir", output,
      "--batch-size", "4",
    ])).toMatchObject({
      caseCount: 2,
      completeCaseCount: 1,
      pendingCaseCount: 1,
      plannedBatchCount: 1,
    });
    const queue = JSON.parse(fs.readFileSync(path.join(output, "queue.json"), "utf8"));
    expect(queue.batches[0].caseIds).toEqual(["three"]);
    expect(queue.completeCaseCount).toBe(1);
  });

  it("rejects reuse when the corpus or expectations digest changes", () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "soundrobe-evidence-state-"));
    temporaryRoots.push(root);
    const { corpus, expectations } = inputs(root);
    const output = path.join(root, "out");
    run(["--corpus", corpus, "--expectations", expectations, "--output-dir", output]);
    fs.appendFileSync(expectations, "\n", "utf8");
    expect(() => run(["--corpus", corpus, "--expectations", expectations, "--output-dir", output])).toThrow(
      /collector state expectationsSha256 does not match/,
    );
  });

  it("passes only case IDs to the native batch and never seeds release IDs", () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "soundrobe-evidence-native-"));
    temporaryRoots.push(root);
    const { corpus, expectations } = inputs(root);
    const bin = path.join(root, "bin");
    fs.mkdirSync(bin);
    const capture = path.join(root, "cases.txt");
    fs.writeFileSync(path.join(bin, "cargo"), `#!/bin/sh\nprintf '%s|%s' "$SOUNDROBE_AUTO_TAG_EVAL_CASES" "$SOUNDROBE_AUTO_TAG_EVAL_ARTISTS" > "${capture}"\n`, { mode: 0o755 });
    const output = path.join(root, "out");
    const env = { ...process.env, PATH: `${bin}:${process.env.PATH ?? ""}` };
    run([
      "--corpus", corpus,
      "--expectations", expectations,
      "--output-dir", output,
      "--case-ids", "two",
      "--run",
      "--repo-root", root,
    ], env);
    expect(fs.readFileSync(capture, "utf8")).toBe("two|");
    expect(fs.readFileSync(path.join(output, "batch-001", "collector-command.log"), "utf8")).not.toContain("36441795");
  });
});
