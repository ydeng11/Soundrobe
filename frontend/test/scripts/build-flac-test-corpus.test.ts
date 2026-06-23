// @vitest-environment node
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

const repoRoot = path.resolve(__dirname, "../../..");
const scriptPath = path.join(repoRoot, "scripts/build-flac-test-corpus.js");

function writeFile(filePath: string, contents = "fake flac") {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, contents);
}

function runBuilder(args: string[]) {
  return execFileSync("node", [scriptPath, ...args], {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });
}

describe("build-flac-test-corpus script", () => {
  let tmpDir: string;
  let sourceDir: string;
  let destDir: string;
  let reportsDir: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "build-flac-corpus-"));
    sourceDir = path.join(tmpDir, "source");
    destDir = path.join(tmpDir, "auto-tagger-flac-corpus");
    reportsDir = path.join(tmpDir, "reports");
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  it("copies targeted report files first and preserves relative paths", () => {
    writeFile(path.join(sourceDir, "Artist A", "Album 1", "target.flac"), "target");
    writeFile(path.join(sourceDir, "Artist B", "Album 2", "random.flac"), "random");
    fs.mkdirSync(reportsDir, { recursive: true });
    fs.writeFileSync(
      path.join(reportsDir, "flac-metadata-report-2026-06-19T04-02-17-254Z.txt"),
      [
        "  ALERT_STRICT_DECODE_FAIL  Artist A/Album 1/target.flac  (Strict decode failed after 100 samples)",
        "  WARNING_STRICT_DECODE  Missing Artist/Missing Album/missing.flac  (Playable but strict decode reported a warning)",
      ].join("\n"),
    );

    runBuilder([
      "--source", sourceDir,
      "--dest", destDir,
      "--reports-dir", reportsDir,
      "--count", "2",
      "--seed", "seed-a",
    ]);

    const manifest = JSON.parse(fs.readFileSync(path.join(destDir, "manifest.json"), "utf8"));
    expect(manifest.entries).toHaveLength(2);
    expect(manifest.entries[0]).toMatchObject({
      relativePath: "Artist A/Album 1/target.flac",
      reason: "targeted",
      tags: ["strict-alert"],
    });
    expect(fs.readFileSync(path.join(destDir, "Artist A", "Album 1", "target.flac"), "utf8")).toBe("target");
    expect(manifest.missingTargets).toEqual([
      {
        relativePath: "Missing Artist/Missing Album/missing.flac",
        tags: ["strict-warning"],
      },
    ]);
  });

  it("uses deterministic random ordering for the same seed", () => {
    for (const name of ["one", "two", "three", "four", "five"]) {
      writeFile(path.join(sourceDir, "Artist", "Album", `${name}.flac`), name);
    }
    fs.mkdirSync(reportsDir, { recursive: true });

    runBuilder([
      "--source", sourceDir,
      "--dest", destDir,
      "--reports-dir", reportsDir,
      "--count", "3",
      "--seed", "same-seed",
    ]);
    const first = JSON.parse(fs.readFileSync(path.join(destDir, "manifest.json"), "utf8"))
      .entries.map((entry: { relativePath: string }) => entry.relativePath);

    runBuilder([
      "--source", sourceDir,
      "--dest", destDir,
      "--reports-dir", reportsDir,
      "--count", "3",
      "--seed", "same-seed",
    ]);
    const second = JSON.parse(fs.readFileSync(path.join(destDir, "manifest.json"), "utf8"))
      .entries.map((entry: { relativePath: string }) => entry.relativePath);

    expect(second).toEqual(first);
  });

  it("refuses to write the corpus inside the source tree", () => {
    writeFile(path.join(sourceDir, "Artist", "Album", "one.flac"));
    const unsafeDest = path.join(sourceDir, "auto-tagger-flac-corpus");

    expect(() => runBuilder([
      "--source", sourceDir,
      "--dest", unsafeDest,
      "--reports-dir", reportsDir,
      "--count", "1",
    ])).toThrow(/refusing to write destination inside source/i);
  });
});
