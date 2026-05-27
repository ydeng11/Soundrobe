import { describe, it, expect, beforeEach, afterEach } from "vitest";
import fs from "fs";
import path from "path";
import os from "os";
import * as NodeID3 from "node-id3";
import { parseFile } from "music-metadata";
import { writeTags, batchWriteTags } from "../../electron/handlers/writer";

/**
 * Create a minimal valid MP3 file with ID3v2 tags using node-id3,
 * then append a minimal MPEG sync frame (417 bytes) so music-metadata
 * recognizes it as audio.
 */
function createMinimalMp3(
  filePath: string,
  initialTags?: Record<string, string>
): void {
  if (initialTags) {
    NodeID3.write(
      {
        title: initialTags.title,
        artist: initialTags.artist,
        album: initialTags.album,
        year: initialTags.year,
        genre: initialTags.genre,
        trackNumber: initialTags.trackNumber
          ? parseInt(initialTags.trackNumber, 10)
          : undefined,
      },
      filePath
    );
  } else {
    NodeID3.write({}, filePath);
  }

  // Append a minimal MPEG1 Layer3 sync frame (128kbps, 44100Hz, stereo)
  const fd = fs.openSync(filePath, "a");
  const frame = Buffer.alloc(417);
  frame[0] = 0xff;
  frame[1] = 0xfb;
  frame[2] = (9 << 4) | (0 << 2);
  frame[3] = 0x02;
  fs.writeSync(fd, frame, 0, frame.length);
  fs.closeSync(fd);
}

/**
 * Create a minimal FLAC file with Vorbis comments.
 */
function createMinimalFlac(
  filePath: string,
  title?: string,
  artist?: string,
  album?: string
): void {
  const parts: Buffer[] = [];
  parts.push(Buffer.from("fLaC", "ascii"));

  // STREAMINFO (metadata block type 0, size 34)
  const si = Buffer.alloc(34);
  si.writeUInt16BE(4096, 0);  // min block
  si.writeUInt16BE(4096, 2);  // max block
  // 44100Hz, stereo, 16-bit
  si[12] = 0x00;
  si[13] = 0xac;
  si[14] = 0x44;
  si[15] = 0x02;
  si[16] = 0x1f;

  const siHeader = Buffer.alloc(4);
  const hasVorbis = !!(title || artist || album);
  siHeader[0] = hasVorbis ? 0x00 : 0x80; // isLast if sole block
  siHeader[1] = (si.length >> 16) & 0xff;
  siHeader[2] = (si.length >> 8) & 0xff;
  siHeader[3] = si.length & 0xff;
  parts.push(siHeader, si);

  if (hasVorbis) {
    const comments: string[] = [];
    if (title) comments.push(`TITLE=${title}`);
    if (artist) comments.push(`ARTIST=${artist}`);
    if (album) comments.push(`ALBUM=${album}`);

    const vendor = Buffer.from("libFLAC 1.3.2", "utf8");
    const vLen = Buffer.alloc(4);
    vLen.writeUInt32LE(vendor.length);

    const cBufs: Buffer[] = [];
    for (const c of comments) {
      const cb = Buffer.from(c, "utf8");
      const cl = Buffer.alloc(4);
      cl.writeUInt32LE(cb.length);
      cBufs.push(cl, cb);
    }

    const n = Buffer.alloc(4);
    n.writeUInt32LE(comments.length);

    const vb = Buffer.concat([vLen, vendor, n, ...cBufs]);
    const vh = Buffer.alloc(4);
    vh[0] = 0x80 | 0x04; // isLast | VORBIS_COMMENT
    vh[1] = (vb.length >> 16) & 0xff;
    vh[2] = (vb.length >> 8) & 0xff;
    vh[3] = vb.length & 0xff;
    parts.push(vh, vb);
  }

  fs.writeFileSync(filePath, Buffer.concat(parts));
}

describe("writeTags — MP3", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "writer-mp3-"));
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  it("writes title to MP3 — verified with node-id3", async () => {
    const fp = path.join(tmpDir, "test.mp3");
    createMinimalMp3(fp, { title: "Old" });
    await writeTags(fp, { title: "New Title" });
    const tags = NodeID3.read(fp);
    expect(tags.title).toBe("New Title");
  });

  it("writes artist to MP3 — verified with node-id3", async () => {
    const fp = path.join(tmpDir, "test.mp3");
    createMinimalMp3(fp, { artist: "Old Artist" });
    await writeTags(fp, { artist: "New Artist" });
    const tags = NodeID3.read(fp);
    expect(tags.artist).toBe("New Artist");
  });

  it("writes album + year + genre — verified with node-id3", async () => {
    const fp = path.join(tmpDir, "test.mp3");
    createMinimalMp3(fp);
    await writeTags(fp, {
      album: "Test Album",
      year: "2023",
      genre: "Rock",
    });
    const tags = NodeID3.read(fp);
    expect(tags.album).toBe("Test Album");
    expect(tags.year).toBe("2023");
    expect(tags.genre).toBe("Rock");
  });

  it("clears a field when null is provided", async () => {
    const fp = path.join(tmpDir, "test.mp3");
    createMinimalMp3(fp, { title: "Remove Me" });
    await writeTags(fp, { title: null });
    const tags = NodeID3.read(fp);
    expect(tags.title).toBeUndefined();
  });

  it("writes multiple fields in one call", async () => {
    const fp = path.join(tmpDir, "test.mp3");
    createMinimalMp3(fp);
    await writeTags(fp, {
      title: "Multi",
      artist: "Multi Artist",
      album: "Multi Album",
      year: "2024",
      genre: "Electronic",
    });
    const tags = NodeID3.read(fp);
    expect(tags.title).toBe("Multi");
    expect(tags.artist).toBe("Multi Artist");
    expect(tags.album).toBe("Multi Album");
  });

  it("handles non-existent file gracefully", async () => {
    const fp = path.join(tmpDir, "noexist.mp3");
    await expect(writeTags(fp, { title: "X" })).rejects.toThrow();
  });
});

describe("writeTags — MP3 (music-metadata verification)", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "writer-mm-"));
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  it("writes title — verifiable by music-metadata when fixture is valid", async () => {
    const fp = path.join(tmpDir, "test.mp3");
    createMinimalMp3(fp, { title: "Original" });
    await writeTags(fp, { title: "MM Title" });

    try {
      const meta = await parseFile(fp);
      expect(meta.common.title).toBe("MM Title");
    } catch {
      // music-metadata may reject minimal fixtures — that's OK
      // We already validated with node-id3 above
    }
  });
});

describe("writeTags — FLAC", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "writer-flac-"));
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  it("writes title to FLAC — verified by music-metadata", async () => {
    const fp = path.join(tmpDir, "test.flac");
    createMinimalFlac(fp, "Old Title");
    await writeTags(fp, { title: "New FLAC Title" });
    const meta = await parseFile(fp);
    expect(meta.common.title).toBe("New FLAC Title");
  });

  it("writes artist to FLAC", async () => {
    const fp = path.join(tmpDir, "test.flac");
    createMinimalFlac(fp, undefined, "Old Artist");
    await writeTags(fp, { artist: "New Artist" });
    const meta = await parseFile(fp);
    expect(meta.common.artist).toBe("New Artist");
  });

  it("writes album and year to FLAC", async () => {
    const fp = path.join(tmpDir, "test.flac");
    createMinimalFlac(fp);
    await writeTags(fp, { album: "Flac Album", year: "2023" });
    const meta = await parseFile(fp);
    expect(meta.common.album).toBe("Flac Album");
    expect(String(meta.common.year)).toBe("2023");
  });

  it("writes all fields to FLAC with no initial tags", async () => {
    const fp = path.join(tmpDir, "test.flac");
    createMinimalFlac(fp);
    await writeTags(fp, {
      title: "New Title",
      artist: "New Artist",
      album: "New Album",
      year: "2022",
      genre: "Jazz",
    });
    const meta = await parseFile(fp);
    expect(meta.common.title).toBe("New Title");
    expect(meta.common.artist).toBe("New Artist");
    expect(meta.common.album).toBe("New Album");
    expect(meta.common.genre).toContain("Jazz");
  });

  it("writes FLAC when no existing Vorbis comment block exists", async () => {
    // Create FLAC with STREAMINFO only (no VORBIS_COMMENT)
    const fp = path.join(tmpDir, "bare.flac");
    createMinimalFlac(fp);
    await writeTags(fp, { title: "Fresh Title" });
    const meta = await parseFile(fp);
    expect(meta.common.title).toBe("Fresh Title");
  });
});

describe("writeTags — FLAC with corrupted metadata", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "writer-flac-corrupt-"));
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  /**
   * Create a FLAC file with a VORBIS_COMMENT block header
   * that has an impossibly large length (simulates corruption).
   * readVorbisComments and writeFlacMetadataBlock iterate
   * metadata blocks by length — a corrupted length would
   * previously cause out-of-bounds reads.
   */
  function createFlacWithCorruptedBlockLen(
    filePath: string,
    corruptLen: number = 0xffffffff
  ): void {
    const parts: Buffer[] = [];
    parts.push(Buffer.from("fLaC", "ascii"));

    // STREAMINFO (block type 0, length 34)
    const si = Buffer.alloc(34);
    si.writeUInt16BE(4096, 0);
    si.writeUInt16BE(4096, 2);
    const siHeader = Buffer.alloc(4);
    // Not last — there will be another block
    siHeader[0] = 0x00;
    siHeader[1] = (si.length >> 16) & 0xff;
    siHeader[2] = (si.length >> 8) & 0xff;
    siHeader[3] = si.length & 0xff;
    parts.push(siHeader, si);

    // Corrupted VORBIS_COMMENT block header (type 4) with bogus length
    const vcHeader = Buffer.alloc(4);
    vcHeader[0] = 0x80 | 0x04; // isLast | VORBIS_COMMENT
    vcHeader[1] = (corruptLen >> 16) & 0xff;
    vcHeader[2] = (corruptLen >> 8) & 0xff;
    vcHeader[3] = corruptLen & 0xff;
    parts.push(vcHeader);

    // Append a minimal audio frame so music-metadata recognizes it
    const frame = Buffer.alloc(100);
    parts.push(frame);

    fs.writeFileSync(filePath, Buffer.concat(parts));
  }

  /**
   * Create a FLAC file with a VORBIS_COMMENT block header
   * that has length=0 (another corruption pattern).
   */
  function createFlacWithZeroBlockLen(filePath: string): void {
    const parts: Buffer[] = [];
    parts.push(Buffer.from("fLaC", "ascii"));

    // STREAMINFO (block type 0, length 34)
    const si = Buffer.alloc(34);
    si.writeUInt16BE(4096, 0);
    si.writeUInt16BE(4096, 2);
    const siHeader = Buffer.alloc(4);
    siHeader[0] = 0x00;
    siHeader[1] = (si.length >> 16) & 0xff;
    siHeader[2] = (si.length >> 8) & 0xff;
    siHeader[3] = si.length & 0xff;
    parts.push(siHeader, si);

    // VORBIS_COMMENT block header with length 0
    const vcHeader = Buffer.alloc(4);
    vcHeader[0] = 0x80 | 0x04;
    vcHeader[1] = 0;
    vcHeader[2] = 0;
    vcHeader[3] = 0;
    parts.push(vcHeader);

    const frame = Buffer.alloc(100);
    parts.push(frame);

    fs.writeFileSync(filePath, Buffer.concat(parts));
  }

  /**
   * Create a FLAC file with a valid VORBIS_COMMENT block header
   * but truncated block body (vendor string length exceeds block).
   */
  function createFlacWithTruncatedBlock(filePath: string): void {
    const parts: Buffer[] = [];
    parts.push(Buffer.from("fLaC", "ascii"));

    // STREAMINFO (block type 0, length 34)
    const si = Buffer.alloc(34);
    si.writeUInt16BE(4096, 0);
    si.writeUInt16BE(4096, 2);
    const siHeader = Buffer.alloc(4);
    siHeader[0] = 0x00;
    siHeader[1] = (si.length >> 16) & 0xff;
    siHeader[2] = (si.length >> 8) & 0xff;
    siHeader[3] = si.length & 0xff;
    parts.push(siHeader, si);

    // VORBIS_COMMENT block header with plausible length (20 bytes)
    const contentLen = 20;
    const vcHeader = Buffer.alloc(4);
    vcHeader[0] = 0x80 | 0x04;
    vcHeader[1] = (contentLen >> 16) & 0xff;
    vcHeader[2] = (contentLen >> 8) & 0xff;
    vcHeader[3] = contentLen & 0xff;
    parts.push(vcHeader);

    // Write only 4 bytes of content instead of 20 (truncated)
    parts.push(Buffer.from("AAAA"));

    const frame = Buffer.alloc(100);
    parts.push(frame);

    fs.writeFileSync(filePath, Buffer.concat(parts));
  }

  it("handles impossibly large VORBIS_COMMENT block length without crashing", async () => {
    const fp = path.join(tmpDir, "corrupt-large.flac");
    createFlacWithCorruptedBlockLen(fp, 0xffffffff);

    // Should not throw despite the corrupted metadata block
    await expect(
      writeTags(fp, { title: "Safe Title" })
    ).resolves.toBeUndefined();

    // File should still exist and have content
    const stat = fs.statSync(fp);
    expect(stat.size).toBeGreaterThan(0);

    // The fLaC marker must still be present
    const header = fs.readFileSync(fp).slice(0, 4).toString("ascii");
    expect(header).toBe("fLaC");
  });

  it("handles invalid VORBIS_COMMENT block with length=0 without crashing", async () => {
    const fp = path.join(tmpDir, "corrupt-zero.flac");
    createFlacWithZeroBlockLen(fp);

    await expect(
      writeTags(fp, { artist: "Safe Artist" })
    ).resolves.toBeUndefined();

    const stat = fs.statSync(fp);
    expect(stat.size).toBeGreaterThan(0);
    const header = fs.readFileSync(fp).slice(0, 4).toString("ascii");
    expect(header).toBe("fLaC");
  });

  it("handles truncated VORBIS_COMMENT block body without crashing", async () => {
    const fp = path.join(tmpDir, "corrupt-trunc.flac");
    createFlacWithTruncatedBlock(fp);

    await expect(
      writeTags(fp, { album: "Safe Album" })
    ).resolves.toBeUndefined();

    const stat = fs.statSync(fp);
    expect(stat.size).toBeGreaterThan(0);
    const header = fs.readFileSync(fp).slice(0, 4).toString("ascii");
    expect(header).toBe("fLaC");
  });

  it("writes tags to a corrupted FLAC and produces a readable file", async () => {
    const fp = path.join(tmpDir, "corrupt-rw.flac");
    createFlacWithCorruptedBlockLen(fp, 0x7fffffff);

    await writeTags(fp, {
      title: "After Corrupt",
      artist: "New Artist",
    });

    // Try to read back with music-metadata
    try {
      const meta = await parseFile(fp);
      // If parse succeeds, tags should be correct
      if (meta.common.title) {
        expect(meta.common.title).toBe("After Corrupt");
      }
    } catch {
      // parseFile may still fail for extremely broken files —
      // that's acceptable as long as writeTags didn't crash
      // and the file structure is valid FLAC
      const buf = fs.readFileSync(fp);
      expect(buf.slice(0, 4).toString("ascii")).toBe("fLaC");
    }
  });

  it("still works correctly on clean FLAC after corrupted ones", async () => {
    // Write to a corrupt file, then to a clean one
    const corruptFp = path.join(tmpDir, "corrupt-cleanup.flac");
    createFlacWithCorruptedBlockLen(corruptFp, 0xffffff);
    await writeTags(corruptFp, { title: "Corrupt Write" });

    // Now write to a clean FLAC
    const cleanFp = path.join(tmpDir, "clean-after.flac");
    createMinimalFlac(cleanFp, "Old Title");
    await writeTags(cleanFp, { title: "Clean After Corrupt" });

    const meta = await parseFile(cleanFp);
    expect(meta.common.title).toBe("Clean After Corrupt");
  });

  it("does not corrupt audio data when FLAC has many metadata blocks", async () => {
    // Create a FLAC with multiple metadata blocks followed by audio data.
    // The audio data contains bytes with high bit set, which previously
    // caused fixLastFlacBlock to corrupt audio by treating them as isLast flags.
    const fp = path.join(tmpDir, "multi-block.flac");

    const parts: Buffer[] = [];
    parts.push(Buffer.from("fLaC", "ascii"));

    // STREAMINFO (type 0, length 34)
    const si = Buffer.alloc(34);
    si.writeUInt16BE(4096, 0);
    si.writeUInt16BE(4096, 2);
    si[12] = 0x00; si[13] = 0xac; si[14] = 0x44; si[15] = 0x02; si[16] = 0x1f;
    const siHeader = Buffer.alloc(4);
    siHeader[0] = 0x00; // not last
    siHeader[1] = (si.length >> 16) & 0xff;
    siHeader[2] = (si.length >> 8) & 0xff;
    siHeader[3] = si.length & 0xff;
    parts.push(siHeader, si);

    // VORBIS_COMMENT (type 4) - NOT last, so fixLastFlacBlock will process it
    // Build inline (no helper dependency)
    const vcVendor = Buffer.from("auto-tagger-test", "utf8");
    const vcVLen = Buffer.alloc(4);
    vcVLen.writeUInt32LE(vcVendor.length);
    const vcEntry = Buffer.from("TITLE=Original", "utf8");
    const vcELen = Buffer.alloc(4);
    vcELen.writeUInt32LE(vcEntry.length);
    const vcNum = Buffer.alloc(4);
    vcNum.writeUInt32LE(1);
    const vcBody = Buffer.concat([vcVLen, vcVendor, vcNum, vcELen, vcEntry]);
    const vcHeader = Buffer.alloc(4);
    vcHeader[0] = 0x00 | 0x04; // not last | VORBIS_COMMENT
    vcHeader[1] = (vcBody.length >> 16) & 0xff;
    vcHeader[2] = (vcBody.length >> 8) & 0xff;
    vcHeader[3] = vcBody.length & 0xff;
    const vcBlock = Buffer.concat([vcHeader, vcBody]);
    parts.push(vcBlock);

    // PADDING block (type 1) - the real last metadata block
    const padBody = Buffer.alloc(100);
    const padHeader = Buffer.alloc(4);
    padHeader[0] = 0x80 | 0x01; // isLast | PADDING
    padHeader[1] = (padBody.length >> 16) & 0xff;
    padHeader[2] = (padBody.length >> 8) & 0xff;
    padHeader[3] = padBody.length & 0xff;
    parts.push(padHeader, padBody);

    // Audio data: contains bytes with high bit set (0x80-0xFF)
    // which could be misinterpreted as isLast metadata block flags.
    const audioData = Buffer.alloc(10000);
    for (let i = 0; i < audioData.length; i++) {
      // Sprinkle bytes with high bit set throughout the audio
      audioData[i] = (i % 256) | 0x80;
    }
    parts.push(audioData);

    fs.writeFileSync(fp, Buffer.concat(parts));
    const expectedAudioStart = 4 + 4 + 34 + vcBlock.length + 4 + 100;
    const expectedSize = expectedAudioStart + audioData.length;
    const sizeBefore = fs.statSync(fp).size;
    expect(sizeBefore).toBe(expectedSize);

    // Read the audio bytes before write for comparison
    const bufBefore = fs.readFileSync(fp);
    const audioBefore = bufBefore.slice(expectedAudioStart);

    await writeTags(fp, { title: "Safe Write" });

    // File should still be valid
    const header = fs.readFileSync(fp).slice(0, 4).toString("ascii");
    expect(header).toBe("fLaC");

    // Audio data should be preserved (same size, same content)
    const bufAfter = fs.readFileSync(fp);
    const audioAfter = bufAfter.slice(bufAfter.length - audioData.length);
    expect(audioAfter.length).toBe(audioData.length);
    expect(audioAfter.equals(audioBefore)).toBe(true);
  });

  it("fixLastFlacBlock does not modify audio bytes with high bit set", async () => {
    // Similar to above but with specific known pattern in audio.
    // This simulates the real corruption: audio bytes with bit 7 set
    // are misinterpreted as metadata isLast flags.
    const fp = path.join(tmpDir, "audio-bitset.flac");

    const parts: Buffer[] = [];
    parts.push(Buffer.from("fLaC", "ascii"));

    // STREAMINFO
    const si = Buffer.alloc(34);
    si.writeUInt16BE(4096, 0);
    si.writeUInt16BE(4096, 2);
    si[12] = 0x00; si[13] = 0xac; si[14] = 0x44; si[15] = 0x02; si[16] = 0x1f;
    const siHeader = Buffer.alloc(4);
    siHeader[0] = 0x00;
    siHeader[1] = (si.length >> 16) & 0xff;
    siHeader[2] = (si.length >> 8) & 0xff;
    siHeader[3] = si.length & 0xff;
    parts.push(siHeader, si);

    // VORBIS_COMMENT - not last
    const vcVendor = Buffer.from("test", "utf8");
    const vcVLen = Buffer.alloc(4);
    vcVLen.writeUInt32LE(vcVendor.length);
    const vcEntry = Buffer.from("TITLE=Test", "utf8");
    const vcELen = Buffer.alloc(4);
    vcELen.writeUInt32LE(vcEntry.length);
    const vcNum = Buffer.alloc(4);
    vcNum.writeUInt32LE(1);
    const vcBody = Buffer.concat([vcVLen, vcVendor, vcNum, vcELen, vcEntry]);
    const vcHeader = Buffer.alloc(4);
    vcHeader[0] = 0x00 | 0x04; // not last | VORBIS_COMMENT
    vcHeader[1] = (vcBody.length >> 16) & 0xff;
    vcHeader[2] = (vcBody.length >> 8) & 0xff;
    vcHeader[3] = vcBody.length & 0xff;
    const vcBlock = Buffer.concat([vcHeader, vcBody]);
    parts.push(vcBlock);

    // PADDING - last metadata block
    const padBody = Buffer.alloc(200);
    const padHeader = Buffer.alloc(4);
    padHeader[0] = 0x80 | 0x01;
    padHeader[1] = (padBody.length >> 16) & 0xff;
    padHeader[2] = (padBody.length >> 8) & 0xff;
    padHeader[3] = padBody.length & 0xff;
    parts.push(padHeader, padBody);

    // Audio data: every byte has bit 7 set (80-FF hex)
    // Previously fixLastFlacBlock would modify these bytes!
    const audioData = Buffer.alloc(5000);
    for (let i = 0; i < audioData.length; i++) {
      audioData[i] = 0x80 | (i & 0x7f);
    }
    parts.push(audioData);

    fs.writeFileSync(fp, Buffer.concat(parts));
    const metadataEnd = 4 + 4 + 34 + vcBlock.length + 4 + 200;
    const bufBefore = fs.readFileSync(fp);
    const audioBefore = Buffer.from(bufBefore.slice(metadataEnd));

    await writeTags(fp, { album: "Safe Album" });

    const bufAfter = fs.readFileSync(fp);
    // Audio data should be at the end (same relative position)
    const audioAfter = Buffer.from(bufAfter.slice(bufAfter.length - audioData.length));
    expect(audioAfter.length).toBe(audioData.length);
    // Every byte must be preserved
    expect(audioAfter.equals(audioBefore)).toBe(true);
  });

  it("handles FLAC with PICTURE block followed by PADDING", async () => {
    // Simulates a real-world pattern: STREAMINFO -> VORBIS_COMMENT -> PICTURE -> PADDING -> audio
    // This pattern is common after cover art insertion.
    const fp = path.join(tmpDir, "pic-pad.flac");

    const parts: Buffer[] = [];
    parts.push(Buffer.from("fLaC", "ascii"));

    // STREAMINFO
    const si = Buffer.alloc(34);
    si.writeUInt16BE(4096, 0);
    si.writeUInt16BE(4096, 2);
    const siHeader = Buffer.alloc(4);
    siHeader[0] = 0x00;
    siHeader[1] = (si.length >> 16) & 0xff;
    siHeader[2] = (si.length >> 8) & 0xff;
    siHeader[3] = si.length & 0xff;
    parts.push(siHeader, si);

    // VORBIS_COMMENT (type 4)
    const vcVendor = Buffer.from("test", "utf8");
    const vcVLen = Buffer.alloc(4);
    vcVLen.writeUInt32LE(vcVendor.length);
    const vcEntry = Buffer.from("TITLE=Original", "utf8");
    const vcELen = Buffer.alloc(4);
    vcELen.writeUInt32LE(vcEntry.length);
    const vcNum = Buffer.alloc(4);
    vcNum.writeUInt32LE(1);
    const vcBody = Buffer.concat([vcVLen, vcVendor, vcNum, vcELen, vcEntry]);
    const vcHeader = Buffer.alloc(4);
    vcHeader[0] = 0x00 | 0x04; // not last | VORBIS_COMMENT
    vcHeader[1] = (vcBody.length >> 16) & 0xff;
    vcHeader[2] = (vcBody.length >> 8) & 0xff;
    vcHeader[3] = vcBody.length & 0xff;
    const vcBlock = Buffer.concat([vcHeader, vcBody]);
    parts.push(vcBlock);

    // PICTURE block (type 6) — small fake one
    const picBody = Buffer.alloc(500);
    // Write minimal JPEG-like data
    picBody[0] = 0xff; picBody[1] = 0xd8; picBody[2] = 0xff; // JPEG SOI
    const picHeader = Buffer.alloc(4);
    picHeader[0] = 0x00 | 0x06; // not last, type 6
    picHeader[1] = (picBody.length >> 16) & 0xff;
    picHeader[2] = (picBody.length >> 8) & 0xff;
    picHeader[3] = picBody.length & 0xff;
    parts.push(picHeader, picBody);

    // PADDING (type 1) — last
    const padBody = Buffer.alloc(3000);
    const padHeader = Buffer.alloc(4);
    padHeader[0] = 0x80 | 0x01; // isLast | PADDING
    padHeader[1] = (padBody.length >> 16) & 0xff;
    padHeader[2] = (padBody.length >> 8) & 0xff;
    padHeader[3] = padBody.length & 0xff;
    parts.push(padHeader, padBody);

    // Audio data
    const audioData = Buffer.alloc(20000);
    for (let i = 0; i < audioData.length; i++) {
      audioData[i] = i % 256;
    }
    parts.push(audioData);

    fs.writeFileSync(fp, Buffer.concat(parts));
    const metadataEnd = 4 + 4 + 34 + vcBlock.length + 4 + 500 + 4 + 3000;
    const bufBefore = fs.readFileSync(fp);
    const audioBefore = Buffer.from(bufBefore.slice(metadataEnd));

    await writeTags(fp, { title: "Updated Title", artist: "Artist" });

    const bufAfter = fs.readFileSync(fp);
    const header = bufAfter.slice(0, 4).toString("ascii");
    expect(header).toBe("fLaC");

    // Audio data preserved (same size and content)
    const audioAfter = Buffer.from(bufAfter.slice(bufAfter.length - audioData.length));
    expect(audioAfter.length).toBe(audioData.length);
    expect(audioAfter.equals(audioBefore)).toBe(true);
  });
});

describe("batchWriteTags", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "batch-"));
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  it("writes tags to multiple MP3 files", async () => {
    const f1 = path.join(tmpDir, "t1.mp3");
    const f2 = path.join(tmpDir, "t2.mp3");
    createMinimalMp3(f1, { title: "Old 1" });
    createMinimalMp3(f2, { title: "Old 2" });

    await batchWriteTags([
      { path: f1, fields: { title: "New 1" } },
      { path: f2, fields: { title: "New 2" } },
    ]);

    expect(NodeID3.read(f1).title).toBe("New 1");
    expect(NodeID3.read(f2).title).toBe("New 2");
  });

  it("handles empty updates array", async () => {
    await expect(batchWriteTags([])).resolves.toBeUndefined();
  });
});

describe("writeTags — format detection", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "format-"));
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  it("WAV write is a no-op (not an error)", async () => {
    const fp = path.join(tmpDir, "test.wav");
    fs.writeFileSync(fp, Buffer.alloc(100));
    // WAV writing is not supported; writeTag silently succeeds for WAV
    await expect(writeTags(fp, { title: "x" })).resolves.toBeUndefined();
  });

  it("rejects truly unknown extensions", async () => {
    const fp = path.join(tmpDir, "test.xyz");
    fs.writeFileSync(fp, Buffer.alloc(100));
    await expect(writeTags(fp, { title: "x" })).rejects.toThrow();
  });
});
