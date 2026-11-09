// @vitest-environment node
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";

const repoRoot = path.resolve(__dirname, "../..");
const script = path.join(repoRoot, "scripts/build_auto_tag_eval_suite.py");
const fixtureRoot = path.join(repoRoot, "test/fixtures/tauri");
const evalRoot = path.join(fixtureRoot, "auto-tag-eval");
const temporaryRoots: string[] = [];
const args = ["--fixture-root", fixtureRoot,
  "--corpus", path.join(evalRoot, "corpus.json"),
  "--corpus", path.join(evalRoot, "artist-batch-extension/corpus.json"),
  "--corpus", path.join(evalRoot, "curated-hard-cases/corpus.json"),
  "--corpus", path.join(evalRoot, "library-sample/corpus.json"),
  "--reviewed-manifest", path.join(evalRoot, "reviewed-subset.json"),
  "--reviewed-manifest", path.join(evalRoot, "reviewed-billie-20261108.json"),
  "--reviewed-manifest", path.join(evalRoot, "reviewed-cjk-artist-20261108.json"),
  "--reviewed-manifest", path.join(evalRoot, "reviewed-cjk-library-20261108.json"),
  "--reviewed-manifest", path.join(evalRoot, "reviewed-expansion-2-20261108.json"),
  "--reviewed-manifest", path.join(evalRoot, "reviewed-cjk-artist-2-20261108.json"),
  "--reviewed-manifest", path.join(evalRoot, "reviewed-library-2-20261108.json"),
  "--reviewed-manifest", path.join(evalRoot, "reviewed-curated-2-20261108.json"),
  "--candidate-pools", path.join(evalRoot, "candidate-pools.json"),
  "--discovery", path.join(evalRoot, "provider-discovery.json")];

afterEach(() => {
  for (const root of temporaryRoots.splice(0)) fs.rmSync(root, { recursive: true, force: true });
});

describe("build_auto_tag_eval_suite.py", () => {
  it("locks the expanded inputs and separates inventory from scored provider truth", () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "soundrobe-eval-suite-"));
    temporaryRoots.push(root);
    const output = path.join(root, "suite.json");
    execFileSync("python3", [script, ...args, "--output", output], { stdio: "pipe" });
    const text = fs.readFileSync(output, "utf8");
    const suite = JSON.parse(text);
    expect(suite.inputs.map((item: { path: string }) => item.path)).toEqual(expect.arrayContaining([
      "auto-tag-eval/curated-hard-cases/selection.json", "auto-tag-eval/library-sample/selection.json",
    ]));
    expect(suite.cases.find((item: { sourceRelativeFolder: string }) => item.sourceRelativeFolder.endsWith("Make Me Say It Again, Girl")))
      .toMatchObject({ difficultyLabels: ["ordered_credits", "group_name"], selectionRationale: expect.stringContaining("one act") });
    expect(text).toBe(fs.readFileSync(path.join(evalRoot, "batch-suite.json"), "utf8"));
    expect(suite.summary).toMatchObject({ cases: 378, tracks: 3506, scoredCases: 51,
      unscoredCases: 327, batches: 27, compilationBatches: 12,
      statusCounts: { verified_match: 51, unresolved: 22, unverified: 305 } });
    for (const caseId of ["036e0a00e45b", "6f2e1cfe0454", "fbd49f56b4d7"]) {
      expect(suite.cases.find((item: { caseId: string }) => item.caseId === caseId))
        .toMatchObject({ status: "verified_match", scored: true });
    }
    for (const caseId of ["2508e8259d36", "b4276f145b7a", "6e2f46055e18"]) {
      expect(suite.cases.find((item: { caseId: string }) => item.caseId === caseId))
        .toMatchObject({ status: "unresolved", scored: false });
    }
    expect(suite.inputs.every((item: { path: string; sha256: string }) =>
      !path.isAbsolute(item.path) && /^[a-f0-9]{64}$/.test(item.sha256))).toBe(true);
    expect(text).not.toContain("/Users/");
    const membership = suite.batches.flatMap((batch: { caseIds: string[] }) => batch.caseIds);
    expect(new Set(membership).size).toBe(378);
    expect(membership).toHaveLength(378);
    for (const batch of suite.batches) {
      expect(["development", "holdout"]).toContain(batch.split);
      if (batch.kind === "compilation_album") expect(batch.discovery).toBe("album_only");
    }
    const families = new Map<string, string>();
    for (const item of suite.cases) {
      const previous = families.get(item.releaseGroupId);
      if (previous) expect(item.split).toBe(previous);
      families.set(item.releaseGroupId, item.split);
    }
    expect(suite.cases.filter((item: { scored: boolean; split: string }) => item.scored && item.split === "holdout"))
      .toHaveLength(8);
    expect(suite.cases.filter((item: { scored: boolean; corpus: string }) => item.scored && item.corpus !== "auto-tag-eval/corpus.json"))
      .toHaveLength(11);
    execFileSync("python3", [script, ...args, "--output", output, "--check"], { stdio: "pipe" });
    expect(fs.readFileSync(output, "utf8")).toBe(text);
    suite.batches[0].caseIds.pop();
    fs.writeFileSync(output, JSON.stringify(suite));
    expect(() => execFileSync("python3", [script, ...args, "--output", output, "--check"],
      { stdio: "pipe" })).toThrow(/suite differs/);
  });

  it("rejects overlapping corpora instead of counting a physical case twice", () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "soundrobe-eval-suite-duplicate-"));
    temporaryRoots.push(root);
    expect(() => execFileSync("python3", [script, ...args,
      "--corpus", path.join(evalRoot, "corpus.json"), "--output", path.join(root, "suite.json")],
    { stdio: "pipe" })).toThrow(/duplicate case/);
  });

  it("combines review manifests without silently replacing earlier answers", () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "soundrobe-eval-suite-reviews-"));
    temporaryRoots.push(root);
    for (const name of ["corpus.json", "expectations.json"]) {
      fs.copyFileSync(path.join(evalRoot, name), path.join(root, name));
    }
    const first = { schemaVersion: 1, corpusVersion: "2026-09-12.inventory-1", cases: [
      { caseId: "07748489dd52", status: "unresolved", reason: "Awaiting independent review." },
    ] };
    const second = { ...first, cases: [
      { ...first.cases[0], caseId: "07e09ba5552a" },
    ] };
    const firstPath = path.join(root, "first.json");
    const secondPath = path.join(root, "second.json");
    fs.writeFileSync(firstPath, JSON.stringify(first));
    fs.writeFileSync(secondPath, JSON.stringify(second));
    const output = path.join(root, "suite.json");
    const localArgs = ["--fixture-root", root, "--corpus", path.join(root, "corpus.json"),
      "--reviewed-manifest", firstPath, "--reviewed-manifest", secondPath, "--output", output];
    execFileSync("python3", [script, ...localArgs], { stdio: "pipe" });
    const suite = JSON.parse(fs.readFileSync(output, "utf8"));
    expect(suite.cases.filter((item: { status: string }) => item.status === "unresolved"))
      .toEqual(expect.arrayContaining([
        expect.objectContaining({ caseId: "07748489dd52", expectationSource: "first.json" }),
        expect.objectContaining({ caseId: "07e09ba5552a", expectationSource: "second.json" }),
      ]));
    expect(suite.inputs.map((item: { path: string }) => item.path))
      .toEqual(expect.arrayContaining(["first.json", "second.json"]));
    second.cases = [Object.assign({}, first.cases[0], { hardNegativeIds: ["contradictory-answer"] })];
    fs.writeFileSync(secondPath, JSON.stringify(second));
    expect(() => execFileSync("python3", [script, ...localArgs], { stdio: "pipe" }))
      .toThrow(/conflicting reviewed answers/);
  });

  it("detects changed inputs and refuses conflicting reviewed answers", () => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "soundrobe-eval-suite-conflict-"));
    temporaryRoots.push(root);
    for (const name of ["corpus.json", "expectations.json"]) {
      fs.copyFileSync(path.join(evalRoot, name), path.join(root, name));
    }
    const output = path.join(root, "suite.json");
    const localArgs = ["--fixture-root", root, "--corpus", path.join(root, "corpus.json"), "--output", output];
    execFileSync("python3", [script, ...localArgs], { stdio: "pipe" });
    const ledgerPath = path.join(root, "expectations.json");
    const ledger = JSON.parse(fs.readFileSync(ledgerPath, "utf8"));
    ledger.cases.find((item: { caseId: string }) => item.caseId === "8452b6f2b3ea").rejectedHardNegativeIds = ["conflicting-review"];
    fs.writeFileSync(ledgerPath, JSON.stringify(ledger));
    expect(() => execFileSync("python3", [script, ...localArgs, "--check"], { stdio: "pipe" })).toThrow(/suite differs/);
    const manifest = JSON.parse(fs.readFileSync(path.join(evalRoot, "reviewed-subset.json"), "utf8"));
    manifest.cases = manifest.cases.filter((item: { caseId: string }) => item.caseId === "8452b6f2b3ea");
    const snapshot = manifest.cases[0].snapshot;
    fs.mkdirSync(path.dirname(path.join(root, snapshot)), { recursive: true });
    fs.copyFileSync(path.join(evalRoot, snapshot), path.join(root, snapshot));
    const manifestPath = path.join(root, "reviewed-subset.json");
    fs.writeFileSync(manifestPath, JSON.stringify(manifest));
    expect(() => execFileSync("python3", [script, ...localArgs, "--reviewed-manifest", manifestPath],
      { stdio: "pipe" })).toThrow(/conflicting reviewed answers/);
  });
});
