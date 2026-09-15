// @vitest-environment node
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";

const repoRoot = path.resolve(__dirname, "../..");
const scriptPath = path.join(repoRoot, "scripts/generate_auto_tag_eval_corpus.py");
const fixturePath = path.join(repoRoot, "test/fixtures/tauri/media-corpus/minimal.wav");
const temporaryRoots: string[] = [];

afterEach(() => {
  for (const root of temporaryRoots.splice(0)) fs.rmSync(root, { recursive: true, force: true });
});

describe("generate_auto_tag_eval_corpus.py", () => {
  it.each([
    ["--source-root", (source: string) => ["--source-root", source], {}],
    ["SOUNDROBE_AUTO_TAG_EVAL_SOURCE_ROOT", (source: string) => [], { useEnv: true }],
  ])("accepts %s and writes relative corpus paths", (_label, argsFor, options) => {
    const root = fs.mkdtempSync(path.join(os.tmpdir(), "soundrobe-generate-corpus-"));
    temporaryRoots.push(root);
    const source = path.join(root, "curated");
    const album = path.join(source, "Ariana Grande", "Test Album");
    fs.mkdirSync(album, { recursive: true });
    fs.copyFileSync(fixturePath, path.join(album, "01.wav"));
    fs.mkdirSync(path.join(root, "test/fixtures/tauri/auto-tag-eval"), { recursive: true });

    execFileSync("python3", [scriptPath, ...argsFor(source)], {
      cwd: root,
      env: options.useEnv
        ? { ...process.env, SOUNDROBE_AUTO_TAG_EVAL_SOURCE_ROOT: source }
        : process.env,
      encoding: "utf8",
    });

    const corpus = JSON.parse(fs.readFileSync(
      path.join(root, "test/fixtures/tauri/auto-tag-eval/corpus.json"),
      "utf8",
    ));
    expect(corpus.sourceRoot).toBe(".");
    expect(corpus.cases).toHaveLength(1);
    expect(corpus.cases[0].sourceRelativeFolder).toBe("Ariana Grande/Test Album");
    expect(corpus.cases[0].sourceRelativeFolder).not.toContain("/Users/");
  });
});
