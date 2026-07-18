#!/usr/bin/env node
/**
 * fix-flac-metadata — Detect and fix issues in FLAC files:
 *
 * 1. Vorbis comment block length mismatches (soundrobe writer bug)
 * 2. Audio frame sync corruption (first byte bit 7 cleared: 0x7f → 0xff)
 * 3. Metadata-audio gap (extra zero bytes between metadata and audio)
 * 4. Corrupted METADATA_BLOCK_PICTURE tags (raw image data instead of
 *    proper Vorbis picture structure)
 *
 * Usage:
 *   node scripts/fix-flac-metadata.js <directory>              # fix mode
 *   node scripts/fix-flac-metadata.js <directory> --doctor     # diagnosis only
 *   node scripts/fix-flac-metadata.js <directory> --dry-run    # show what would change
 *
 * Options:
 *   --concurrency N    Max parallel workers (default: min(cpus-1, 4))
 *   --batch-size N     Files per batch (default: 200)
 *   --resume           Resume from latest checkpoint for this directory
 *   --checkpoint DIR   Explicit checkpoint directory
 *   --report [FILE]    Save final diagnosis to JSON by default; non-.json paths keep text output
 *   --verbose, -v      Show OK files too
 *
 * What it checks:
 *   - Vorbis comment block header length vs actual Vorbis comment content size
 *   - Trailing zero bytes inside the Vorbis comment block (the bug signature)
 *   - First byte of first FLAC audio frame: expects 0xff, finds 0x7f
 *   - Gap between last metadata block and first audio frame
 *   - METADATA_BLOCK_PICTURE tags with raw JPEG/PNG data instead of proper structure
 *
 * What it fixes:
 *   - Updates the Vorbis comment block header length to match actual content
 *   - Converts leftover bytes (≥4) into a valid PADDING metadata block
 *   - Falls back to ffmpeg re-mux when leftover is <4 bytes
 *   - Restores the first audio frame sync byte (bit 7)
 *
 * Checkpoint/Resume:
 *   Results are saved to reports/checkpoints/scan-<timestamp>/results/batch-NNNNNN.json
 *   Use --resume to skip already-processed files and continue from where you left off.
 *   Interrupted runs (Ctrl+C) save partial progress automatically.
 */

const fs = require("fs");
const path = require("path");
const { execFileSync, spawnSync } = require("child_process");

// ── CLI args ────────────────────────────────────────────────────────

const os = require("os");

const args = process.argv.slice(2);

function defaultReportPath() {
  const stamp = new Date().toISOString().replace(/[:.]/g, "-");
  return path.resolve(process.cwd(), "reports", `flac-metadata-report-${stamp}.json`);
}

function defaultCheckpointDir() {
  const stamp = new Date().toISOString().replace(/[:.]/g, "-");
  return path.resolve(process.cwd(), "reports", "checkpoints", `scan-${stamp}`);
}

function parseArgs(rawArgs) {
  const parsed = { flags: [], positional: [], reportPath: null, checkpointDir: null, concurrency: null, batchSize: null };

  for (let i = 0; i < rawArgs.length; i++) {
    const arg = rawArgs[i];
    if (arg === "--report") {
      parsed.flags.push(arg);
      const next = rawArgs[i + 1];
      if (next && !next.startsWith("--")) {
        parsed.reportPath = path.resolve(next);
        i++;
      } else {
        parsed.reportPath = defaultReportPath();
      }
    } else if (arg.startsWith("--report=")) {
      parsed.flags.push("--report");
      parsed.reportPath = path.resolve(arg.slice("--report=".length));
    } else if (arg === "--checkpoint") {
      parsed.flags.push(arg);
      const next = rawArgs[i + 1];
      if (next && !next.startsWith("--")) {
        parsed.checkpointDir = path.resolve(next);
        i++;
      } else {
        parsed.checkpointDir = defaultCheckpointDir();
      }
    } else if (arg.startsWith("--checkpoint=")) {
      parsed.flags.push("--checkpoint");
      parsed.checkpointDir = path.resolve(arg.slice("--checkpoint=".length));
    } else if (arg === "--concurrency") {
      const next = rawArgs[i + 1];
      if (next && !next.startsWith("--")) {
        parsed.concurrency = Math.max(1, parseInt(next, 10) || 1);
        i++;
      }
    } else if (arg === "--batch-size") {
      const next = rawArgs[i + 1];
      if (next && !next.startsWith("--")) {
        parsed.batchSize = Math.max(1, parseInt(next, 10) || 100);
        i++;
      }
    } else if (arg.startsWith("--") || arg === "-v") {
      parsed.flags.push(arg);
    } else {
      parsed.positional.push(arg);
    }
  }

  return parsed;
}

const { flags, positional, reportPath: REPORT_PATH, checkpointDir: CHECKPOINT_DIR, concurrency: CONCURRENCY, batchSize: BATCH_SIZE } = parseArgs(args);

const DOCTOR = flags.includes("--doctor");
const DRY_RUN = flags.includes("--dry-run"  );
const VERBOSE = flags.includes("--verbose") || flags.includes("-v");
const RESUME = flags.includes("--resume");
const WORKER_BATCH = flags.find(f => f.startsWith("--worker-batch="))?.slice("--worker-batch=".length)
  || (flags.includes("--worker-batch") ? positional[positional.indexOf("--worker-batch") + 1] : null);
const WORKER_MODE = !!WORKER_BATCH;
const DEFAULT_CONCURRENCY = Math.max(1, Math.min(4, os.cpus().length - 1));
const DEFAULT_BATCH_SIZE = 200;

if (!WORKER_MODE && positional.length === 0) {
  console.error("Usage: node fix-flac-metadata.js <directory> [--doctor] [--dry-run] [--verbose] [--report [file]] [--checkpoint [dir]] [--resume] [--concurrency N] [--batch-size N]");
  process.exit(1);
}

const TOP_DIR = WORKER_MODE ? null : path.resolve(positional[0]);

if (!WORKER_MODE && (!fs.existsSync(TOP_DIR) || !fs.statSync(TOP_DIR).isDirectory())) {
  console.error(`Error: "${TOP_DIR}" is not a directory`);
  process.exit(1);
}

// Report capture (parent mode only)
const reportLines = [];
if (!WORKER_MODE && REPORT_PATH) {
  const originalLog = console.log.bind(console);
  const originalError = console.error.bind(console);
  console.log = (...values) => {
    reportLines.push(values.map(String).join(" "));
    originalLog(...values);
  };
  console.error = (...values) => {
    reportLines.push(values.map(String).join(" "));
    originalError(...values);
  };
}

function isJsonReportPath(reportPath) {
  return /\.json$/i.test(reportPath || "");
}

function saveReportFile(finalReport) {
  if (!REPORT_PATH) return;
  fs.mkdirSync(path.dirname(REPORT_PATH), { recursive: true });
  if (isJsonReportPath(REPORT_PATH)) {
    fs.writeFileSync(REPORT_PATH, `${JSON.stringify(finalReport, null, 2)}\n`, "utf8");
  } else {
    fs.writeFileSync(REPORT_PATH, `${reportLines.join("\n")}\n`, "utf8");
  }
  console.log(`Report saved: ${REPORT_PATH}`);
}

// ── Checkpoint management ───────────────────────────────────────────

function loadCheckpoint(checkpointDir) {
  const processed = new Map(); // rel -> result
  if (!checkpointDir || !fs.existsSync(checkpointDir)) return processed;

  const manifestPath = path.join(checkpointDir, "manifest.json");
  const resultsDir = path.join(checkpointDir, "results");

  if (!fs.existsSync(resultsDir)) return processed;

  const files = fs.readdirSync(resultsDir).filter(f => f.endsWith(".json")).sort();
  for (const file of files) {
    try {
      const batch = JSON.parse(fs.readFileSync(path.join(resultsDir, file), "utf8"));
      if (batch.results && Array.isArray(batch.results)) {
        for (const result of batch.results) {
          processed.set(result.rel, result);
        }
      }
    } catch (err) {
      console.error(`Warning: failed to load checkpoint ${file}: ${err.message}`);
    }
  }

  return processed;
}

function writeBatchResult(checkpointDir, batchId, results) {
  if (!checkpointDir) return;
  const resultsDir = path.join(checkpointDir, "results");
  fs.mkdirSync(resultsDir, { recursive: true });
  const filePath = path.join(resultsDir, `batch-${String(batchId).padStart(6, "0")}.json`);
  const tmpPath = filePath + ".tmp";
  const data = { batchId, completedAt: new Date().toISOString(), count: results.length, results };
  fs.writeFileSync(tmpPath, JSON.stringify(data, null, 2), "utf8");
  fs.renameSync(tmpPath, filePath);
}

function writeManifest(checkpointDir, manifest) {
  if (!checkpointDir) return;
  fs.mkdirSync(checkpointDir, { recursive: true });
  const filePath = path.join(checkpointDir, "manifest.json");
  const tmpPath = filePath + ".tmp";
  fs.writeFileSync(tmpPath, JSON.stringify(manifest, null, 2), "utf8");
  fs.renameSync(tmpPath, filePath);
}

// ── FLAC parsing ────────────────────────────────────────────────────

/**
 * Parse the FLAC metadata block layout from a buffer.
 * Returns { blocks, audioOffset }.
 */
function parseFlacLayout(buf) {
  const blocks = [];
  
  // Find fLaC marker (some files have ID3v2 tag prepended)
  let flacOffset = -1;
  for (let i = 0; i <= Math.min(buf.length - 4, 25000); i++) {
    if (buf[i] === 0x66 && buf[i + 1] === 0x4C && buf[i + 2] === 0x61 && buf[i + 3] === 0x43) {
      flacOffset = i;
      break;
    }
  }
  
  if (flacOffset < 0) {
    // No fLaC marker found, try offset 4 as fallback
    flacOffset = 0;
  }
  
  let offset = flacOffset + 4; // skip "fLaC"

  while (offset + 4 <= buf.length) {
    const byte0 = buf[offset];
    const isLast = !!(byte0 >> 7);
    const type = byte0 & 0x7f;
    const length = (buf[offset + 1] << 16) | (buf[offset + 2] << 8) | buf[offset + 3];
    const headerOffset = offset;
    const dataOffset = offset + 4;

    // Sanity: stop if block extends past EOF or looks invalid
    if (type > 126 || length > 20_000_000 || dataOffset + length > buf.length) break;

    blocks.push({ type, headerOffset, dataOffset, length, isLast });
    if (isLast) break;
    offset = dataOffset + length;
  }

  const last = blocks[blocks.length - 1];
  const audioOffset = last ? last.dataOffset + last.length : 4;
  return { blocks, audioOffset };
}

function hasPrematureLastMetadataBlock(buf) {
  let flacOffset = -1;
  for (let i = 0; i <= Math.min(buf.length - 4, 25000); i++) {
    if (buf[i] === 0x66 && buf[i + 1] === 0x4C && buf[i + 2] === 0x61 && buf[i + 3] === 0x43) {
      flacOffset = i;
      break;
    }
  }
  if (flacOffset < 0) return false;

  let offset = flacOffset + 4;
  let sawLastBeforeEnd = false;

  while (offset + 4 <= buf.length) {
    const byte0 = buf[offset];
    const isLast = !!(byte0 >> 7);
    const type = byte0 & 0x7f;
    const length = (buf[offset + 1] << 16) | (buf[offset + 2] << 8) | buf[offset + 3];
    const dataOffset = offset + 4;

    if (type > 6 || length > 20_000_000 || dataOffset + length > buf.length) break;
    if (sawLastBeforeEnd) return true;
    if (isLast) sawLastBeforeEnd = true;
    offset = dataOffset + length;
  }

  return false;
}

/**
 * Compute the actual Vorbis comment content size from the raw block data.
 * Returns the expected block length (header fields + vendor + comments).
 * Returns -1 if the data is too corrupted to parse.
 */
function computeVorbisContentSize(buf, dataOffset, blockLength) {
  const end = dataOffset + blockLength;
  if (end > buf.length) return -1;

  let pos = dataOffset;

  // vendor string length (4 bytes LE)
  if (pos + 4 > end) return -1;
  const vendorLen = buf.readUInt32LE(pos);
  pos += 4;

  // vendor string
  pos += vendorLen;
  if (pos > end) return -1;

  // number of comments (4 bytes LE)
  if (pos + 4 > end) return -1;
  const numComments = buf.readUInt32LE(pos);
  pos += 4;

  // each comment: length (4 bytes LE) + data
  for (let i = 0; i < numComments; i++) {
    if (pos + 4 > end) return -1;
    const cLen = buf.readUInt32LE(pos);
    pos += 4;
    pos += cLen;
    if (pos > end) return -1;
  }

  // Total = 4 (vendorLen) + vendorLen + 4 (numComments) + sum(4 + cLen)
  return pos - dataOffset;
}

/**
 * Analyse a single FLAC file.
 * Returns { file, headerLen, actualLen, mismatch, leftover, hasValidVorbis }.
 */
function analyzeFlac(filePath) {
  const buf = fs.readFileSync(filePath);
  
  // Find fLaC marker (some files have ID3v2 tag prepended)
  let hasFlacMarker = false;
  for (let i = 0; i <= Math.min(buf.length - 4, 25000); i++) {
    if (buf[i] === 0x66 && buf[i + 1] === 0x4C && buf[i + 2] === 0x61 && buf[i + 3] === 0x43) {
      hasFlacMarker = true;
      break;
    }
  }
  
  if (buf.length < 8 || !hasFlacMarker) {
    return { file: filePath, error: "not a valid FLAC file" };
  }

  const layout = parseFlacLayout(buf);
  const strictDecode = runStrictFlacDecode(filePath, layout);
  const needsPlaybackProbe = strictDecode?.ok === false;
  const playbackDecode = needsPlaybackProbe ? runFfmpegAudioDecode(filePath) : null;
  const audioProbe = needsPlaybackProbe ? runFfprobeAudioProbe(filePath) : null;

  if (strictDecode?.ok === false && (playbackDecode?.ok === false || audioProbe?.ok === false)) {
    return {
      file: filePath,
      headerLen: 0,
      actualLen: 0,
      mismatch: 0,
      leftover: 0,
      isLast: false,
      hasTrailingZeros: false,
      audioOffset: layout.audioOffset,
      audioSyncCorrupted: false,
      metadataAudioGap: 0,
      corruptedPictureTag: false,
      brokenChain: false,
      strictDecode,
      playbackDecode,
      audioProbe,
      skippedLowerSeverityChecks: true,
    };
  }

  // Check for broken metadata chain: STREAMINFO has isLast=true but
  // there are more metadata blocks after it. This happens when a Vorbis
  // block with large embedded art (>5MB) causes fixLastFlacBlock to skip
  // it, leaving STREAMINFO marked as last.
  const vorbisBlock = layout.blocks.find((b) => b.type === 4);
  let brokenChain = hasPrematureLastMetadataBlock(buf);
  if (!brokenChain && layout.blocks.length > 0 && layout.blocks[0].isLast) {
    const streamInfoEnd = layout.blocks[0].dataOffset + layout.blocks[0].length;
    if (streamInfoEnd + 4 <= buf.length) {
      const nextType = buf[streamInfoEnd] & 0x7f;
      if (nextType <= 6) {
        brokenChain = true;
      }
    }
  }

  if (!vorbisBlock && !brokenChain) {
    return { file: filePath, error: "no VORBIS_COMMENT block found" };
  }

  let headerLen = 0;
  let actualLen = 0;
  let mismatch = 0;
  let leftover = 0;
  let hasTrailingZeros = false;
  let corruptedPictureTag = false;

  if (vorbisBlock) {
    headerLen = vorbisBlock.length;
    actualLen = computeVorbisContentSize(buf, vorbisBlock.dataOffset, headerLen);

    if (actualLen < 0) {
      return { file: filePath, error: "Vorbis comment block too corrupted to parse", brokenChain };
    }

    mismatch = headerLen - actualLen;
    leftover = mismatch;
    hasTrailingZeros = mismatch > 0;

    corruptedPictureTag = hasCorruptedMetadataBlockPicture(
      buf, vorbisBlock.dataOffset, vorbisBlock.length
    );
  }

  // Check for audio frame sync corruption (first byte bit 7 cleared)
  const audioOffset = layout.audioOffset;
  let audioSyncCorrupted = false;
  if (audioOffset < buf.length) {
    const b0 = buf[audioOffset];
    const b1 = buf[audioOffset + 1] ?? 0;
    audioSyncCorrupted = (b0 === 0x7f && (b1 & 0xf8) === 0xf8);
  }

  // Check for gap between last metadata block and first audio frame
  let metadataAudioGap = 0;
  if (layout.blocks.length > 0) {
    const lastBlock = layout.blocks[layout.blocks.length - 1];
    const expectedAudio = lastBlock.dataOffset + lastBlock.length;
    for (let i = expectedAudio; i < Math.min(expectedAudio + 200000, buf.length - 1); i++) {
      if (buf[i] === 0xff && (buf[i + 1] & 0xf8) === 0xf8) {
        metadataAudioGap = i - expectedAudio;
        break;
      }
    }
  }

  return {
    file: filePath,
    headerLen,
    actualLen,
    mismatch,
    leftover,
    isLast: vorbisBlock ? vorbisBlock.isLast : false,
    hasTrailingZeros,
    audioOffset,
    audioSyncCorrupted,
    metadataAudioGap,
    corruptedPictureTag,
    brokenChain,
    strictDecode,
    playbackDecode,
    audioProbe,
    skippedLowerSeverityChecks: false,
  };
}

function formatDecodeMessage(text) {
  const details = [];
  const seen = new Set();
  const lines = text
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean)
    .filter((line) => !/^flac \d/.test(line))
    .filter((line) => !/^Copyright /.test(line))
    .filter((line) => !/^flac comes with /.test(line))
    .filter((line) => !/^Type `flac' /.test(line))
    .filter((line) => !/^welcome to redistribute /.test(line));

  for (const line of lines) {
    const statusMatch = line.match(/FLAC__STREAM_DECODER_ERROR_STATUS_[A-Z_]+/);
    const key = statusMatch ? statusMatch[0] : line.replace(/^.*?:\s*/, "");
    if (seen.has(key)) continue;
    seen.add(key);
    details.push(key);
    if (details.length >= 6) break;
  }

  return details.join(" | ");
}

function runStrictFlacDecode(filePath, layout) {
  try {
    execFileSync("flac", ["-t", "-w", "--", filePath], {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    });
    return { ok: true };
  } catch (err) {
    if (err && err.code === "ENOENT") {
      return { ok: null, unavailable: true, message: "flac command not found" };
    }

    const stderr = Buffer.isBuffer(err.stderr)
      ? err.stderr.toString("utf8")
      : String(err.stderr ?? "");
    const stdout = Buffer.isBuffer(err.stdout)
      ? err.stdout.toString("utf8")
      : String(err.stdout ?? "");
    const raw = `${stderr}\n${stdout}`;
    const samples = extractLastProcessedSamples(raw);
    const streamInfo = extractStreamInfo(filePath, layout);
    const warningOnly = isStrictDecodeWarningOnly(raw);
    return {
      ok: warningOnly,
      warning: warningOnly,
      decodedSamples: samples,
      decodedSeconds: samples != null && streamInfo.sampleRate ? samples / streamInfo.sampleRate : null,
      decodedPercent:
        samples != null && streamInfo.totalSamples
          ? (samples / streamInfo.totalSamples) * 100
          : null,
      message: formatDecodeMessage(raw) || (err.message ?? "strict FLAC decode failed"),
    };
  }
}

function runFfmpegAudioDecode(filePath) {
  try {
    execFileSync("ffmpeg", [
      "-hide_banner",
      "-nostdin",
      "-v", "error",
      "-i", filePath,
      "-map", "0:a:0",
      "-f", "null",
      "-",
    ], {
      encoding: "utf8",
      stdio: ["ignore", "pipe", "pipe"],
    });
    return { ok: true };
  } catch (err) {
    if (err && err.code === "ENOENT") {
      return { ok: null, unavailable: true, message: "ffmpeg command not found" };
    }

    const stderr = Buffer.isBuffer(err.stderr)
      ? err.stderr.toString("utf8")
      : String(err.stderr ?? "");
    const stdout = Buffer.isBuffer(err.stdout)
      ? err.stdout.toString("utf8")
      : String(err.stdout ?? "");
    return {
      ok: false,
      message: formatFfmpegDecodeMessage(`${stderr}\n${stdout}`) || (err.message ?? "ffmpeg audio decode failed"),
    };
  }
}

function runFfprobeAudioProbe(filePath) {
  const result = spawnSync("ffprobe", [
    "-v", "error",
    "-show_entries", "format=duration:stream=duration,sample_rate,channels",
    "-of", "json",
    filePath,
  ], {
    encoding: "utf8",
    stdio: ["ignore", "pipe", "pipe"],
  });

  if (result.error?.code === "ENOENT") {
    return { ok: null, unavailable: true, message: "ffprobe command not found" };
  }

  const stderr = String(result.stderr ?? "");
  const stdout = String(result.stdout ?? "");
  const stderrMessage = formatFfmpegDecodeMessage(stderr);
  if (result.status !== 0 || stderrMessage) {
    return {
      ok: false,
      message: stderrMessage || formatFfmpegDecodeMessage(stdout) || (result.error?.message ?? "ffprobe audio probe failed"),
    };
  }

  return { ok: true };
}

function formatFfmpegDecodeMessage(text) {
  return text
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean)
    .slice(0, 4)
    .join(" | ");
}

function isStrictDecodeWarningOnly(text) {
  return (
    /WARNING/i.test(text) &&
    /(^|\s)ok\s*$/im.test(text) &&
    !/FLAC__STREAM_DECODER_ERROR_STATUS_/i.test(text) &&
    !/ERROR (while|during) decoding/i.test(text) &&
    !/FLAC__STREAM_DECODER_ABORTED/i.test(text)
  );
}

function formatDecodePosition(strictDecode) {
  if (strictDecode.decodedSamples == null) return "";
  if (strictDecode.decodedSeconds == null) {
    return ` after ${strictDecode.decodedSamples} samples`;
  }
  const percent =
    strictDecode.decodedPercent == null
      ? ""
      : `, ${strictDecode.decodedPercent.toFixed(1)}% of declared samples`;
  return ` after ${strictDecode.decodedSamples} samples (${strictDecode.decodedSeconds.toFixed(2)}s${percent})`;
}

function extractLastProcessedSamples(text) {
  let last = null;
  const re = /after processing\s+(\d+)\s+samples/g;
  let match;
  while ((match = re.exec(text)) !== null) {
    last = Number(match[1]);
  }
  return Number.isFinite(last) ? last : null;
}

function extractStreamInfo(filePath, layout) {
  const streamInfo = layout.blocks.find((b) => b.type === 0);
  if (!streamInfo || streamInfo.length < 18) return { sampleRate: null, totalSamples: null };
  const buf = fs.readFileSync(filePath);
  if (streamInfo.dataOffset + 18 > buf.length) return { sampleRate: null, totalSamples: null };
  const b10 = buf[streamInfo.dataOffset + 10];
  const b11 = buf[streamInfo.dataOffset + 11];
  const b12 = buf[streamInfo.dataOffset + 12];
  const sampleRate = (b10 << 12) | (b11 << 4) | (b12 >> 4);
  const totalSamples =
    ((BigInt(buf[streamInfo.dataOffset + 13] & 0x0f) << 32n) |
      (BigInt(buf[streamInfo.dataOffset + 14]) << 24n) |
      (BigInt(buf[streamInfo.dataOffset + 15]) << 16n) |
      (BigInt(buf[streamInfo.dataOffset + 16]) << 8n) |
      BigInt(buf[streamInfo.dataOffset + 17]));
  return {
    sampleRate,
    totalSamples: totalSamples > 0n ? Number(totalSamples) : null,
  };
}

// ── Detection helpers ───────────────────────────────────────────────

/**
 * Check if a Vorbis comment contains a corrupted METADATA_BLOCK_PICTURE tag.
 * The soundrobe stored raw JPEG/PNG data instead of the proper structure:
 *   Picture type (4) + MIME len (4) + MIME + Desc len (4) + Desc + ... + Data
 *
 * A corrupted tag starts with 0xff 0xd8 (JPEG) or 0x89 0x50 (PNG).
 * A valid tag starts with picture type (0-21), so first byte should be 0x00.
 */
function hasCorruptedMetadataBlockPicture(buf, vorbisDataOffset, vorbisLength) {
  const end = vorbisDataOffset + vorbisLength;
  let pos = vorbisDataOffset;

  // vendor string
  if (pos + 4 > end) return false;
  const vendorLen = buf.readUInt32LE(pos);
  pos += 4 + vendorLen;

  // number of comments
  if (pos + 4 > end) return false;
  const numComments = buf.readUInt32LE(pos);
  pos += 4;

  for (let i = 0; i < numComments; i++) {
    if (pos + 4 > end) return false;
    const cLen = buf.readUInt32LE(pos);
    const commentStart = pos + 4;
    const commentEnd = commentStart + cLen;

    // Check if this is METADATA_BLOCK_PICTURE
    const keyLen = (() => {
      for (let j = commentStart; j < commentEnd && j < commentStart + 100; j++) {
        if (buf[j] === 0x3d) return j - commentStart; // '='
      }
      return -1;
    })();

    if (keyLen > 0) {
      const key = buf.toString('ascii', commentStart, commentStart + keyLen);
      if (key === 'METADATA_BLOCK_PICTURE') {
        const valueStart = commentStart + keyLen + 1;
        const valueLen = commentEnd - valueStart;
        if (valueLen > 4) {
          // Check first bytes of base64-decoded value
          // Valid: picture type (0x00 0x00 0x00 0x00-0x15)
          // Corrupted: JPEG header (0xff 0xd8) or PNG header (0x89 0x50)
          const firstByte = buf[valueStart];
          const secondByte = buf[valueStart + 1];
          // Base64 for 0xff is '/9j' or similar — check for JPEG/PNG base64 patterns
          if (firstByte === 0x2f && secondByte === 0x39) { // '/9' = base64 for 0xff 0xd8
            return true;
          }
        }
      }
    }

    pos = commentEnd;
  }
  return false;
}

// ── Fix logic ───────────────────────────────────────────────────────

function writeFlacBuffer(filePath, buf) {
  const dir = path.dirname(filePath);
  const base = path.basename(filePath);
  const tmpPath = path.join(dir, `.${base}.fix-${process.pid}-${Date.now()}.tmp`);
  const mode = fs.statSync(filePath).mode;

  try {
    fs.writeFileSync(tmpPath, buf);
    fs.chmodSync(tmpPath, mode);
    fs.renameSync(tmpPath, filePath);
  } catch (err) {
    try { if (fs.existsSync(tmpPath)) fs.unlinkSync(tmpPath); } catch (_) {}
    throw err;
  }
}

/**
 * Fix audio frame sync corruption: restore bit 7 of the first audio byte.
 * The soundrobe writer cleared it (0x7f → 0xff).
 *
 * Returns true if the file was modified, false otherwise.
 */
function fixAudioSync(filePath) {
  const buf = Buffer.from(fs.readFileSync(filePath));
  const layout = parseFlacLayout(buf);
  const audioOffset = layout.audioOffset;

  if (audioOffset >= buf.length) return false;

  const b0 = buf[audioOffset];
  const b1 = buf[audioOffset + 1] ?? 0;

  // Only fix if first byte is 0x7f with valid second byte pattern
  if (b0 === 0x7f && (b1 & 0xf8) === 0xf8) {
    buf[audioOffset] = 0xff; // restore bit 7
    writeFlacBuffer(filePath, buf);
    return true;
  }
  return false;
}

/**
 * Fix corrupted METADATA_BLOCK_PICTURE tag by removing it from the
 * Vorbis comment block. The file already has a proper PICTURE metadata
 * block, so this corrupted tag is redundant.
 *
 * Returns true if the file was modified, false otherwise.
 */
/**
 * Fix a broken FLAC metadata chain: STREAMINFO has isLast=1 but there
 * are more metadata blocks after it.
 *
 * Clears isLast on STREAMINFO, scans all metadata blocks, and sets
 * isLast on the last one found.
 */
function fixBrokenMetadataChain(filePath) {
  const buf = fs.readFileSync(filePath);

  // Find fLaC marker
  let flacOffset = -1;
  for (let i = 0; i <= Math.min(buf.length - 4, 25000); i++) {
    if (buf[i] === 0x66 && buf[i + 1] === 0x4C && buf[i + 2] === 0x61 && buf[i + 3] === 0x43) {
      flacOffset = i;
      break;
    }
  }
  if (flacOffset < 0) return false;

  // Clear isLast on STREAMINFO
  buf[flacOffset + 4] &= 0x7f;

  // Scan through ALL metadata blocks (ignoring isLast) to find the last one
  let offset = flacOffset + 4;
  let lastMetadataOffset = -1;

  while (offset + 4 <= buf.length) {
    const byte0 = buf[offset];
    const type = byte0 & 0x7f;
    const length = (buf[offset + 1] << 16) | (buf[offset + 2] << 8) | buf[offset + 3];
    const dataOffset = offset + 4;

    if (type > 6 || length > 20_000_000 || dataOffset + length > buf.length) break;

    // Clear isLast on this block
    buf[offset] = type & 0x7f;
    lastMetadataOffset = offset;
    offset = dataOffset + length;
  }

  if (lastMetadataOffset >= 0) {
    buf[lastMetadataOffset] |= 0x80; // set isLast on last block
  }

  writeFlacBuffer(filePath, buf);
  return true;
}

function fixCorruptedPictureTag(filePath) {
  const buf = Buffer.from(fs.readFileSync(filePath));
  const layout = parseFlacLayout(buf);
  const vorbisBlock = layout.blocks.find((b) => b.type === 4);
  if (!vorbisBlock) return false;

  // Verify the tag exists
  if (!hasCorruptedMetadataBlockPicture(buf, vorbisBlock.dataOffset, vorbisBlock.length)) {
    return false;
  }

  return removeVorbisTagManually(buf, layout, vorbisBlock, 'METADATA_BLOCK_PICTURE');
}

/**
 * Manually remove a Vorbis comment tag by rebuilding the block.
 */
function removeVorbisTagManually(buf, layout, vorbisBlock, tagKey) {
  const dataEnd = vorbisBlock.dataOffset + vorbisBlock.length;
  let pos = vorbisBlock.dataOffset;

  // vendor string
  const vendorLen = buf.readUInt32LE(pos);
  const vendor = buf.subarray(pos, pos + 4 + vendorLen);
  pos += 4 + vendorLen;

  // number of comments
  const numComments = buf.readUInt32LE(pos);
  pos += 4;

  // Collect all comments except the target
  const keptComments = [];
  let removed = 0;
  for (let i = 0; i < numComments; i++) {
    const cLen = buf.readUInt32LE(pos);
    const comment = buf.subarray(pos, pos + 4 + cLen);
    const keyEnd = pos + 4;
    // Find '=' to get key length
    let eqIdx = -1;
    for (let j = keyEnd; j < pos + 4 + cLen; j++) {
      if (buf[j] === 0x3d) { eqIdx = j - keyEnd; break; }
    }
    if (eqIdx > 0) {
      const key = buf.toString('ascii', keyEnd, keyEnd + eqIdx);
      if (key === tagKey) {
        removed++;
      } else {
        keptComments.push(comment);
      }
    } else {
      keptComments.push(comment);
    }
    pos += 4 + cLen;
  }

  if (removed === 0) return false;

  // Rebuild Vorbis comment block
  const newNumComments = Buffer.alloc(4);
  newNumComments.writeUInt32LE(keptComments.length);
  const newBlockData = Buffer.concat([vendor, newNumComments, ...keptComments]);

  // Rebuild the file
  const before = buf.subarray(0, vorbisBlock.headerOffset);
  const after = buf.subarray(dataEnd);
  const newHeader = Buffer.alloc(4);
  newHeader[0] = 0x04; // type=4, isLast=false
  newHeader[1] = (newBlockData.length >> 16) & 0xff;
  newHeader[2] = (newBlockData.length >> 8) & 0xff;
  newHeader[3] = newBlockData.length & 0xff;

  const result = Buffer.concat([before, newHeader, newBlockData, after]);
  result[4] &= 0x7f; // clear isLast on STREAMINFO
  fixLastFlacBlock(result);
  writeFlacBuffer(filePath, result);
  return true;
}

/**
 * Fix the gap between the last metadata block and the first audio frame.
 * Some files have extra zero bytes here that confuse strict decoders.
 * The fix absorbs the gap into the last PADDING block's length.
 *
 * Returns true if the file was modified, false otherwise.
 */
function fixMetadataAudioGap(filePath) {
  const buf = Buffer.from(fs.readFileSync(filePath));
  const layout = parseFlacLayout(buf);

  if (layout.blocks.length === 0) return false;

  const lastBlock = layout.blocks[layout.blocks.length - 1];
  const expectedAudioOffset = lastBlock.dataOffset + lastBlock.length;

  // Find actual audio offset by scanning for FLAC frame sync.
  // Some files have hidden PADDING blocks (64 KiB) between metadata and audio.
  let actualAudioOffset = -1;
  for (let i = expectedAudioOffset; i < Math.min(expectedAudioOffset + 200000, buf.length - 1); i++) {
    if (buf[i] === 0xff && (buf[i + 1] & 0xf8) === 0xf8) {
      actualAudioOffset = i;
      break;
    }
  }

  if (actualAudioOffset < 0 || actualAudioOffset === expectedAudioOffset) return false;

  const gap = actualAudioOffset - expectedAudioOffset;

  // Only fix if the last block is PADDING
  if (lastBlock.type !== 1) return false;

  // Absorb the entire gap into the PADDING block's length.
  // The gap may contain:
  // - All zeros (simple padding)
  // - A hidden PADDING block header + zeros (soundrobe wrote extra 64 KiB)
  // Either way, extending the PADDING length is safe.
  const newPadLen = lastBlock.length + gap;
  buf[lastBlock.headerOffset + 1] = (newPadLen >> 16) & 0xff;
  buf[lastBlock.headerOffset + 2] = (newPadLen >> 8) & 0xff;
  buf[lastBlock.headerOffset + 3] = newPadLen & 0xff;

  writeFlacBuffer(filePath, buf);
  return true;
}

/**
 * Fix a FLAC file by directly patching the Vorbis comment block header
 * and converting leftover bytes to a PADDING block.
 *
 * ONLY safe when the Vorbis comment block is the last metadata block
 * (isLast=true). Otherwise the leftover PADDING would orphan any blocks
 * that follow (e.g. PICTURE).
 *
 * Returns true on success, false on failure.
 */
function fixFlacDirect(filePath) {
  const buf = Buffer.from(fs.readFileSync(filePath));
  const layout = parseFlacLayout(buf);
  const vorbisBlock = layout.blocks.find((b) => b.type === 4);

  if (!vorbisBlock) return false;

  const actualLen = computeVorbisContentSize(buf, vorbisBlock.dataOffset, vorbisBlock.length);
  if (actualLen < 0) return false;

  const leftover = vorbisBlock.length - actualLen;
  if (leftover === 0) return true; // already correct

  if (leftover >= 4) {
    // Patch the Vorbis header length
    buf[vorbisBlock.headerOffset + 1] = (actualLen >> 16) & 0xff;
    buf[vorbisBlock.headerOffset + 2] = (actualLen >> 8) & 0xff;
    buf[vorbisBlock.headerOffset + 3] = actualLen & 0xff;

    // Clear isLast on Vorbis block (PADDING will follow)
    buf[vorbisBlock.headerOffset] = 0x04; // type=4, isLast=false

    // Write PADDING block header in the leftover space.
    // isLast follows the original Vorbis isLast: if Vorbis was last,
    // PADDING becomes last; otherwise PADDING is not last so the
    // decoder continues to the next block (e.g. PICTURE).
    const padOffset = vorbisBlock.dataOffset + actualLen;
    const padDataLen = leftover - 4;
    const padIsLast = vorbisBlock.isLast;
    buf[padOffset] = (padIsLast ? 0x81 : 0x01); // isLast flag | type=1 (PADDING)
    buf[padOffset + 1] = (padDataLen >> 16) & 0xff;
    buf[padOffset + 2] = (padDataLen >> 8) & 0xff;
    buf[padOffset + 3] = padDataLen & 0xff;
    // Body is already zeros (from the original file)

    writeFlacBuffer(filePath, buf);
    return true;
  }

  // leftover < 4: can't fit a PADDING header — fall back to ffmpeg
  return fixFlacFfmpeg(filePath);
}

/**
 * Fix a FLAC file by re-muxing through ffmpeg (rebuilds all metadata).
 * Slower but handles edge cases the direct patch can't.
 */
function fixFlacFfmpeg(filePath) {
  const tmpPath = filePath + ".fix-tmp.flac";
  try {
    execFileSync("ffmpeg", [
      "-v", "error",
      "-i", filePath,
      "-c:a", "copy",
      "-map_metadata", "0",
      "-y",
      tmpPath,
    ], { stdio: "pipe" });

    // Verify the output is valid
    const outBuf = fs.readFileSync(tmpPath);
    if (outBuf.length < 8 || outBuf.toString("ascii", 0, 4) !== "fLaC") {
      fs.unlinkSync(tmpPath);
      return false;
    }

    // Replace original with fixed
    fs.renameSync(tmpPath, filePath);
    return true;
  } catch {
    if (fs.existsSync(tmpPath)) fs.unlinkSync(tmpPath);
    return false;
  }
}

// ── File discovery ──────────────────────────────────────────────────

function* walkFlac(dir) {
  const entries = fs.readdirSync(dir, { withFileTypes: true });
  for (const entry of entries) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) {
      yield* walkFlac(full);
    } else if (entry.isFile() && /\.flac$/i.test(entry.name)) {
      yield full;
    }
  }
}

// ── Artist issue summary ────────────────────────────────────────────

function inferTrackIdentity(rel, topDir) {
  const relParts = rel.split(path.sep).filter(Boolean);
  const track = relParts.at(-1) || path.basename(rel);
  if (relParts.length >= 3) {
    return {
      artist: relParts[0] || "Unknown Artist",
      album: relParts.slice(1, -1).join(" / ") || "Unknown Album",
      track,
    };
  }
  if (relParts.length === 2) {
    return {
      artist: path.basename(topDir) || "Unknown Artist",
      album: relParts[0] || "Unknown Album",
      track,
    };
  }
  return {
    artist: path.basename(path.dirname(topDir)) || "Unknown Artist",
    album: path.basename(topDir) || "Unknown Album",
    track,
  };
}

function getArtistStats(artistIssues, artistName) {
  if (!artistIssues.has(artistName)) {
    artistIssues.set(artistName, {
      totalFiles: 0,
      filesWithIssues: 0,
      vorbisMismatch: 0,
      audioCorrupted: 0,
      metadataGap: 0,
      corruptedPicture: 0,
      brokenChain: 0,
      strictDecodeFailed: 0,
      strictDecodeWarnings: 0,
      playableStrictDecodeFailures: 0,
      errors: 0,
    });
  }
  return artistIssues.get(artistName);
}

function printArtistIssueSummary(artistIssues) {
  console.log();
  console.log("Issues by artist:");

  const rows = [...artistIssues.entries()]
    .filter(([, artist]) => artist.filesWithIssues > 0 || artist.errors > 0)
    .sort(([a], [b]) => a.localeCompare(b));

  if (rows.length === 0) {
    console.log("  None");
    return;
  }

  for (const [artistName, artist] of rows) {
    const parts = [];
    if (artist.vorbisMismatch > 0) parts.push(`Vorbis length mismatch: ${artist.vorbisMismatch}`);
    if (artist.audioCorrupted > 0) parts.push(`audio frame corrupted: ${artist.audioCorrupted}`);
    if (artist.metadataGap > 0) parts.push(`metadata/audio gap: ${artist.metadataGap}`);
    if (artist.corruptedPicture > 0) parts.push(`corrupted picture tag: ${artist.corruptedPicture}`);
    if (artist.brokenChain > 0) parts.push(`broken metadata chain: ${artist.brokenChain}`);
    if (artist.strictDecodeFailed > 0) parts.push(`strict decode failed: ${artist.strictDecodeFailed}`);
    if (artist.strictDecodeWarnings > 0) parts.push(`strict decode warning: ${artist.strictDecodeWarnings}`);
    if (artist.playableStrictDecodeFailures > 0) parts.push(`playable strict decode failure: ${artist.playableStrictDecodeFailures}`);
    if (artist.errors > 0) parts.push(`errors: ${artist.errors}`);
    console.log(`  ${artistName}: ${artist.filesWithIssues}/${artist.totalFiles} files with issues; ${parts.join("; ")}`);
  }
}

function printSeveritySummary(severityResults) {
  console.log();
  console.log("Results by bucket:");
  printSeverityGroup("CLEAN", severityResults.clean);
  printSeverityGroup("MINOR", severityResults.minor);
  printSeverityGroup("MEDIUM", severityResults.medium);
  printSeverityGroup("BROKEN", severityResults.broken);
}

function printSeverityGroup(label, items) {
  console.log(`  ${label}:`);
  if (items.length === 0) {
    console.log("    None");
    return;
  }

  for (const item of items) {
    console.log(`    ${item.file} (${item.reasons.join("; ")})`);
  }
}

function emptyCounters() {
  return {
    ok: 0,
    errors: 0,
    vorbisMismatch: 0,
    audioCorrupted: 0,
    strictDecodeFailed: 0,
    strictDecodeWarnings: 0,
    playableStrictDecodeFailures: 0,
    strictDecodeUnchecked: 0,
    fixedVorbis: 0,
    fixedAudio: 0,
    fixedChain: 0,
    fixed: 0,
    fixFailed: 0,
  };
}

function decodeSummary(result, tool) {
  if (!result) return { ok: null, tool };
  const summary = { ok: result.ok, tool };
  if (result.message) summary.message = result.message;
  if (result.warning) summary.warning = true;
  if (result.unavailable) summary.unavailable = true;
  return summary;
}

function playbackSummary(diagnosis) {
  const flags = diagnosis.flags || {};
  if (flags.hasUnplayableStrictDecodeIssue) {
    const probeMessage = diagnosis.info?.audioProbe?.message;
    const decodeMessage = diagnosis.info?.playbackDecode?.message;
    return {
      ok: false,
      tool: probeMessage ? "ffprobe" : "ffmpeg",
      message: probeMessage || decodeMessage || "compatibility playback probe failed",
    };
  }
  if (flags.hasKnownStructuralRepair && diagnosis.info?.strictDecode?.ok === false) {
    return {
      ok: false,
      tool: "compatibility",
      message: "strict FLAC structure is invalid; repair is required before treating the track as playable",
    };
  }
  return decodeSummary(diagnosis.info?.playbackDecode, "ffmpeg");
}

function diagnoseTrack(filePath, topDir) {
  const rel = path.relative(topDir, filePath);
  const identity = inferTrackIdentity(rel, topDir);
  const info = analyzeFlac(filePath);

  if (info.error) {
    return {
      rel,
      identity,
      info,
      bucket: "broken",
      issues: [info.brokenChain ? "broken-metadata-chain" : "flac-parse-error"],
      knownRepairs: info.brokenChain ? ["fix-broken-metadata-chain"] : [],
      action: "redownload",
      error: info.error,
    };
  }

  const issues = [];
  const knownRepairs = [];
  const hasVorbisIssue = info.mismatch > 0;
  const hasAudioIssue = info.audioSyncCorrupted;
  const hasGapIssue = info.metadataAudioGap > 0;
  const hasPictureIssue = info.corruptedPictureTag;
  const hasChainIssue = info.brokenChain;
  const hasStrictDecodeIssue = info.strictDecode?.ok === false;
  const hasAudioProbeIssue = hasStrictDecodeIssue && info.audioProbe?.ok === false;
  const hasPlaybackDecodeFailure = hasStrictDecodeIssue && info.playbackDecode?.ok === false;
  const hasUnplayableStrictDecodeIssue = hasAudioProbeIssue || hasPlaybackDecodeFailure;
  const hasKnownStructuralRepair = hasVorbisIssue || hasAudioIssue || hasGapIssue || hasPictureIssue || hasChainIssue;
  const hasPlayableStrictDecodeIssue = hasStrictDecodeIssue && info.playbackDecode?.ok === true && !hasAudioProbeIssue && !hasKnownStructuralRepair;
  const hasStrictDecodeAlert = hasStrictDecodeIssue && (!hasPlayableStrictDecodeIssue || hasAudioProbeIssue);
  const hasStrictDecodeWarning = info.strictDecode?.warning === true;
  const strictDecodeUnchecked = info.strictDecode?.ok === null;

  if (hasVorbisIssue) {
    issues.push("vorbis-length-mismatch");
    knownRepairs.push("fix-vorbis-length");
  }
  if (hasAudioIssue) {
    issues.push("audio-sync-corrupted");
    knownRepairs.push("fix-audio-sync");
  }
  if (hasGapIssue) {
    issues.push("metadata-audio-gap");
    knownRepairs.push("fix-metadata-audio-gap");
  }
  if (hasPictureIssue) {
    issues.push("corrupted-picture-tag");
    knownRepairs.push("fix-corrupted-picture-tag");
  }
  if (hasChainIssue) {
    issues.push("broken-metadata-chain");
    knownRepairs.push("fix-broken-metadata-chain");
  }
  if (hasStrictDecodeAlert) issues.push("strict-decode-failed");
  if (hasAudioProbeIssue) issues.push("ffprobe-audio-probe-failed");
  if (hasPlayableStrictDecodeIssue) {
    issues.push("strict-decode-invalid-playable");
  }
  if (hasStrictDecodeWarning) issues.push("strict-decode-warning");
  if (strictDecodeUnchecked) issues.push("strict-decode-unchecked");

  let bucket = "clean";
  if (hasUnplayableStrictDecodeIssue) {
    bucket = "broken";
  } else if (knownRepairs.length > 0) {
    bucket = "minor";
  } else if (hasStrictDecodeAlert) {
    bucket = "broken";
  } else if (hasPlayableStrictDecodeIssue || hasStrictDecodeWarning) {
    bucket = "medium";
  }

  return {
    rel,
    identity,
    info,
    bucket,
    issues,
    knownRepairs,
    action: bucket === "minor" ? "would_fix" : bucket === "broken" ? "redownload" : "none",
    flags: {
      hasVorbisIssue,
      hasAudioIssue,
      hasGapIssue,
      hasPictureIssue,
      hasChainIssue,
      hasKnownStructuralRepair,
      hasStrictDecodeAlert,
      hasPlayableStrictDecodeIssue,
      hasAudioProbeIssue,
      hasPlaybackDecodeFailure,
      hasUnplayableStrictDecodeIssue,
      hasStrictDecodeWarning,
      strictDecodeUnchecked,
    },
  };
}

function diagnosisDetails(diagnosis) {
  return {
    artist: diagnosis.identity.artist,
    album: diagnosis.identity.album,
    track: diagnosis.identity.track,
    relativePath: diagnosis.rel,
    bucket: diagnosis.bucket,
    issues: diagnosis.issues,
    knownRepairs: diagnosis.knownRepairs,
    action: diagnosis.action,
    playback: playbackSummary(diagnosis),
    probe: decodeSummary(diagnosis.info?.audioProbe, "ffprobe"),
    strict: decodeSummary(diagnosis.info?.strictDecode, "flac -t -w"),
  };
}

function countersForDiagnosis(diagnosis) {
  const counters = emptyCounters();
  const f = diagnosis.flags || {};
  counters.ok = diagnosis.bucket === "clean" ? 1 : 0;
  counters.vorbisMismatch = f.hasVorbisIssue ? 1 : 0;
  counters.audioCorrupted = f.hasAudioIssue ? 1 : 0;
  counters.strictDecodeFailed = f.hasStrictDecodeAlert ? 1 : 0;
  counters.strictDecodeWarnings = (f.hasStrictDecodeWarning || f.hasPlayableStrictDecodeIssue) ? 1 : 0;
  counters.playableStrictDecodeFailures = f.hasPlayableStrictDecodeIssue ? 1 : 0;
  counters.strictDecodeUnchecked = f.strictDecodeUnchecked ? 1 : 0;
  return counters;
}

function artistDeltaForDiagnosis(diagnosis) {
  const f = diagnosis.flags || {};
  const filesWithIssues = diagnosis.bucket === "clean" ? 0 : 1;
  const delta = { totalFiles: 1, filesWithIssues };
  if (f.hasVorbisIssue) delta.vorbisMismatch = 1;
  if (f.hasAudioIssue) delta.audioCorrupted = 1;
  if (f.hasGapIssue) delta.metadataGap = 1;
  if (f.hasPictureIssue) delta.corruptedPicture = 1;
  if (f.hasChainIssue) delta.brokenChain = 1;
  if (f.hasStrictDecodeAlert) delta.strictDecodeFailed = 1;
  if (f.hasStrictDecodeWarning || f.hasPlayableStrictDecodeIssue) delta.strictDecodeWarnings = 1;
  if (f.hasPlayableStrictDecodeIssue) delta.playableStrictDecodeFailures = 1;
  return delta;
}

// ── Worker mode ─────────────────────────────────────────────────────

/**
 * Process a single file and return a structured result.
 * This is the core logic extracted from the old main() loop.
 */
function processFile(filePath, topDir, doctor, dryRun, verbose) {
  const rel = path.relative(topDir, filePath);
  const statusLines = [];
  const log = (line) => statusLines.push(line);

  let diagnosis;
  try {
    diagnosis = diagnoseTrack(filePath, topDir);
  } catch (err) {
    const identity = inferTrackIdentity(rel, topDir);
    const counters = emptyCounters();
    counters.errors = 1;
    return {
      rel,
      artistName: identity.artist,
      statusLines: [`  ERROR  ${rel}: ${err.message}`],
      error: err.message,
      bucket: "broken",
      diagnosis: {
        artist: identity.artist,
        album: identity.album,
        track: identity.track,
        relativePath: rel,
        bucket: "broken",
        issues: ["diagnosis-error"],
        knownRepairs: [],
        action: "redownload",
        playback: { ok: null, tool: "ffmpeg" },
        strict: { ok: null, tool: "flac -t -w" },
      },
      counters,
      artistDelta: { totalFiles: 1, filesWithIssues: 1, errors: 1 },
      severity: { level: "BROKEN", reasons: [err.message] },
    };
  }

  const info = diagnosis.info || {};
  const flags = diagnosis.flags || {};
  const hasAnyIssue = diagnosis.bucket !== "clean";

  if (!hasAnyIssue) {
    if (flags.strictDecodeUnchecked) {
      if (verbose) log(`  OK     ${rel}  (strict decode not checked: ${info.strictDecode.message})`);
    } else if (verbose) {
      log(`  OK     ${rel}`);
    }
    const counters = countersForDiagnosis(diagnosis);
    return {
      rel, artistName: diagnosis.identity.artist, statusLines, error: null,
      bucket: diagnosis.bucket,
      diagnosis: diagnosisDetails(diagnosis),
      counters,
      artistDelta: { totalFiles: 1, filesWithIssues: 0 },
      severity: { level: "CLEAN", reasons: [] },
    };
  }

  const severityReasons = diagnosis.issues.map((issue) => issue.replace(/-/g, " "));

  if (flags.hasVorbisIssue) log(`  MINOR_VORBIS_MISMATCH  ${rel}\n                   header says ${info.headerLen} bytes, actual content is ${info.actualLen} bytes (+${info.mismatch} trailing bytes)`);
  if (flags.hasAudioIssue) log(`  MINOR_AUDIO_CORRUPTED  ${rel}  (first audio byte 0x7f instead of 0xff)`);
  if (flags.hasGapIssue) log(`  MINOR_METADATA_GAP    ${rel}  (${info.metadataAudioGap} zero bytes between metadata and audio)`);
  if (flags.hasPictureIssue) log(`  MINOR_CORRUPTED_PICTURE  ${rel}  (METADATA_BLOCK_PICTURE contains raw image data)`);
  if (flags.hasChainIssue) log(`  MINOR_BROKEN_CHAIN      ${rel}  (metadata block is marked last before later metadata blocks)`);
  if (diagnosis.bucket === "broken" && flags.hasStrictDecodeAlert) {
    const position = formatDecodePosition(info.strictDecode);
    const playbackMessage = info.playbackDecode?.message ? ` | ffmpeg decode: ${info.playbackDecode.message}` : "";
    const probeMessage = info.audioProbe?.message ? ` | ffprobe probe: ${info.audioProbe.message}` : "";
    log(`  BROKEN_STRICT_DECODE_FAIL  ${rel}  (Strict decode failed${position})\n                     ${info.strictDecode.message}${playbackMessage}${probeMessage}`);
  }
  if (diagnosis.bucket === "medium" && flags.hasPlayableStrictDecodeIssue) {
    const position = formatDecodePosition(info.strictDecode);
    log(`  MEDIUM_STRICT_DECODE_PLAYABLE  ${rel}  (Playable by ffmpeg; strict FLAC decode reported an error${position})\n                              ${info.strictDecode.message}`);
  }
  if (diagnosis.bucket === "medium" && flags.hasStrictDecodeWarning) {
    log(`  MEDIUM_STRICT_DECODE  ${rel}  (Playable but strict decode reported a warning)\n                         ${info.strictDecode.message}`);
  }

  const counters = countersForDiagnosis(diagnosis);
  const artistDelta = artistDeltaForDiagnosis(diagnosis);

  if (doctor || dryRun) {
    return {
      rel,
      artistName: diagnosis.identity.artist,
      statusLines,
      error: null,
      bucket: diagnosis.bucket,
      diagnosis: diagnosisDetails(diagnosis),
      counters,
      artistDelta,
      severity: { level: diagnosis.bucket.toUpperCase(), reasons: severityReasons },
    };
  }

  if (diagnosis.bucket !== "minor") {
    if (diagnosis.bucket === "medium" && flags.hasPlayableStrictDecodeIssue) {
      log(`                   ! playable strict decode failure is medium; no safe repair is known`);
    } else if (diagnosis.bucket === "medium") {
      log(`                   ! medium issue; no safe repair is known`);
    } else if (diagnosis.bucket === "broken") {
      log(`                   ✗ broken; restore, redownload, or re-rip the audio`);
    }
    return {
      rel,
      artistName: diagnosis.identity.artist,
      statusLines,
      error: null,
      bucket: diagnosis.bucket,
      diagnosis: diagnosisDetails(diagnosis),
      counters,
      artistDelta,
      severity: { level: diagnosis.bucket.toUpperCase(), reasons: severityReasons },
    };
  }

  // Fix mode
  let vorbisFixed = !flags.hasVorbisIssue;
  let audioFixed = !flags.hasAudioIssue;
  let gapFixed = !flags.hasGapIssue;
  let pictureFixed = !flags.hasPictureIssue;
  let chainFixed = !flags.hasChainIssue;

  if (flags.hasVorbisIssue) { try { vorbisFixed = fixFlacDirect(filePath); } catch (_) {} if (!vorbisFixed) { try { vorbisFixed = fixFlacFfmpeg(filePath); } catch (_) {} } }
  if (flags.hasAudioIssue) { try { audioFixed = fixAudioSync(filePath); } catch (_) {} }
  if (flags.hasGapIssue) { try { gapFixed = fixMetadataAudioGap(filePath); } catch (_) {} }
  if (flags.hasPictureIssue) { try { pictureFixed = fixCorruptedPictureTag(filePath); } catch (_) {} }
  if (flags.hasChainIssue) { try { chainFixed = fixBrokenMetadataChain(filePath); } catch (_) {} }

  if (vorbisFixed && audioFixed && gapFixed && pictureFixed && chainFixed) {
    log(`                   ✓ fixed`);
    let finalDiagnosis = diagnosis;
    try {
      finalDiagnosis = diagnoseTrack(filePath, topDir);
    } catch (err) {
      finalDiagnosis = {
        ...diagnosis,
        bucket: "broken",
        issues: ["post-fix-diagnosis-error"],
        knownRepairs: [],
        action: "fix_failed",
      };
    }
    const repairResolved = finalDiagnosis.bucket === "clean" || (
      finalDiagnosis.bucket === "medium" &&
      finalDiagnosis.knownRepairs.length === 0 &&
      !finalDiagnosis.issues.includes("strict-decode-failed")
    );
    finalDiagnosis.action = repairResolved ? "fixed" : "fix_failed";
    const finalCounters = countersForDiagnosis(finalDiagnosis);
    finalCounters.fixedVorbis = flags.hasVorbisIssue ? 1 : 0;
    finalCounters.fixedAudio = flags.hasAudioIssue ? 1 : 0;
    finalCounters.fixedChain = flags.hasChainIssue ? 1 : 0;
    finalCounters.fixed = 1;
    if (!repairResolved) {
      finalCounters.fixFailed = 1;
    }
    return {
      rel,
      artistName: finalDiagnosis.identity.artist,
      statusLines,
      error: repairResolved ? null : "repair did not resolve known structural issues",
      bucket: finalDiagnosis.bucket,
      diagnosis: diagnosisDetails(finalDiagnosis),
      counters: finalCounters,
      artistDelta: artistDeltaForDiagnosis(finalDiagnosis),
      severity: { level: finalDiagnosis.bucket.toUpperCase(), reasons: finalDiagnosis.issues.map((issue) => issue.replace(/-/g, " ")) },
    };
  } else {
    counters.errors = 1;
    counters.fixFailed = 1;
    const failed = [];
    if (flags.hasVorbisIssue && !vorbisFixed) failed.push("vorbis");
    if (flags.hasAudioIssue && !audioFixed) failed.push("audio");
    if (flags.hasGapIssue && !gapFixed) failed.push("gap");
    if (flags.hasPictureIssue && !pictureFixed) failed.push("picture");
    if (flags.hasChainIssue && !chainFixed) failed.push("chain");
    log(`                   ✗ ${failed.join(" + ")} fix failed`);
  }

  diagnosis.action = "fix_failed";
  return {
    rel,
    artistName: diagnosis.identity.artist,
    statusLines,
    error: "repair failed",
    bucket: diagnosis.bucket,
    diagnosis: diagnosisDetails(diagnosis),
    counters,
    artistDelta,
    severity: { level: diagnosis.bucket.toUpperCase(), reasons: severityReasons },
  };
}

/**
 * Worker mode: process a batch file and write results to checkpoint.
 */
function runWorker(batchFile) {
  const batch = JSON.parse(fs.readFileSync(batchFile, "utf8"));
  const { topDir, files, doctor, dryRun, verbose, checkpointDir, batchId } = batch;
  const results = [];

  for (const filePath of files) {
    const result = processFile(filePath, topDir, doctor, dryRun, verbose);
    results.push(result);
  }

  // Write batch result to checkpoint
  if (checkpointDir) {
    writeBatchResult(checkpointDir, batchId, results);
  }

  // Output results as JSON to stdout for parent to collect
  process.stdout.write(JSON.stringify({ batchId, results }));
}

// ── Parent orchestrator ─────────────────────────────────────────────

/**
 * Find the most recent checkpoint directory for resume.
 */
function findLatestCheckpoint(topDir) {
  const checkpointsDir = path.resolve(process.cwd(), "reports", "checkpoints");
  if (!fs.existsSync(checkpointsDir)) return null;

  const entries = fs.readdirSync(checkpointsDir)
    .filter(e => e.startsWith("scan-") && fs.statSync(path.join(checkpointsDir, e)).isDirectory())
    .sort()
    .reverse();

  for (const entry of entries) {
    const dir = path.join(checkpointsDir, entry);
    const manifestPath = path.join(dir, "manifest.json");
    if (fs.existsSync(manifestPath)) {
      try {
        const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8"));
        if (manifest.topDir === topDir) {
          return dir;
        }
      } catch (_) {}
    }
  }
  return null;
}

async function runParent() {
  const effectiveConcurrency = CONCURRENCY || DEFAULT_CONCURRENCY;
  const effectiveBatchSize = BATCH_SIZE || DEFAULT_BATCH_SIZE;
  let checkpointDir;
  if (CHECKPOINT_DIR) {
    checkpointDir = CHECKPOINT_DIR;
  } else if (RESUME) {
    checkpointDir = findLatestCheckpoint(TOP_DIR);
    if (!checkpointDir) {
      console.log("No previous checkpoint found. Starting fresh scan.");
      checkpointDir = defaultCheckpointDir();
    }
  } else {
    checkpointDir = defaultCheckpointDir();
  }

  console.log(`Scanning: ${TOP_DIR}`);
  console.log(`Mode:     ${DRY_RUN ? "dry-run" : DOCTOR ? "doctor (diagnosis only)" : "fix"}`);
  console.log(`Concurrency: ${effectiveConcurrency}, Batch size: ${effectiveBatchSize}`);
  if (checkpointDir) console.log(`Checkpoint: ${checkpointDir}`);
  console.log();

  // Collect all files
  const allFiles = [];
  for (const filePath of walkFlac(TOP_DIR)) {
    allFiles.push(filePath);
  }
  console.log(`Found ${allFiles.length} FLAC files.`);

  // Load checkpoint
  const processed = loadCheckpoint(checkpointDir);
  console.log(`Already processed: ${processed.size} files.`);

  // Filter unprocessed files
  const unprocessed = allFiles.filter(f => !processed.has(path.relative(TOP_DIR, f)));
  console.log(`Remaining: ${unprocessed.length} files.`);
  console.log();

  if (unprocessed.length === 0) {
    console.log("All files already processed. Use --resume without --checkpoint to re-scan.");
    // Still print summary from checkpoint
    aggregateAndPrint(checkpointDir, allFiles);
    return;
  }

  // Write manifest
  if (checkpointDir) {
    writeManifest(checkpointDir, {
      topDir: TOP_DIR,
      mode: DOCTOR ? "doctor" : DRY_RUN ? "dry-run" : "fix",
      startedAt: new Date().toISOString(),
      totalFiles: allFiles.length,
      concurrency: effectiveConcurrency,
      batchSize: effectiveBatchSize,
    });
  }

  // Partition into batches
  const batches = [];
  for (let i = 0; i < unprocessed.length; i += effectiveBatchSize) {
    batches.push(unprocessed.slice(i, i + effectiveBatchSize));
  }
  console.log(`Split into ${batches.length} batches.`);
  console.log();

  // Create batch input files
  const batchDir = checkpointDir ? path.join(checkpointDir, "pending") : path.resolve(process.cwd(), ".tmp-flac-batches");
  fs.mkdirSync(batchDir, { recursive: true });
  const batchFiles = [];
  for (let i = 0; i < batches.length; i++) {
    const batchFile = path.join(batchDir, `batch-${String(i + 1).padStart(6, "0")}.json`);
    fs.writeFileSync(batchFile, JSON.stringify({
      topDir: TOP_DIR,
      files: batches[i],
      doctor: DOCTOR,
      dryRun: DRY_RUN,
      verbose: VERBOSE,
      checkpointDir,
      batchId: i + 1,
    }), "utf8");
    batchFiles.push(batchFile);
  }

  // Run workers with concurrency control
  const results = [];
  let completed = 0;
  const startTime = Date.now();

  // Handle SIGINT
  let interrupted = false;
  const sigintHandler = () => {
    interrupted = true;
    console.log("\nInterrupted. Saving partial results...");
  };
  process.on("SIGINT", sigintHandler);

  // Process batches with concurrency
  const queue = [...batchFiles];
  const running = new Set();

  async function runNext() {
    if (interrupted || queue.length === 0) return;
    const batchFile = queue.shift();
    const batchId = path.basename(batchFile, ".json").replace("batch-", "");

    const child = new Promise((resolve, reject) => {
      const childProcess = require("child_process").fork(
        process.argv[1],
        ["--worker-batch=" + batchFile],
        { stdio: ["ignore", "pipe", "pipe", "ipc"] }
      );

      let stdout = "";
      childProcess.stdout.on("data", (data) => { stdout += data; });
      childProcess.stderr.on("data", (data) => { process.stderr.write(data); });

      childProcess.on("close", (code) => {
        if (code !== 0) {
          reject(new Error(`Worker batch ${batchId} exited with code ${code}`));
        } else {
          try {
            const batchResults = JSON.parse(stdout);
            resolve(batchResults);
          } catch (err) {
            reject(new Error(`Failed to parse worker output: ${err.message}`));
          }
        }
      });

      childProcess.on("error", reject);
    });

    running.add(child);
    try {
      const batchResults = await child;
      results.push(...batchResults.results);
      completed++;
      const elapsed = ((Date.now() - startTime) / 1000).toFixed(1);
      const total = unprocessed.length;
      const processed = results.length;
      process.stdout.write(`\r  Progress: ${processed}/${total} files (${elapsed}s)`);
    } catch (err) {
      console.error(`\n  Error in batch ${batchId}: ${err.message}`);
    } finally {
      running.delete(child);
    }

    if (!interrupted) await runNext();
  }

  // Start workers
  const workers = [];
  for (let i = 0; i < Math.min(effectiveConcurrency, batchFiles.length); i++) {
    workers.push(runNext());
  }
  await Promise.allSettled(workers);

  process.removeListener("SIGINT", sigintHandler);
  console.log();
  console.log();

  // Clean up batch files (optional)
  if (!checkpointDir) {
    try { fs.rmSync(batchDir, { recursive: true }); } catch (_) {}
  }

  // Aggregate checkpointed files plus this run. Current-run results win.
  aggregateResults([...processed.values(), ...results], allFiles);
}

/**
 * Aggregate results and print summary.
 */
function aggregateResults(results, allFiles) {
  const artistIssues = new Map();
  const severityResults = { clean: [], minor: [], medium: [], broken: [] };
  const stats = {
    total: allFiles.length,
    ok: 0,
    clean: 0,
    minor: 0,
    medium: 0,
    broken: 0,
    vorbisMismatch: 0,
    audioCorrupted: 0,
    strictDecodeFailed: 0,
    strictDecodeWarnings: 0,
    playableStrictDecodeFailures: 0,
    strictDecodeUnchecked: 0,
    fixedVorbis: 0,
    fixedAudio: 0,
    fixedChain: 0,
    fixed: 0,
    fixFailed: 0,
    errors: 0,
  };
  const report = {
    generatedAt: new Date().toISOString(),
    mode: DOCTOR ? "doctor" : DRY_RUN ? "dry-run" : "fix",
    sourceRoot: TOP_DIR,
    summary: {
      total: allFiles.length,
      clean: 0,
      minor: 0,
      medium: 0,
      broken: 0,
      fixed: 0,
      fixFailed: 0,
    },
    diagnosis: {},
    details: {},
  };

  // Also include results from checkpoint that weren't in this run
  // (they were already processed in a previous run)
  const resultMap = new Map();
  for (const r of results) {
    resultMap.set(r.rel, r);
    resultMap.set(r.rel.normalize("NFC"), r);
    resultMap.set(r.rel.normalize("NFD"), r);
  }

  // Process all files
  for (const filePath of allFiles) {
    const rel = path.relative(TOP_DIR, filePath);
    const r = resultMap.get(rel) || resultMap.get(rel.normalize("NFC")) || resultMap.get(rel.normalize("NFD"));

    if (r) {
      // Print status lines from this run
      for (const line of r.statusLines) {
        console.log(line);
      }

      // Aggregate stats
      stats.ok += r.counters.ok;
      stats.clean += r.bucket === "clean" ? 1 : 0;
      stats.minor += r.bucket === "minor" ? 1 : 0;
      stats.medium += r.bucket === "medium" ? 1 : 0;
      stats.broken += r.bucket === "broken" ? 1 : 0;
      stats.errors += r.counters.errors;
      stats.vorbisMismatch += r.counters.vorbisMismatch;
      stats.audioCorrupted += r.counters.audioCorrupted;
      stats.strictDecodeFailed += r.counters.strictDecodeFailed;
      stats.strictDecodeWarnings += r.counters.strictDecodeWarnings;
      stats.playableStrictDecodeFailures += r.counters.playableStrictDecodeFailures;
      stats.strictDecodeUnchecked += r.counters.strictDecodeUnchecked;
      stats.fixedVorbis += r.counters.fixedVorbis;
      stats.fixedAudio += r.counters.fixedAudio;
      stats.fixedChain += r.counters.fixedChain;
      stats.fixed += r.counters.fixed || 0;
      stats.fixFailed += r.counters.fixFailed || 0;

      // Artist stats
      const artist = getArtistStats(artistIssues, r.artistName);
      artist.totalFiles += r.artistDelta.totalFiles || 1;
      artist.filesWithIssues += r.artistDelta.filesWithIssues || 0;
      artist.vorbisMismatch += r.artistDelta.vorbisMismatch || 0;
      artist.audioCorrupted += r.artistDelta.audioCorrupted || 0;
      artist.metadataGap += r.artistDelta.metadataGap || 0;
      artist.corruptedPicture += r.artistDelta.corruptedPicture || 0;
      artist.brokenChain += r.artistDelta.brokenChain || 0;
      artist.strictDecodeFailed += r.artistDelta.strictDecodeFailed || 0;
      artist.strictDecodeWarnings += r.artistDelta.strictDecodeWarnings || 0;
      artist.playableStrictDecodeFailures += r.artistDelta.playableStrictDecodeFailures || 0;
      artist.errors += r.artistDelta.errors || 0;

      // Severity
      if (r.severity) {
        const bucket = (r.bucket || "clean").toLowerCase();
        severityResults[bucket].push({ file: rel, reasons: r.severity.reasons });
      }
      if (r.diagnosis) {
        const d = r.diagnosis;
        if (!report.diagnosis[d.artist]) report.diagnosis[d.artist] = {};
        if (!report.diagnosis[d.artist][d.album]) report.diagnosis[d.artist][d.album] = {};
        report.diagnosis[d.artist][d.album][d.track] = [d.bucket];
        report.details[rel] = d;
      }
    } else {
      stats.errors++;
      stats.broken++;
      const identity = inferTrackIdentity(rel, TOP_DIR);
      if (!report.diagnosis[identity.artist]) report.diagnosis[identity.artist] = {};
      if (!report.diagnosis[identity.artist][identity.album]) report.diagnosis[identity.artist][identity.album] = {};
      report.diagnosis[identity.artist][identity.album][identity.track] = ["broken"];
      report.details[rel] = {
        artist: identity.artist,
        album: identity.album,
        track: identity.track,
        relativePath: rel,
        bucket: "broken",
        issues: ["missing-diagnosis-result"],
        knownRepairs: [],
        action: "redownload",
        playback: { ok: null, tool: "ffmpeg" },
        strict: { ok: null, tool: "flac -t -w" },
      };
      severityResults.broken.push({ file: rel, reasons: ["missing diagnosis result"] });
    }
  }
  report.summary.clean = stats.clean;
  report.summary.minor = stats.minor;
  report.summary.medium = stats.medium;
  report.summary.broken = stats.broken;
  report.summary.fixed = stats.fixed;
  report.summary.fixFailed = stats.fixFailed;

  // Print summary
  console.log();
  console.log("─".repeat(60));
  console.log(`Total files scanned:       ${stats.total}`);
  console.log(`  OK (no issues):          ${stats.ok}`);
  console.log(`  Clean:                   ${stats.clean}`);
  console.log(`  Minor (known repair):    ${stats.minor}`);
  console.log(`  Medium (unknown repair):  ${stats.medium}`);
  console.log(`  Broken (redownload):     ${stats.broken}`);
  console.log(`  Vorbis length mismatch:   ${stats.vorbisMismatch}`);
  console.log(`  Audio frame corrupted:    ${stats.audioCorrupted}`);
  console.log(`  Strict decode alerts:     ${stats.strictDecodeFailed}`);
  console.log(`  Strict decode warnings:   ${stats.strictDecodeWarnings}`);
  if (stats.strictDecodeUnchecked > 0) {
    console.log(`  Strict decode unchecked:  ${stats.strictDecodeUnchecked}`);
  }
  if (!DOCTOR && !DRY_RUN) {
    console.log(`  Vorbis fixed:             ${stats.fixedVorbis}`);
    console.log(`  Audio fixed:              ${stats.fixedAudio}`);
    console.log(`  Chain fixed:              ${stats.fixedChain}`);
    console.log(`  Fixed:                    ${stats.fixed}`);
    console.log(`  Fix failed:               ${stats.fixFailed}`);
  }
  console.log(`  Errors:                   ${stats.errors}`);
  console.log("─".repeat(60));

  const totalIssues = stats.minor + stats.medium + stats.broken;
  const repairableIssues = stats.minor;
  if (DOCTOR && totalIssues > 0) {
    console.log();
    console.log(
      `Found ${stats.minor} minor file(s), ${stats.medium} medium file(s), ` +
      `${stats.broken} broken file(s).`
    );
    if (stats.minor > 0) {
      console.log(`Run without --doctor to fix minor metadata/audio structural issues.`);
    }
    if (stats.medium > 0 && stats.playableStrictDecodeFailures > 0) {
      console.log(`Playable strict-decode failures are medium: playback works, but no safe repair is known yet.`);
    }
    if (stats.broken > 0) {
      console.log(`Broken files usually require restoring, redownloading, or re-ripping the audio.`);
    }
    if (stats.medium > 0 && stats.strictDecodeWarnings > 0) {
      console.log(`Strict decode warnings are medium unless a safe structural repair is identified.`);
    }
  }

  if (DRY_RUN && totalIssues > 0) {
    console.log();
    if (repairableIssues > 0) {
      console.log(`Would fix ${repairableIssues} minor issue(s). Run without --dry-run to apply.`);
    }
    if (stats.medium > 0 && stats.playableStrictDecodeFailures > 0) {
      console.log(`Would leave ${stats.playableStrictDecodeFailures} playable strict-decode failure(s) as medium.`);
    }
    if (stats.broken > 0) {
      console.log(`Would flag ${stats.broken} broken file(s) for redownload/re-rip.`);
    }
    if (stats.medium > 0 && stats.strictDecodeWarnings > 0) {
      console.log(`Would report ${stats.strictDecodeWarnings} medium strict decode warning(s).`);
    }
  }

  if (totalIssues === 0 && stats.errors === 0) {
    console.log();
    console.log("All files are clean. No action needed.");
  }

  printArtistIssueSummary(artistIssues);
  printSeveritySummary(severityResults);
  saveReportFile(report);

  process.exit(stats.errors > 0 ? 1 : 0);
}

/**
 * Aggregate from checkpoint and print summary (for --resume without new files).
 */
function aggregateAndPrint(checkpointDir, allFiles) {
  const processed = loadCheckpoint(checkpointDir);
  const results = [];
  for (const rel of processed.keys()) {
    results.push(processed.get(rel));
  }
  aggregateResults(results, allFiles);
}

// ── Entry point ─────────────────────────────────────────────────────

if (WORKER_MODE) {
  runWorker(WORKER_BATCH);
} else {
  runParent();
}
