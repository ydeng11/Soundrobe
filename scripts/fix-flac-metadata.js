#!/usr/bin/env node
/**
 * fix-flac-metadata — Detect and fix issues in FLAC files:
 *
 * 1. Vorbis comment block length mismatches (auto-tagger writer bug)
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
 */

const fs = require("fs");
const path = require("path");
const { execFileSync } = require("child_process");

// ── CLI args ────────────────────────────────────────────────────────

const args = process.argv.slice(2);
const flags = args.filter((a) => a.startsWith("--"));
const positional = args.filter((a) => !a.startsWith("--"));

const DOCTOR = flags.includes("--doctor");
const DRY_RUN = flags.includes("--dry-run"  );
const VERBOSE = flags.includes("--verbose") || flags.includes("-v");

if (positional.length === 0) {
  console.error("Usage: node fix-flac-metadata.js <directory> [--doctor] [--dry-run] [--verbose]");
  process.exit(1);
}

const TOP_DIR = path.resolve(positional[0]);

if (!fs.existsSync(TOP_DIR) || !fs.statSync(TOP_DIR).isDirectory()) {
  console.error(`Error: "${TOP_DIR}" is not a directory`);
  process.exit(1);
}

// ── FLAC parsing ────────────────────────────────────────────────────

/**
 * Parse the FLAC metadata block layout from a buffer.
 * Returns { blocks, audioOffset }.
 */
function parseFlacLayout(buf) {
  const blocks = [];
  let offset = 4; // skip "fLaC"

  while (offset + 4 <= buf.length) {
    const byte0 = buf[offset];
    const isLast = !!(byte0 >> 7);
    const type = byte0 & 0x7f;
    const length = (buf[offset + 1] << 16) | (buf[offset + 2] << 8) | buf[offset + 3];
    const headerOffset = offset;
    const dataOffset = offset + 4;

    // Sanity: stop if block extends past EOF or looks invalid
    if (type > 126 || length > 5_000_000 || dataOffset + length > buf.length) break;

    blocks.push({ type, headerOffset, dataOffset, length, isLast });
    if (isLast) break;
    offset = dataOffset + length;
  }

  const last = blocks[blocks.length - 1];
  const audioOffset = last ? last.dataOffset + last.length : 4;
  return { blocks, audioOffset };
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
  if (buf.length < 8 || buf.toString("ascii", 0, 4) !== "fLaC") {
    return { file: filePath, error: "not a valid FLAC file" };
  }

  const layout = parseFlacLayout(buf);
  const vorbisBlock = layout.blocks.find((b) => b.type === 4);

  if (!vorbisBlock) {
    return { file: filePath, error: "no VORBIS_COMMENT block found" };
  }

  const headerLen = vorbisBlock.length;
  const actualLen = computeVorbisContentSize(buf, vorbisBlock.dataOffset, headerLen);

  if (actualLen < 0) {
    return { file: filePath, error: "Vorbis comment block too corrupted to parse" };
  }

  const mismatch = headerLen - actualLen;
  const leftover = mismatch;

  // Check for audio frame sync corruption (first byte bit 7 cleared)
  const audioOffset = layout.audioOffset;
  let audioSyncCorrupted = false;
  if (audioOffset < buf.length) {
    const b0 = buf[audioOffset];
    const b1 = buf[audioOffset + 1] ?? 0;
    // Valid FLAC frame sync: first byte 0xff, second byte top 3 bits all 1
    audioSyncCorrupted = (b0 === 0x7f && (b1 & 0xf8) === 0xf8);
  }

  // Check for gap between last metadata block and first audio frame
  let metadataAudioGap = 0;
  if (layout.blocks.length > 0) {
    const lastBlock = layout.blocks[layout.blocks.length - 1];
    const expectedAudio = lastBlock.dataOffset + lastBlock.length;
    // Scan for FLAC frame sync starting from expected position
    for (let i = expectedAudio; i < Math.min(expectedAudio + 200, buf.length - 1); i++) {
      if (buf[i] === 0xff && (buf[i + 1] & 0xf8) === 0xf8) {
        metadataAudioGap = i - expectedAudio;
        break;
      }
    }
  }

  // Check for corrupted METADATA_BLOCK_PICTURE tag
  let corruptedPictureTag = false;
  if (vorbisBlock) {
    corruptedPictureTag = hasCorruptedMetadataBlockPicture(
      buf, vorbisBlock.dataOffset, vorbisBlock.length
    );
  }

  return {
    file: filePath,
    headerLen,
    actualLen,
    mismatch,
    leftover,
    isLast: vorbisBlock.isLast,
    hasTrailingZeros: mismatch > 0,
    audioOffset,
    audioSyncCorrupted,
    metadataAudioGap,
    corruptedPictureTag,
  };
}

// ── Detection helpers ───────────────────────────────────────────────

/**
 * Check if a Vorbis comment contains a corrupted METADATA_BLOCK_PICTURE tag.
 * The auto-tagger stored raw JPEG/PNG data instead of the proper structure:
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

/**
 * Fix audio frame sync corruption: restore bit 7 of the first audio byte.
 * The auto-tagger writer cleared it (0x7f → 0xff).
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
    fs.writeFileSync(filePath, buf);
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
function fixCorruptedPictureTag(filePath) {
  const buf = Buffer.from(fs.readFileSync(filePath));
  const layout = parseFlacLayout(buf);
  const vorbisBlock = layout.blocks.find((b) => b.type === 4);
  if (!vorbisBlock) return false;

  // Verify the tag exists
  if (!hasCorruptedMetadataBlockPicture(buf, vorbisBlock.dataOffset, vorbisBlock.length)) {
    return false;
  }

  // Find and remove the corrupted tag using metaflac
  try {
    // Use metaflac to remove the specific tag
    execFileSync("metaflac", [
      "--remove-tag=METADATA_BLOCK_PICTURE",
      filePath,
    ], { stdio: "pipe" });
    return true;
  } catch {
    // Fallback: remove the tag manually by rebuilding the Vorbis comment
    // without the METADATA_BLOCK_PICTURE entry
    return removeVorbisTagManually(buf, layout, vorbisBlock, 'METADATA_BLOCK_PICTURE');
  }
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
  fs.writeFileSync(filePath, result);
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

  // Find actual audio offset by scanning for FLAC frame sync
  let actualAudioOffset = -1;
  for (let i = expectedAudioOffset; i < Math.min(expectedAudioOffset + 200, buf.length - 1); i++) {
    if (buf[i] === 0xff && (buf[i + 1] & 0xf8) === 0xf8) {
      actualAudioOffset = i;
      break;
    }
  }

  if (actualAudioOffset < 0 || actualAudioOffset === expectedAudioOffset) return false;

  const gap = actualAudioOffset - expectedAudioOffset;

  // Only fix if the gap is all zeros and the last block is PADDING
  if (lastBlock.type !== 1) return false;
  for (let i = expectedAudioOffset; i < actualAudioOffset; i++) {
    if (buf[i] !== 0) return false;
  }

  // Absorb the gap into the PADDING block's length
  const newPadLen = lastBlock.length + gap;
  buf[lastBlock.headerOffset + 1] = (newPadLen >> 16) & 0xff;
  buf[lastBlock.headerOffset + 2] = (newPadLen >> 8) & 0xff;
  buf[lastBlock.headerOffset + 3] = newPadLen & 0xff;

  fs.writeFileSync(filePath, buf);
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

    fs.writeFileSync(filePath, buf);
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

// ── Main ────────────────────────────────────────────────────────────

function main() {
  console.log(`Scanning: ${TOP_DIR}`);
  console.log(`Mode:     ${DRY_RUN ? "dry-run" : DOCTOR ? "doctor (diagnosis only)" : "fix"}`);
  console.log();

  const stats = {
    total: 0,
    ok: 0,
    vorbisMismatch: 0,
    audioCorrupted: 0,
    fixedVorbis: 0,
    fixedAudio: 0,
    errors: 0,
  };

  for (const filePath of walkFlac(TOP_DIR)) {
    stats.total++;

    let info;
    try {
      info = analyzeFlac(filePath);
    } catch (err) {
      stats.errors++;
      if (VERBOSE) console.error(`  ERROR  ${path.relative(TOP_DIR, filePath)}: ${err.message}`);
      continue;
    }

    if (info.error) {
      stats.errors++;
      if (VERBOSE) console.error(`  ERROR  ${path.relative(TOP_DIR, filePath)}: ${info.error}`);
      continue;
    }

    const rel = path.relative(TOP_DIR, filePath);
    const hasVorbisIssue = info.mismatch > 0;
    const hasAudioIssue = info.audioSyncCorrupted;
    const hasGapIssue = info.metadataAudioGap > 0;
    const hasPictureIssue = info.corruptedPictureTag;

    if (!hasVorbisIssue && !hasAudioIssue && !hasGapIssue && !hasPictureIssue) {
      stats.ok++;
      if (VERBOSE) console.log(`  OK     ${rel}`);
      continue;
    }

    // Report issues
    if (hasVorbisIssue) {
      stats.vorbisMismatch++;
      console.log(
        `  VORBIS_MISMATCH  ${rel}\n` +
        `                   header says ${info.headerLen} bytes, actual content is ${info.actualLen} bytes (+${info.mismatch} trailing bytes)`
      );
    }
    if (hasAudioIssue) {
      stats.audioCorrupted++;
      console.log(`  AUDIO_CORRUPTED  ${rel}  (first audio byte 0x7f instead of 0xff)`);
    }
    if (hasGapIssue) {
      console.log(`  METADATA_GAP    ${rel}  (${info.metadataAudioGap} zero bytes between metadata and audio)`);
    }
    if (hasPictureIssue) {
      console.log(`  CORRUPTED_PICTURE  ${rel}  (METADATA_BLOCK_PICTURE contains raw image data)`);
    }

    if (DOCTOR || DRY_RUN) continue;

    // Fix mode — apply all fixes independently
    let vorbisFixed = !hasVorbisIssue; // already good
    let audioFixed = !hasAudioIssue;   // already good
    let gapFixed = !hasGapIssue;       // already good
    let pictureFixed = !hasPictureIssue; // already good

    if (hasVorbisIssue) {
      try {
        vorbisFixed = fixFlacDirect(filePath);
      } catch (err) {
        if (VERBOSE) console.error(`                   vorbis direct fix failed: ${err.message}`);
      }
      if (!vorbisFixed) {
        try {
          vorbisFixed = fixFlacFfmpeg(filePath);
        } catch (err) {
          if (VERBOSE) console.error(`                   vorbis ffmpeg fix failed: ${err.message}`);
        }
      }
    }

    if (hasAudioIssue) {
      try {
        audioFixed = fixAudioSync(filePath);
      } catch (err) {
        if (VERBOSE) console.error(`                   audio fix failed: ${err.message}`);
      }
    }

    if (hasGapIssue) {
      try {
        gapFixed = fixMetadataAudioGap(filePath);
      } catch (err) {
        if (VERBOSE) console.error(`                   gap fix failed: ${err.message}`);
      }
    }

    if (hasPictureIssue) {
      try {
        pictureFixed = fixCorruptedPictureTag(filePath);
      } catch (err) {
        if (VERBOSE) console.error(`                   picture tag fix failed: ${err.message}`);
      }
    }

    if (vorbisFixed && audioFixed && gapFixed && pictureFixed) {
      stats.fixedVorbis += hasVorbisIssue ? 1 : 0;
      stats.fixedAudio += hasAudioIssue ? 1 : 0;
      console.log(`                   ✓ fixed`);
    } else {
      stats.errors++;
      const failed = [];
      if (hasVorbisIssue && !vorbisFixed) failed.push("vorbis");
      if (hasAudioIssue && !audioFixed) failed.push("audio");
      if (hasGapIssue && !gapFixed) failed.push("gap");
      if (hasPictureIssue && !pictureFixed) failed.push("picture");
      console.log(`                   ✗ ${failed.join(" + ")} fix failed`);
    }
  }

  // Summary
  console.log();
  console.log("─".repeat(60));
  console.log(`Total files scanned:       ${stats.total}`);
  console.log(`  OK (no issues):          ${stats.ok}`);
  console.log(`  Vorbis length mismatch:   ${stats.vorbisMismatch}`);
  console.log(`  Audio frame corrupted:    ${stats.audioCorrupted}`);
  if (!DOCTOR && !DRY_RUN) {
    console.log(`  Vorbis fixed:             ${stats.fixedVorbis}`);
    console.log(`  Audio fixed:              ${stats.fixedAudio}`);
  }
  console.log(`  Errors:                   ${stats.errors}`);
  console.log("─".repeat(60));

  const totalIssues = stats.vorbisMismatch + stats.audioCorrupted;
  if (DOCTOR && totalIssues > 0) {
    console.log();
    console.log(`Found ${stats.vorbisMismatch} file(s) with Vorbis length mismatch, ${stats.audioCorrupted} with audio corruption.`);
    console.log(`Run without --doctor to fix them.`);
  }

  if (DRY_RUN && totalIssues > 0) {
    console.log();
    console.log(`Would fix ${totalIssues} issue(s). Run without --dry-run to apply.`);
  }

  if (totalIssues === 0 && stats.errors === 0) {
    console.log();
    console.log("All files are clean. No action needed.");
  }

  process.exit(stats.errors > 0 ? 1 : 0);
}

main();
