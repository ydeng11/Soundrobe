// @vitest-environment node
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";

const repoRoot = path.resolve(__dirname, "../../..");
const scriptPath = path.join(repoRoot, "scripts/generate-corruption-report.js");

describe("generate-corruption-report script", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "generate-corruption-report-"));
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  it("renders bucket JSON without calling playable medium files failed", () => {
    const reportPath = path.join(tmpDir, "flac-metadata-report-2026-06-19T04-02-17-254Z.json");
    const htmlPath = path.join(tmpDir, "flac-corruption-report.html");
    fs.writeFileSync(
      reportPath,
      JSON.stringify({
        summary: { total: 16, clean: 15, minor: 0, medium: 1, broken: 0, fixed: 0, fixFailed: 0 },
        diagnosis: {
          "織田浩司": {
            "SUPER SOUND COLLECTION スタジオジブリ吹奏楽": {
              "織田浩司_シエナ・ウインド・オーケストラ_久石让-Cave of Mind(Trumpet Solo Feature).flac": ["medium"],
            },
          },
        },
      }),
      "utf8",
    );

    execFileSync("node", [scriptPath, reportPath, htmlPath], {
      encoding: "utf8",
    });

    const html = fs.readFileSync(htmlPath, "utf8");
    expect(html).toContain("Medium Files");
    expect(html).toContain("1 medium");
    expect(html).toContain("Cave of Mind(Trumpet Solo Feature).flac");
    expect(html).not.toContain("1 failed");
    expect(html).not.toContain("Strict Decode Failed");
    expect(html).not.toContain("&#10007; 織田浩司_シエナ");
  });

  it("renders medium files by issue type from JSON details", () => {
    const reportPath = path.join(tmpDir, "flac-metadata-report-2026-06-20T04-49-12-820Z.json");
    const htmlPath = path.join(tmpDir, "flac-corruption-report.html");
    fs.writeFileSync(
      reportPath,
      JSON.stringify({
        summary: { total: 3, clean: 0, minor: 0, medium: 3, broken: 0, fixed: 0, fixFailed: 0 },
        details: {
          "Artist/Album/missing-md5.flac": {
            artist: "Artist",
            album: "Album",
            track: "missing-md5.flac",
            relativePath: "Artist/Album/missing-md5.flac",
            bucket: "medium",
            issues: ["strict-decode-warning"],
            strict: {
              ok: true,
              message: "WARNING, cannot check MD5 signature since it was unset in the STREAMINFO | ok",
            },
          },
          "Artist/Album/playable-invalid.flac": {
            artist: "Artist",
            album: "Album",
            track: "playable-invalid.flac",
            relativePath: "Artist/Album/playable-invalid.flac",
            bucket: "medium",
            issues: ["strict-decode-invalid-playable"],
            strict: {
              ok: false,
              message: "FLAC__STREAM_DECODER_ERROR_STATUS_LOST_SYNC | ERROR during decoding",
            },
            playback: { ok: true, tool: "ffmpeg" },
          },
          "Artist/Album/md5-mismatch.flac": {
            artist: "Artist",
            album: "Album",
            track: "md5-mismatch.flac",
            relativePath: "Artist/Album/md5-mismatch.flac",
            bucket: "medium",
            issues: ["strict-decode-invalid-playable"],
            strict: {
              ok: false,
              message: "ERROR, MD5 signature mismatch",
            },
            playback: { ok: true, tool: "ffmpeg" },
          },
        },
      }),
      "utf8",
    );

    execFileSync("node", [scriptPath, reportPath, htmlPath], {
      encoding: "utf8",
    });

    const html = fs.readFileSync(htmlPath, "utf8");
    expect(html).toContain("Medium Issue Types");
    expect(html).toContain("Missing MD5");
    expect(html).toContain("Playable Strict Decode");
    expect(html).toContain("MD5 Mismatch");
    expect(html).toContain("missing-md5.flac");
    expect(html).toContain("playable-invalid.flac");
    expect(html).toContain("md5-mismatch.flac");
  });
});
