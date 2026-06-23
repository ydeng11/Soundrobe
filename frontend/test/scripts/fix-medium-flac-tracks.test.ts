// @vitest-environment node
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

const repoRoot = path.resolve(__dirname, "../../..");
const scriptPath = path.join(repoRoot, "scripts/fix-medium-flac-tracks.js");

function writeReport(reportPath: string, sourceRoot: string) {
  fs.mkdirSync(path.dirname(reportPath), { recursive: true });
  fs.writeFileSync(
    reportPath,
    JSON.stringify({
      generatedAt: "2026-06-19T00:00:00.000Z",
      mode: "doctor",
      sourceRoot,
      summary: { total: 4, clean: 1, minor: 1, medium: 2, broken: 0, fixed: 0, fixFailed: 0 },
      diagnosis: {},
      details: {
        "Artist/Album/playable-invalid.flac": {
          artist: "Artist",
          album: "Album",
          track: "playable-invalid.flac",
          relativePath: "Artist/Album/playable-invalid.flac",
          bucket: "medium",
          issues: ["strict-decode-invalid-playable"],
          knownRepairs: [],
        },
        "Artist/Album/warning-only.flac": {
          artist: "Artist",
          album: "Album",
          track: "warning-only.flac",
          relativePath: "Artist/Album/warning-only.flac",
          bucket: "medium",
          issues: ["strict-decode-warning"],
          knownRepairs: [],
        },
        "Artist/Album/minor.flac": {
          artist: "Artist",
          album: "Album",
          track: "minor.flac",
          relativePath: "Artist/Album/minor.flac",
          bucket: "minor",
          issues: ["metadata-audio-gap"],
          knownRepairs: ["metadata-audio-gap"],
        },
      },
    }, null, 2),
  );
}

function runScript(args: string[], env: NodeJS.ProcessEnv = {}) {
  return execFileSync("node", [scriptPath, ...args], {
    encoding: "utf8",
    env: { ...process.env, ...env },
  });
}

describe("fix-medium-flac-tracks script", () => {
  let tmpDir: string;
  let libraryDir: string;
  let reportPath: string;
  let workDir: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "fix-medium-flac-"));
    libraryDir = path.join(tmpDir, "library");
    reportPath = path.join(tmpDir, "doctor.json");
    workDir = path.join(tmpDir, "work");
    fs.mkdirSync(path.join(libraryDir, "Artist", "Album"), { recursive: true });
    for (const name of ["playable-invalid.flac", "warning-only.flac", "minor.flac"]) {
      fs.writeFileSync(path.join(libraryDir, "Artist", "Album", name), `fake ${name}`);
    }
    writeReport(reportPath, libraryDir);
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  it("dry-runs only playable strict-invalid medium tracks by default", () => {
    const output = runScript([libraryDir, "--from-report", reportPath, "--work-dir", workDir]);

    expect(output).toContain("Mode: dry-run");
    expect(output).toContain("Candidates: 1");
    expect(output).toContain("Artist/Album/playable-invalid.flac");
    expect(output).not.toContain("Artist/Album/warning-only.flac");

    const plan = JSON.parse(fs.readFileSync(path.join(workDir, "medium-fix-plan.json"), "utf8"));
    expect(plan.apply).toBe(false);
    expect(plan.candidates.map((candidate: { relativePath: string }) => candidate.relativePath)).toEqual([
      "Artist/Album/playable-invalid.flac",
    ]);
  });

  it("can include warning-only medium tracks when explicitly requested", () => {
    runScript([libraryDir, "--from-report", reportPath, "--work-dir", workDir, "--include-warnings"]);

    const plan = JSON.parse(fs.readFileSync(path.join(workDir, "medium-fix-plan.json"), "utf8"));
    expect(plan.candidates.map((candidate: { relativePath: string }) => candidate.relativePath)).toEqual([
      "Artist/Album/playable-invalid.flac",
      "Artist/Album/warning-only.flac",
    ]);
  });

  it("requires --yes before apply mutates files", () => {
    expect(() => runScript([libraryDir, "--from-report", reportPath, "--work-dir", workDir, "--apply"]))
      .toThrow(/Refusing to apply without --yes/);
  });

  it("refuses a reused report for a different source root", () => {
    const otherLibrary = path.join(tmpDir, "other-library");
    fs.mkdirSync(otherLibrary, { recursive: true });

    expect(() => runScript([otherLibrary, "--from-report", reportPath, "--work-dir", workDir]))
      .toThrow(/report sourceRoot does not match/i);
  });

  it("applies recovery through ffmpeg, validates the temp FLAC, and leaves non-candidates untouched", () => {
    const binDir = path.join(tmpDir, "bin");
    fs.mkdirSync(binDir, { recursive: true });
    fs.writeFileSync(path.join(binDir, "ffmpeg"), `#!/bin/sh
out=""
for arg in "$@"; do out="$arg"; done
printf "recovered" > "$out"
exit 0
`, { mode: 0o755 });
    fs.writeFileSync(path.join(binDir, "flac"), `#!/bin/sh
exit 0
`, { mode: 0o755 });
    const fakePath = `${binDir}${path.delimiter}${process.env.PATH ?? ""}`;

    const output = runScript([
      libraryDir,
      "--from-report", reportPath,
      "--work-dir", workDir,
      "--apply",
      "--yes",
      "--skip-post-doctor",
    ], { PATH: fakePath });

    expect(output).toContain("Recovered: 1");
    expect(fs.readFileSync(path.join(libraryDir, "Artist", "Album", "playable-invalid.flac"), "utf8")).toBe("recovered");
    expect(fs.readFileSync(path.join(libraryDir, "Artist", "Album", "warning-only.flac"), "utf8")).toBe("fake warning-only.flac");
    expect(fs.readFileSync(path.join(libraryDir, "Artist", "Album", "minor.flac"), "utf8")).toBe("fake minor.flac");

    const results = JSON.parse(fs.readFileSync(path.join(workDir, "medium-fix-results.json"), "utf8"));
    expect(results.recovered).toHaveLength(1);
    expect(results.failed).toHaveLength(0);
  });
});
