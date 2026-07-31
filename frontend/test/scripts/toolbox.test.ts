// @vitest-environment node
import { execFileSync, spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

const repoRoot = path.resolve(__dirname, "../../..");
const toolboxPath = path.join(repoRoot, "scripts/toolbox.sh");

function hasCommand(command: string): boolean {
  try {
    execFileSync(command, ["-version"], { stdio: "ignore" });
    return true;
  } catch {
    return false;
  }
}

function writeFile(filePath: string, contents = "") {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, contents);
}

function makeCue(albumDir: string, flacName: string, tracks: Array<[number, string, string]>) {
  // tracks: [trackNum, title, index01]
  const lines = [
    `PERFORMER "Test Artist"`,
    `TITLE "${path.basename(albumDir)}"`,
    `FILE "${flacName}" WAVE`,
  ];
  for (const [num, title, index01] of tracks) {
    lines.push(`  TRACK ${String(num).padStart(2, "0")} AUDIO`);
    lines.push(`    TITLE "${title}"`);
    lines.push(`    INDEX 01 ${index01}`);
  }
  writeFile(path.join(albumDir, "album.cue"), lines.join("\n") + "\n");
}

function makeSineFlac(filePath: string, seconds: number) {
  if (!hasCommand("ffmpeg")) {
    throw new Error("ffmpeg is required for the synthetic cue-split test");
  }
  execFileSync(
    "ffmpeg",
    ["-y", "-f", "lavfi", "-i", `sine=frequency=440:duration=${seconds}`, "-c:a", "flac", filePath],
    { stdio: "ignore" },
  );
}

function runTool(args: string[]) {
  const result = spawnSync("bash", [toolboxPath, ...args], {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });
  return { status: result.status, stdout: result.stdout ?? "", stderr: result.stderr ?? "" };
}

describe("toolbox.sh dispatcher", () => {
  it("prints usage and exits 0 with no arguments", () => {
    const r = runTool([]);
    expect(r.status).toBe(0);
    expect(r.stdout).toContain("Usage:");
    expect(r.stdout).toContain("cue-split");
  });

  it("exits 1 for an unknown command", () => {
    const r = runTool(["frobnicate"]);
    expect(r.status).toBe(1);
    expect(r.stdout + r.stderr).toContain("unknown command");
  });

  it("rejects the removed fix-medium command", () => {
    const r = runTool(["fix-medium"]);
    expect(r.status).toBe(1);
    expect(r.stdout + r.stderr).toContain("unknown command");
  });

  it("prints command help for cue-split -h", () => {
    const r = runTool(["cue-split", "-h"]);
    expect(r.status).toBe(0);
    expect(r.stdout).toContain("Usage:");
    expect(r.stdout).toContain("cue-split");
  });
});

describe("toolbox.sh cue-split discovery", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "toolbox-cuesplit-"));
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  it("finds only the top-level cue without -r, and nested cues with -r", () => {
    const root = path.join(tmpDir, "artist");
    const nested = path.join(root, "Album");
    fs.mkdirSync(nested, { recursive: true });
    makeSineFlac(path.join(nested, "X.flac"), 10);
    makeCue(nested, "X.flac", [
      [1, "one", "00:00:00"],
      [2, "two", "00:05:00"],
    ]);

    const nonRecursive = runTool(["cue-split", "-n", root]);
    expect(nonRecursive.status).toBe(1);
    expect(nonRecursive.stdout).toContain("No .cue files found");

    const recursive = runTool(["cue-split", "-n", "-r", root]);
    expect(recursive.status).toBe(0);
    expect(recursive.stdout).toContain("01. one.flac");
    expect(recursive.stdout).toContain("02. two.flac");
  });

  it("rejects --output when multiple albums would resolve", () => {
    const root = path.join(tmpDir, "artist");
    writeFile(path.join(root, "AlbumA", "a.cue"));
    writeFile(path.join(root, "AlbumB", "b.cue"));

    const r = runTool(["cue-split", "-o", path.join(tmpDir, "out"), "-r", root]);
    expect(r.status).toBe(1);
    expect(r.stdout).toContain("ambiguous");
  });

  it("fails cleanly when a track is missing INDEX 01", () => {
    const album = path.join(tmpDir, "BadCue");
    fs.mkdirSync(album, { recursive: true });
    makeSineFlac(path.join(album, "X.flac"), 10);
    makeCue(album, "X.flac", [
      [1, "one", "00:00:00"],
      [2, "two", ""],
    ]);
    // track 2 has no INDEX 01 line -> strip the empty line from the generated cue
    const cueText = fs.readFileSync(path.join(album, "album.cue"), "utf8");
    fs.writeFileSync(path.join(album, "album.cue"), cueText.replace(/INDEX 01 \n/, ""));

    const r = runTool(["cue-split", album]);
    expect(r.status).toBe(1);
    expect(r.stdout).toContain("missing INDEX 01");
  });
});

describe("toolbox.sh doctor", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "toolbox-doctor-"));
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  it("auto-renders an HTML report when the doctor saves one", () => {
    const scanDir = path.join(tmpDir, "Scan");
    fs.mkdirSync(scanDir, { recursive: true });
    makeSineFlac(path.join(scanDir, "X.flac"), 5);
    const report = path.join(tmpDir, "report.json");
    const r = runTool([
      "doctor",
      scanDir,
      "--doctor",
      "--report",
      report,
      "--checkpoint",
      path.join(tmpDir, "cp"),
    ]);
    expect(r.status).toBe(0);
    expect(fs.existsSync(report)).toBe(true);
    expect(fs.existsSync(path.join(tmpDir, "report.html"))).toBe(true);
    expect(fs.readFileSync(path.join(tmpDir, "report.html"), "utf8")).toContain("<!DOCTYPE html>");
  });

  it("does not render HTML when no report is saved", () => {
    const scanDir = path.join(tmpDir, "Scan2");
    fs.mkdirSync(scanDir, { recursive: true });
    makeSineFlac(path.join(scanDir, "X.flac"), 5);
    const r = runTool(["doctor", scanDir, "--doctor", "--checkpoint", path.join(tmpDir, "cp2")]);
    expect(r.status).toBe(0);
    expect(fs.existsSync(path.join(tmpDir, "Scan2.html"))).toBe(false);
  });
});

describe("toolbox.sh cue-split slicing", () => {
  let tmpDir: string;
  let album: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "toolbox-slice-"));
    album = path.join(tmpDir, "Album");
    fs.mkdirSync(album, { recursive: true });
    makeSineFlac(path.join(album, "X.flac"), 20);
    makeCue(album, "X.flac", [
      [1, "Alpha", "00:00:00"],
      [2, "Beta", "00:10:00"],
    ]);
    writeFile(path.join(album, "cover.jpg"), "jpeg-bytes");
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  it("slices a synthetic album into tracks with metadata and copied images", () => {
    const r = runTool(["cue-split", album]);
    expect(r.status).toBe(0);

    const outDir = path.join(tmpDir, "Album-tracks");
    expect(fs.existsSync(path.join(outDir, "01. Alpha.flac"))).toBe(true);
    expect(fs.existsSync(path.join(outDir, "02. Beta.flac"))).toBe(true);
    expect(fs.existsSync(path.join(outDir, "cover.jpg"))).toBe(true);

    const tags = execFileSync(
      "ffprobe",
      ["-v", "error", "-show_entries", "format_tags", "-of", "default=noprint_wrappers=1", path.join(outDir, "01. Alpha.flac")],
      { encoding: "utf8" },
    );
    expect(tags).toContain("TAG:title=Alpha");
    expect(tags).toContain("TAG:artist=Test Artist");
    expect(tags).toContain("TAG:track=1/2");
  });

  it("skips a complete output but re-slices a partial one", () => {
    expect(runTool(["cue-split", album]).status).toBe(0);

    const skip = runTool(["cue-split", album]);
    expect(skip.status).toBe(0);
    expect(skip.stdout).toContain("SKIP: output already has 2 FLAC(s)");

    const outDir = path.join(tmpDir, "Album-tracks");
    fs.rmSync(path.join(outDir, "02. Beta.flac"));
    const partial = runTool(["cue-split", album]);
    expect(partial.status).toBe(0);
    expect(partial.stdout).toContain("Incomplete output (1/2 FLACs)");
    expect(fs.existsSync(path.join(outDir, "02. Beta.flac"))).toBe(true);
  });
});
