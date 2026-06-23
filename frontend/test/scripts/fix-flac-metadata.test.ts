// @vitest-environment node
import { execFileSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { flacHeaderWithDuration, paddingBlock, vorbisCommentBlock } from "../helpers/flac-helpers";

const repoRoot = path.resolve(__dirname, "../../..");
const scriptPath = path.join(repoRoot, "scripts/fix-flac-metadata.js");

function writeBrokenAudioFlac(filePath: string, title: string) {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  const comments = vorbisCommentBlock([`TITLE=${title}`], { isLast: true });
  fs.writeFileSync(
    filePath,
    Buffer.concat([
      flacHeaderWithDuration(false, 2, [comments]),
      Buffer.from([0xff, 0xf8, 0xc9, 0x18]),
      Buffer.alloc(256, 0xff),
    ]),
  );
}

function writeFakeFlacCommand(tmpDir: string, scriptBody: string) {
  const binDir = path.join(tmpDir, "bin");
  fs.mkdirSync(binDir, { recursive: true });
  const flacPath = path.join(binDir, "flac");
  fs.writeFileSync(flacPath, scriptBody, { mode: 0o755 });
  return `${binDir}${path.delimiter}${process.env.PATH ?? ""}`;
}

function writeFakeCommand(tmpDir: string, commandName: string, scriptBody: string) {
  const binDir = path.join(tmpDir, "bin");
  fs.mkdirSync(binDir, { recursive: true });
  const commandPath = path.join(binDir, commandName);
  fs.writeFileSync(commandPath, scriptBody, { mode: 0o755 });
  return `${binDir}${path.delimiter}${process.env.PATH ?? ""}`;
}

function writeFakeCommands(tmpDir: string, commands: Record<string, string>) {
  const binDir = path.join(tmpDir, "bin");
  fs.mkdirSync(binDir, { recursive: true });
  for (const [commandName, scriptBody] of Object.entries(commands)) {
    fs.writeFileSync(path.join(binDir, commandName), scriptBody, { mode: 0o755 });
  }
  return `${binDir}${path.delimiter}${process.env.PATH ?? ""}`;
}

function paddingBlockWithLastFlag(size: number) {
  const block = paddingBlock(size);
  block[0] |= 0x80;
  return block;
}

function writePrematureLastMetadataFlac(filePath: string) {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(
    filePath,
    Buffer.concat([
      flacHeaderWithDuration(false, 2, [
        vorbisCommentBlock(["TITLE=Premature Last"], { isLast: false }),
        paddingBlockWithLastFlag(8),
        paddingBlockWithLastFlag(8),
      ]),
      Buffer.from([0xff, 0xf8, 0xc9, 0x18]),
      Buffer.alloc(256, 0xff),
    ]),
  );
}

function writeMetadataGapFlac(filePath: string, gapBytes: number) {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(
    filePath,
    Buffer.concat([
      flacHeaderWithDuration(false, 2, [
        vorbisCommentBlock(["TITLE=Metadata Gap"], { isLast: false }),
        paddingBlockWithLastFlag(8),
      ]),
      Buffer.alloc(gapBytes),
      Buffer.from([0xff, 0xf8, 0xc9, 0x18]),
      Buffer.alloc(256, 0xff),
    ]),
  );
}

function fakeFlacByBasename(tmpDir: string, results: Record<string, string>) {
  const cases = Object.entries(results)
    .map(([basename, body]) => `${JSON.stringify(basename)})\n${body}\n;;`)
    .join("\n");
  return writeFakeCommand(tmpDir, "flac", `#!/bin/sh
base="$(basename "$4")"
case "$base" in
${cases}
esac
exit 0
`);
}

function fakeFfmpegDecodeFailure(tmpDir: string) {
  return writeFakeCommand(tmpDir, "ffmpeg", `#!/bin/sh
echo "decode failed" >&2
exit 1
`);
}

describe("fix-flac-metadata script", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "fix-flac-metadata-"));
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  it("classifies FLAC files whose tolerant decode fails as broken", () => {
    const filePath = path.join(tmpDir, "metadata-clean-audio-broken.flac");
    writeBrokenAudioFlac(filePath, "Broken Audio");
    const fakePath = fakeFfmpegDecodeFailure(tmpDir);

    const output = execFileSync("node", [scriptPath, tmpDir, "--doctor"], {
      encoding: "utf8",
      env: { ...process.env, PATH: fakePath },
    });

    expect(output).toContain("BROKEN_STRICT_DECODE_FAIL");
    expect(output).toContain("metadata-clean-audio-broken.flac");
    expect(output).toContain("Strict decode failed");
  });

  it("classifies warning-only strict decode output as medium", () => {
    const filePath = path.join(tmpDir, "Artist One", "Album A", "playable-warning.flac");
    writeBrokenAudioFlac(filePath, "Playable Warning");
    const fakePath = writeFakeFlacCommand(tmpDir, `#!/bin/sh
echo "$4: WARNING, cannot check MD5 signature since it was unset in the STREAMINFO" >&2
echo "$4: ok" >&2
exit 1
`);

    const output = execFileSync("node", [scriptPath, tmpDir, "--doctor"], {
      encoding: "utf8",
      env: { ...process.env, PATH: fakePath },
    });

    expect(output).toContain("MEDIUM_STRICT_DECODE");
    expect(output).toContain("playable-warning.flac");
    expect(output).toContain("Playable but strict decode reported a warning");
    expect(output).not.toContain("BROKEN_STRICT_DECODE_FAIL");
    expect(output).toContain("Medium (unknown repair):  1");
    expect(output).toContain("Artist One: 1/1 files with issues; strict decode warning: 1");
  });

  it("classifies playable strict FLAC failures as medium", () => {
    const filePath = path.join(tmpDir, "Artist One", "Album A", "ffmpeg-playable.flac");
    writeBrokenAudioFlac(filePath, "Playable Strict Failure");
    const fakePath = writeFakeCommands(tmpDir, {
      flac: `#!/bin/sh
echo "$4: *** Got error code 0:FLAC__STREAM_DECODER_ERROR_STATUS_LOST_SYNC" >&2
echo "$4: ERROR while decoding data" >&2
exit 1
`,
      ffmpeg: `#!/bin/sh
exit 0
`,
      ffprobe: `#!/bin/sh
echo '{"format":{"duration":"120.0"},"streams":[{"sample_rate":"44100","channels":2}]}'
exit 0
`,
    });

    const output = execFileSync("node", [scriptPath, tmpDir, "--doctor"], {
      encoding: "utf8",
      env: { ...process.env, PATH: fakePath },
    });

    expect(output).toContain("MEDIUM_STRICT_DECODE_PLAYABLE");
    expect(output).toContain("Playable by ffmpeg; strict FLAC decode reported an error");
    expect(output).not.toContain("BROKEN_STRICT_DECODE_FAIL");
    expect(output).toContain("Medium (unknown repair):  1");
    expect(output).toContain("Artist One: 1/1 files with issues; strict decode warning: 1");
  });

  it("classifies ffprobe audio probe errors as broken even when tolerant ffmpeg decode exits zero", () => {
    const filePath = path.join(tmpDir, "Artist One", "Album A", "ffprobe-error.flac");
    writeBrokenAudioFlac(filePath, "FFprobe Error");
    const fakePath = writeFakeCommands(tmpDir, {
      flac: `#!/bin/sh
echo "$4: *** Got error code 0:FLAC__STREAM_DECODER_ERROR_STATUS_LOST_SYNC" >&2
echo "$4: ERROR while decoding data" >&2
exit 1
`,
      ffmpeg: `#!/bin/sh
echo "Decoding error: Invalid data found when processing input" >&2
exit 0
`,
      ffprobe: `#!/bin/sh
echo "invalid sync code" >&2
echo "decode_frame() failed" >&2
exit 1
`,
    });

    const output = execFileSync("node", [scriptPath, tmpDir, "--doctor"], {
      encoding: "utf8",
      env: { ...process.env, PATH: fakePath },
    });

    expect(output).toContain("BROKEN_STRICT_DECODE_FAIL");
    expect(output).toContain("ffprobe probe: invalid sync code");
    expect(output).toContain("Broken (redownload):     1");
    expect(output).not.toContain("MEDIUM_STRICT_DECODE_PLAYABLE");
  });

  it("classifies premature last metadata block chains as minor repairable issues", () => {
    const filePath = path.join(tmpDir, "Artist One", "Album A", "premature-last.flac");
    const reportPath = path.join(tmpDir, "premature-last-report.json");
    writePrematureLastMetadataFlac(filePath);
    const fakePath = writeFakeCommands(tmpDir, {
      flac: `#!/bin/sh
echo "$4: *** Got error code 0:FLAC__STREAM_DECODER_ERROR_STATUS_LOST_SYNC after processing 0 samples" >&2
echo "$4: ERROR while decoding data" >&2
exit 1
`,
      ffmpeg: `#!/bin/sh
exit 0
`,
      ffprobe: `#!/bin/sh
echo '{"format":{"duration":"2.0"},"streams":[{"sample_rate":"44100","channels":2}]}'
exit 0
`,
    });

    const output = execFileSync("node", [scriptPath, tmpDir, "--doctor", "--report", reportPath], {
      encoding: "utf8",
      env: { ...process.env, PATH: fakePath },
    });

    expect(output).toContain("MINOR_BROKEN_CHAIN");
    expect(output).toContain("premature-last.flac");
    expect(output).toContain("Minor (known repair):    1");
    expect(output).not.toContain("MEDIUM_STRICT_DECODE_PLAYABLE");
    expect(output).not.toContain("strict decode invalid playable");
    expect(output).not.toContain("playable strict decode failure");

    const report = JSON.parse(fs.readFileSync(reportPath, "utf8"));
    const detail = report.details["Artist One/Album A/premature-last.flac"];
    expect(detail.bucket).toBe("minor");
    expect(detail.issues).toContain("broken-metadata-chain");
    expect(detail.issues).toContain("strict-decode-failed");
    expect(detail.issues).not.toContain("strict-decode-invalid-playable");
    expect(detail.playback.ok).toBe(false);
    expect(detail.playback.tool).toBe("compatibility");
  });

  it("reports a 64 KiB stale padding block before audio as a metadata gap", () => {
    const filePath = path.join(tmpDir, "Artist One", "Album A", "stale-padding-gap.flac");
    writeMetadataGapFlac(filePath, 65540);
    const fakePath = writeFakeFlacCommand(tmpDir, `#!/bin/sh
exit 0
`);

    const output = execFileSync("node", [scriptPath, tmpDir, "--doctor"], {
      encoding: "utf8",
      env: { ...process.env, PATH: fakePath },
    });

    expect(output).toContain("MINOR_METADATA_GAP");
    expect(output).toContain("65540 zero bytes between metadata and audio");
    expect(output).toContain("Minor (known repair):    1");
    expect(output).not.toContain("All files are clean");
  });

  it("classifies unplayable tracks as broken before lower-severity structural checks", () => {
    const filePath = path.join(tmpDir, "Artist One", "Album A", "chain-plus-probe-failure.flac");
    const reportPath = path.join(tmpDir, "chain-plus-probe-failure-report.json");
    writePrematureLastMetadataFlac(filePath);
    const fakePath = writeFakeCommands(tmpDir, {
      flac: `#!/bin/sh
echo "$4: *** Got error code 0:FLAC__STREAM_DECODER_ERROR_STATUS_LOST_SYNC after processing 0 samples" >&2
echo "$4: ERROR while decoding data" >&2
exit 1
`,
      ffmpeg: `#!/bin/sh
exit 0
`,
      ffprobe: `#!/bin/sh
echo "[mjpeg @ 0x123] No JPEG data found in image" >&2
echo '{"format":{"duration":"2.0"},"streams":[{"sample_rate":"44100","channels":2}]}'
exit 0
`,
    });

    const output = execFileSync("node", [scriptPath, tmpDir, "--doctor", "--report", reportPath], {
      encoding: "utf8",
      env: { ...process.env, PATH: fakePath },
    });

    expect(output).toContain("BROKEN_STRICT_DECODE_FAIL");
    expect(output).toContain("chain-plus-probe-failure.flac");
    expect(output).toContain("Broken (redownload):     1");
    expect(output).toContain("Minor (known repair):    0");

    const report = JSON.parse(fs.readFileSync(reportPath, "utf8"));
    const detail = report.details["Artist One/Album A/chain-plus-probe-failure.flac"];
    expect(detail.bucket).toBe("broken");
    expect(detail.action).toBe("redownload");
    expect(detail.issues).not.toContain("broken-metadata-chain");
    expect(detail.issues).toContain("ffprobe-audio-probe-failed");
    expect(detail.knownRepairs).toEqual([]);
    expect(detail.playback.ok).toBe(false);
  });

  it("does not count a repaired chain as fixFailed when only a strict warning remains", () => {
    const filePath = path.join(tmpDir, "Artist One", "Album A", "repaired-warning.flac");
    const reportPath = path.join(tmpDir, "repaired-warning-report.json");
    const flacCountPath = path.join(tmpDir, "flac-count");
    writePrematureLastMetadataFlac(filePath);
    const fakePath = writeFakeCommands(tmpDir, {
      flac: `#!/bin/sh
count_file=${JSON.stringify(flacCountPath)}
count=0
if [ -f "$count_file" ]; then count=$(cat "$count_file"); fi
count=$((count + 1))
printf "%s" "$count" > "$count_file"
if [ "$count" -eq 1 ]; then
  echo "$4: *** Got error code 0:FLAC__STREAM_DECODER_ERROR_STATUS_LOST_SYNC after processing 0 samples" >&2
  echo "$4: ERROR while decoding data" >&2
  exit 1
fi
echo "$4: WARNING, cannot check MD5 signature since it was unset in the STREAMINFO" >&2
echo "$4: ok" >&2
exit 1
`,
      ffmpeg: `#!/bin/sh
exit 0
`,
      ffprobe: `#!/bin/sh
echo '{"format":{"duration":"2.0"},"streams":[{"sample_rate":"44100","channels":2}]}'
exit 0
`,
    });

    const output = execFileSync("node", [scriptPath, tmpDir, "--report", reportPath], {
      encoding: "utf8",
      env: { ...process.env, PATH: fakePath },
    });

    expect(output).toContain("✓ fixed");
    expect(output).toContain("Fixed:                    1");
    expect(output).toContain("Fix failed:               0");

    const report = JSON.parse(fs.readFileSync(reportPath, "utf8"));
    expect(report.summary.fixed).toBe(1);
    expect(report.summary.fixFailed).toBe(0);
    const detail = report.details["Artist One/Album A/repaired-warning.flac"];
    expect(detail.bucket).toBe("medium");
    expect(detail.action).toBe("fixed");
    expect(detail.issues).toEqual(["strict-decode-warning"]);
  });

  it("does not re-encode playable strict FLAC failures in fix mode", () => {
    const filePath = path.join(tmpDir, "Artist One", "Album A", "reencode-me.flac");
    writeBrokenAudioFlac(filePath, "Reencode Me");
    const fakePath = writeFakeCommands(tmpDir, {
      flac: `#!/bin/sh
file=""
for arg in "$@"; do
  file="$arg"
done
if grep -q fixed-by-ffmpeg "$file"; then
  exit 0
fi
echo "$file: *** Got error code 0:FLAC__STREAM_DECODER_ERROR_STATUS_LOST_SYNC" >&2
echo "$file: ERROR while decoding data" >&2
exit 1
`,
      ffmpeg: `#!/bin/sh
input=""
prev=""
output=""
for arg in "$@"; do
  if [ "$prev" = "-i" ]; then
    input="$arg"
  fi
  output="$arg"
  prev="$arg"
done
if [ "$output" = "-" ]; then
  exit 0
fi
cp "$input" "$output"
printf 'fixed-by-ffmpeg' >> "$output"
exit 0
`,
      ffprobe: `#!/bin/sh
echo '{"format":{"duration":"120.0"},"streams":[{"sample_rate":"44100","channels":2}]}'
exit 0
`,
    });

    const output = execFileSync("node", [scriptPath, tmpDir], {
      encoding: "utf8",
      env: { ...process.env, PATH: fakePath },
    });

    expect(output).toContain("MEDIUM_STRICT_DECODE_PLAYABLE");
    expect(output).toContain("playable strict decode failure is medium; no safe repair is known");
    expect(output).toContain("Medium (unknown repair):  1");
    expect(output).not.toContain("fixed playable strict decode failure via ffmpeg re-encode");
    expect(output).not.toContain("Fixed:                    1");
  });

  it("summarizes issues by artist at the end of a library scan", () => {
    writeBrokenAudioFlac(path.join(tmpDir, "Artist One", "Album A", "one.flac"), "One");
    writeBrokenAudioFlac(path.join(tmpDir, "Artist One", "Album B", "two.flac"), "Two");
    writeBrokenAudioFlac(path.join(tmpDir, "Artist Two", "Album A", "three.flac"), "Three");
    const fakePath = fakeFfmpegDecodeFailure(tmpDir);

    const output = execFileSync("node", [scriptPath, tmpDir, "--doctor"], {
      encoding: "utf8",
      env: { ...process.env, PATH: fakePath },
    });

    const summary = output.slice(output.indexOf("Issues by artist:"));
    expect(summary).toContain("Artist One: 2/2 files with issues; strict decode failed: 2");
    expect(summary).toContain("Artist Two: 1/1 files with issues; strict decode failed: 1");
  });

  it("groups final report results into bucket sections", () => {
    writeBrokenAudioFlac(path.join(tmpDir, "Artist One", "Album A", "alert.flac"), "Alert");
    writeBrokenAudioFlac(path.join(tmpDir, "Artist One", "Album A", "warning.flac"), "Warning");
    const fakePath = fakeFlacByBasename(tmpDir, {
      "alert.flac": `echo "$4: *** Got error code 0:FLAC__STREAM_DECODER_ERROR_STATUS_LOST_SYNC" >&2
echo "$4: ERROR while decoding data" >&2
exit 1`,
      "warning.flac": `echo "$4: WARNING, cannot check MD5 signature since it was unset in the STREAMINFO" >&2
echo "$4: ok" >&2
exit 1`,
    });

    const output = execFileSync("node", [scriptPath, tmpDir, "--doctor"], {
      encoding: "utf8",
      env: { ...process.env, PATH: fakePath },
    });

    const bucketSummary = output.slice(output.indexOf("Results by bucket:"));
    expect(bucketSummary).toContain("BROKEN:");
    expect(bucketSummary).toContain("Artist One/Album A/alert.flac");
    expect(bucketSummary).toContain("MEDIUM:");
    expect(bucketSummary).toContain("Artist One/Album A/warning.flac");
    expect(bucketSummary.indexOf("MEDIUM:")).toBeLessThan(bucketSummary.indexOf("BROKEN:"));
  });

  it("uses the parent directory as artist when scanning one album directory", () => {
    const albumDir = path.join(tmpDir, "Artist One", "Album A");
    writeBrokenAudioFlac(path.join(albumDir, "one.flac"), "One");
    const fakePath = fakeFfmpegDecodeFailure(tmpDir);

    const output = execFileSync("node", [scriptPath, albumDir, "--doctor"], {
      encoding: "utf8",
      env: { ...process.env, PATH: fakePath },
    });

    const summary = output.slice(output.indexOf("Issues by artist:"));
    expect(summary).toContain("Artist One: 1/1 files with issues; strict decode failed: 1");
    expect(summary).not.toContain("Album A: 1/1 files with issues");
  });

  it("saves the terminal diagnosis to a report file when requested", () => {
    const reportPath = path.join(tmpDir, "reports", "flac-doctor.txt");
    writeBrokenAudioFlac(path.join(tmpDir, "Artist One", "Album A", "one.flac"), "One");
    const fakePath = fakeFfmpegDecodeFailure(tmpDir);

    const output = execFileSync("node", [scriptPath, tmpDir, "--doctor", "--report", reportPath], {
      encoding: "utf8",
      env: { ...process.env, PATH: fakePath },
    });

    expect(output).toContain(`Report saved: ${reportPath}`);
    const report = fs.readFileSync(reportPath, "utf8");
    expect(report).toContain(`Scanning: ${tmpDir}`);
    expect(report).toContain("BROKEN_STRICT_DECODE_FAIL");
    expect(report).toContain("Issues by artist:");
    expect(report).toContain("Results by bucket:");
    expect(report).toContain("Artist One: 1/1 files with issues; strict decode failed: 1");
  });

  it("writes a JSON diagnosis report grouped by artist, album, and track", () => {
    const reportPath = path.join(tmpDir, "reports", "flac-doctor.json");
    const filePath = path.join(tmpDir, "Artist One", "Album A", "ffmpeg-playable.flac");
    writeBrokenAudioFlac(filePath, "Playable Strict Failure");
    const fakePath = writeFakeCommands(tmpDir, {
      flac: `#!/bin/sh
echo "$4: *** Got error code 0:FLAC__STREAM_DECODER_ERROR_STATUS_LOST_SYNC" >&2
echo "$4: ERROR while decoding data" >&2
exit 1
`,
      ffmpeg: `#!/bin/sh
exit 0
`,
      ffprobe: `#!/bin/sh
echo '{"format":{"duration":"120.0"},"streams":[{"sample_rate":"44100","channels":2}]}'
exit 0
`,
    });

    execFileSync("node", [scriptPath, tmpDir, "--doctor", "--report", reportPath], {
      encoding: "utf8",
      env: { ...process.env, PATH: fakePath },
    });

    const report = JSON.parse(fs.readFileSync(reportPath, "utf8"));
    expect(report.summary.medium).toBe(1);
    expect(report.summary.broken).toBe(0);
    expect(report.diagnosis["Artist One"]["Album A"]["ffmpeg-playable.flac"]).toEqual(["medium"]);
    expect(report.details["Artist One/Album A/ffmpeg-playable.flac"].bucket).toBe("medium");
    expect(report.details["Artist One/Album A/ffmpeg-playable.flac"].action).toBe("none");
  });

  it("defaults --report without a path to a JSON diagnosis file", () => {
    const libraryDir = path.join(tmpDir, "library");
    writeBrokenAudioFlac(path.join(libraryDir, "Artist One", "Album A", "one.flac"), "One");
    const fakePath = fakeFfmpegDecodeFailure(tmpDir);

    const output = execFileSync("node", [scriptPath, libraryDir, "--doctor", "--report"], {
      cwd: tmpDir,
      encoding: "utf8",
      env: { ...process.env, PATH: fakePath },
    });

    const reportLine = output.split("\n").find((line) => line.startsWith("Report saved: "));
    expect(reportLine).toBeTruthy();
    const reportPath = reportLine!.replace("Report saved: ", "");
    expect(reportPath.endsWith(".json")).toBe(true);
    const report = JSON.parse(fs.readFileSync(reportPath, "utf8"));
    expect(report.summary.broken).toBe(1);
    expect(report.diagnosis["Artist One"]["Album A"]["one.flac"]).toEqual(["broken"]);
  });

  it("keeps doctor and dry-run JSON diagnosis buckets identical", () => {
    writeBrokenAudioFlac(path.join(tmpDir, "Artist One", "Album A", "one.flac"), "One");
    const fakePath = fakeFfmpegDecodeFailure(tmpDir);
    const doctorReport = path.join(tmpDir, "doctor.json");
    const dryRunReport = path.join(tmpDir, "dry-run.json");

    execFileSync("node", [scriptPath, tmpDir, "--doctor", "--report", doctorReport], {
      encoding: "utf8",
      env: { ...process.env, PATH: fakePath },
    });
    execFileSync("node", [scriptPath, tmpDir, "--dry-run", "--report", dryRunReport], {
      encoding: "utf8",
      env: { ...process.env, PATH: fakePath },
    });

    const doctorJson = JSON.parse(fs.readFileSync(doctorReport, "utf8"));
    const dryRunJson = JSON.parse(fs.readFileSync(dryRunReport, "utf8"));
    expect(dryRunJson.diagnosis).toEqual(doctorJson.diagnosis);
  });
});
