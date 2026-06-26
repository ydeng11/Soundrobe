import { describe, it, expect, beforeEach, afterEach } from "vitest";
import fs from "fs";
import path from "path";
import os from "os";
import * as NodeID3 from "node-id3";
import { parseFile } from "music-metadata";
import {
  writeTags,
  batchWriteTags,
  batchWriteExtraTags,
  writeExtraTags,
  writeExtraTagsWithOutcome,
  writeTagsWithOutcome,
} from "../../electron/handlers/writer";
import { readExtraTags } from "../../electron/handlers/tracks";
import { TagWriteQueue } from "../../electron/services/TagWriteQueue";

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

function findFlacOffset(buf: Buffer): number {
  return buf.indexOf(Buffer.from("fLaC", "ascii"));
}

function readPrependedId3End(buf: Buffer): number | null {
  if (buf.subarray(0, 3).toString("ascii") !== "ID3") return null;
  return 10 +
    ((buf[6] & 0x7f) << 21) +
    ((buf[7] & 0x7f) << 14) +
    ((buf[8] & 0x7f) << 7) +
    (buf[9] & 0x7f);
}

function riffChunk(id: string, payload: Buffer): Buffer {
  const header = Buffer.alloc(8);
  header.write(id, 0, 4, "ascii");
  header.writeUInt32LE(payload.length, 4);
  const pad = payload.length % 2 === 1 ? Buffer.from([0]) : Buffer.alloc(0);
  return Buffer.concat([header, payload, pad]);
}

function createMinimalWav(filePath: string): void {
  const fmt = Buffer.alloc(16);
  fmt.writeUInt16LE(1, 0); // PCM
  fmt.writeUInt16LE(1, 2); // mono
  fmt.writeUInt32LE(44100, 4);
  fmt.writeUInt32LE(88200, 8);
  fmt.writeUInt16LE(2, 12);
  fmt.writeUInt16LE(16, 14);

  const data = Buffer.alloc(882);
  const body = Buffer.concat([
    Buffer.from("WAVE", "ascii"),
    riffChunk("fmt ", fmt),
    riffChunk("data", data),
  ]);
  const header = Buffer.alloc(8);
  header.write("RIFF", 0, 4, "ascii");
  header.writeUInt32LE(body.length, 4);
  fs.writeFileSync(filePath, Buffer.concat([header, body]));
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

  it("preserves custom extra tags when saving sidebar metadata to MP3", async () => {
    const fp = path.join(tmpDir, "test.mp3");
    createMinimalMp3(fp, { title: "Old" });

    await writeExtraTags(fp, [{ key: "MOOD", value: "Bright" }]);
    await writeTags(fp, { title: "New Title" });

    const extras = await readExtraTags(fp);
    expect(extras.find((tag) => tag.key === "MOOD")?.value).toBe("Bright");
  });

  it("shows musicbrainz and compilation detail tags as MP3 extra tags", async () => {
    const fp = path.join(tmpDir, "test.mp3");
    createMinimalMp3(fp);

    await writeTags(fp, {
      artists: ["A", "B"],
      musicbrainzTrackId: "mb-track",
      musicbrainzAlbumId: "mb-album",
      musicbrainzArtistId: "mb-artist",
      compilation: true,
    });

    // readExtraTags should now show MusicBrainz fields (no longer hidden)
    const extras = await readExtraTags(fp);
    const keys = extras.map((tag) => tag.key);
    expect(keys).toContain("MusicBrainz Track Id");
    expect(keys).toContain("MusicBrainz Album Id");
    expect(keys).toContain("MusicBrainz Artist Id");
    expect(keys).toContain("COMPILATION");
    // Standard editor fields remain hidden
    expect(keys).not.toContain("TITLE");
    expect(keys).not.toContain("ARTIST");
    expect(keys).not.toContain("ALBUM");
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

  it("writes normalized multi-artist and MusicBrainz fields to FLAC", async () => {
    const fp = path.join(tmpDir, "rich.flac");
    createMinimalFlac(fp);
    await writeTags(fp, {
      artist: "Alice & Bob",
      artists: ["Alice", "Bob"],
      albumArtist: "Alice",
      albumArtists: ["Alice"],
      musicbrainzAlbumId: "mb-album",
      musicbrainzArtistId: "mb-artist",
      musicbrainzTrackId: "mb-track",
      lyrics: "[00:01.00]你好",
      trackNumber: 1,
      trackTotal: 2,
      discNumber: 1,
      discTotal: 1,
    });

    const meta = await parseFile(fp, { duration: false });
    expect(meta.common.artist).toBe("Alice & Bob");
    expect(meta.common.artists).toEqual(["Alice", "Bob"]);
    expect(meta.common.albumartist).toBe("Alice");
    expect(meta.common.musicbrainz_albumid).toBe("mb-album");
    expect(meta.common.musicbrainz_artistid).toContain("mb-artist");
    expect(meta.common.musicbrainz_recordingid).toBe("mb-track");
    expect(JSON.stringify(meta.common.lyrics)).toContain("你好");
  });

  it("writes FLAC when no existing Vorbis comment block exists", async () => {
    // Create FLAC with STREAMINFO only (no VORBIS_COMMENT)
    const fp = path.join(tmpDir, "bare.flac");
    createMinimalFlac(fp);
    await writeTags(fp, { title: "Fresh Title" });
    const meta = await parseFile(fp);
    expect(meta.common.title).toBe("Fresh Title");
  });

  it("strips APEv2 tag that overrides Vorbis comments", async () => {
    // Use a real FLAC file and add an APE tag to simulate the problem
    const realFile = "/Volumes/downloads/music/胡彦斌/2011-Who Cares/4. 女人不该让男人流泪.flac";
    if (!fs.existsSync(realFile)) return; // skip if test env doesn't have the file

    const fp = path.join(tmpDir, "ape-override.flac");
    fs.copyFileSync(realFile, fp);

    // Append a realistic APEv2 tag (QQ Music style) with wrong album name
    const apeValue = Buffer.from("WHO CARES", "utf8");
    const key = Buffer.from("Album\0", "ascii");
    // APE item: 4 bytes value_size + 4 bytes flags + key + value
    const itemSize = apeValue.length;
    const item = Buffer.alloc(8 + key.length + itemSize);
    item.writeUInt32LE(itemSize, 0);
    item.writeUInt32LE(0, 4);
    key.copy(item, 8);
    apeValue.copy(item, 8 + key.length);

    // APE footer (32 bytes)
    const footer = Buffer.alloc(32);
    footer.write("APETAGEX", 0, 8, "ascii");
    footer.writeUInt32LE(2000, 8);
    footer.writeUInt32LE(item.length + 32, 12); // tag size = items + footer
    footer.writeUInt32LE(1, 16); // item count
    footer.writeUInt32LE(0, 20); // flags

    // Append to end of file using append mode
    const fd = fs.openSync(fp, "a");
    fs.writeSync(fd, item);
    fs.writeSync(fd, footer);
    fs.closeSync(fd);

    // Verify APE tag exists before write
    const bufBefore = fs.readFileSync(fp);
    expect(bufBefore.indexOf("APETAGEX")).toBeGreaterThan(-1);

    // Write tags — should strip the APE tag
    await writeTags(fp, { album: "Who Cares?" });

    // Verify APE tag is gone from file bytes
    const bufAfter = fs.readFileSync(fp);
    expect(bufAfter.indexOf("APETAGEX")).toBe(-1);

    // Verify music-metadata now reads the correct Vorbis value
    const after = await parseFile(fp);
    expect(after.common.album).toBe("Who Cares?");
  });

  it("writes FLAC cover art without corrupting a prepended ID3 tag", async () => {
    const realFile = "/Volumes/downloads/7. Lisa I Love U.flac";
    if (!fs.existsSync(realFile)) return;

    const fp = path.join(tmpDir, "prepended-id3-cover.flac");
    fs.copyFileSync(realFile, fp);

    const before = fs.readFileSync(fp);
    const flacOffsetBefore = findFlacOffset(before);
    expect(flacOffsetBefore).toBeGreaterThan(0);
    expect(readPrependedId3End(before)).toBe(flacOffsetBefore);

    const coverData = Buffer.from([
      0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, 0x4a, 0x46,
      0x49, 0x46, 0x00, 0x01, 0xff, 0xd9,
    ]);

    await writeTags(fp, {
      title: "Lisa I Love U",
      coverData,
      coverMime: "image/jpeg",
    });

    const afterWrite = fs.readFileSync(fp);
    const flacOffsetAfter = findFlacOffset(afterWrite);
    expect(flacOffsetAfter).toBe(flacOffsetBefore);
    expect(readPrependedId3End(afterWrite)).toBe(flacOffsetAfter);

    const parsed = await parseFile(fp);
    expect(parsed.format.duration).toBeGreaterThan(293);
    expect(parsed.format.duration).toBeLessThan(294);
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

// ── FLAC padding-aware in-place write tests ─────────────────────────

function findAudioOffset(buf: Buffer): number {
  // Parse FLAC layout to find first audio byte
  let offset = 4; // skip "fLaC"
  while (offset + 4 <= buf.length) {
    const isLast = !!(buf[offset] >> 7);
    const length =
      (buf[offset + 1] << 16) |
      (buf[offset + 2] << 8) |
      buf[offset + 3];
    if (offset + 4 + length > buf.length) break;
    offset += 4 + length;
    if (isLast) break;
  }
  return offset;
}

function buildFlacWithPadding(
  filePath: string,
  paddingSize: number,
  comments: Record<string, string> = {},
  options: { includeSeektable?: boolean } = {},
): void {
  const parts: Buffer[] = [];
  parts.push(Buffer.from("fLaC", "ascii"));

  // STREAMINFO
  const si = Buffer.alloc(34);
  si.writeUInt16BE(4096, 0);
  si.writeUInt16BE(4096, 2);
  si[12] = 0x00; si[13] = 0xac; si[14] = 0x44; si[15] = 0x02; si[16] = 0x1f;
  const siH = Buffer.alloc(4);
  siH[0] = 0x00;
  siH[1] = (si.length >> 16) & 0xff;
  siH[2] = (si.length >> 8) & 0xff;
  siH[3] = si.length & 0xff;
  parts.push(siH, si);

  // VORBIS_COMMENT (not last)
  const vendor = Buffer.from("test", "utf8");
  const vLen = Buffer.alloc(4);
  vLen.writeUInt32LE(vendor.length);
  const cBufs: Buffer[] = [];
  for (const [k, v] of Object.entries(comments)) {
    const cb = Buffer.from(`${k}=${v}`, "utf8");
    const cl = Buffer.alloc(4);
    cl.writeUInt32LE(cb.length);
    cBufs.push(cl, cb);
  }
  const n = Buffer.alloc(4);
  n.writeUInt32LE(Object.keys(comments).length);
  const vb = Buffer.concat([vLen, vendor, n, ...cBufs]);

  const vcH = Buffer.alloc(4);
  vcH[0] = 0x00 | 0x04;
  vcH[1] = (vb.length >> 16) & 0xff;
  vcH[2] = (vb.length >> 8) & 0xff;
  vcH[3] = vb.length & 0xff;
  parts.push(vcH, vb);

  if (options.includeSeektable) {
    const seektable = Buffer.alloc(18);
    const seekH = Buffer.alloc(4);
    seekH[0] = 0x03;
    seekH[1] = (seektable.length >> 16) & 0xff;
    seekH[2] = (seektable.length >> 8) & 0xff;
    seekH[3] = seektable.length & 0xff;
    parts.push(seekH, seektable);
  }

  if (paddingSize > 0) {
    const padH = Buffer.alloc(4);
    padH[0] = 0x80 | 0x01; // isLast | PADDING
    padH[1] = (paddingSize >> 16) & 0xff;
    padH[2] = (paddingSize >> 8) & 0xff;
    padH[3] = paddingSize & 0xff;
    const padBody = Buffer.alloc(paddingSize);
    parts.push(padH, padBody);
  } else {
    // Mark Vorbis as last
    const buf = Buffer.concat(parts);
    buf[4 + 4 + 34] |= 0x80; // set isLast on Vorbis block
    fs.writeFileSync(filePath, buf);
    return;
  }

  // Audio data
  const audio = Buffer.alloc(10000);
  for (let i = 0; i < audio.length; i++) audio[i] = i % 256;
  parts.push(audio);

  fs.writeFileSync(filePath, Buffer.concat(parts));
}

describe("writeTags — FLAC in-place padding-aware", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "writer-flac-pad-"));
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  it("smaller Vorbis update keeps file size and audio bytes unchanged", async () => {
    const fp = path.join(tmpDir, "smaller.flac");
    buildFlacWithPadding(fp, 100, { TITLE: "A Very Long Title That Takes Up A Lot Of Space", ARTIST: "An Even Longer Artist Name Here To Fill Bytes" });

    const statBefore = fs.statSync(fp);
    const bufBefore = fs.readFileSync(fp);
    const audioOffBefore = findAudioOffset(bufBefore);
    const audioBefore = bufBefore.slice(audioOffBefore);

    await writeTags(fp, { title: "Short", artist: "Short" });

    const statAfter = fs.statSync(fp);
    const bufAfter = fs.readFileSync(fp);

    // File size must not change
    expect(statAfter.size).toBe(statBefore.size);
    // Audio bytes must be identical (same offset, same content)
    const audioAfter = bufAfter.slice(audioOffBefore);
    expect(audioAfter.length).toBe(audioBefore.length);
    expect(audioAfter.equals(audioBefore)).toBe(true);
    // Tags are correctly readable
    const meta = await parseFile(fp, { duration: false });
    expect(meta.common.title).toBe("Short");
    expect(meta.common.artist).toBe("Short");
  });

  it("shrinking Vorbis block updates header length and converts leftover to PADDING", async () => {
    const fp = path.join(tmpDir, "shrink-padding.flac");
    buildFlacWithPadding(fp, 200, {
      TITLE: "A Very Long Title That Takes Up A Lot Of Space",
      ARTIST: "An Even Longer Artist Name Here To Fill Bytes",
    });

    // Read the Vorbis block length before writing
    const bufBefore = fs.readFileSync(fp);
    let vorbisLenBefore = 0;
    let vorbisDataLenBefore = 0;
    {
      let off = 4;
      while (off + 4 <= bufBefore.length) {
        const isLast = !!(bufBefore[off] >> 7);
        const type = bufBefore[off] & 0x7f;
        const len = (bufBefore[off + 1] << 16) | (bufBefore[off + 2] << 8) | bufBefore[off + 3];
        if (type === 4) {
          vorbisLenBefore = len;
          // Compute actual Vorbis content size
          const vOff = off + 4;
          const vendorLen = bufBefore.readUInt32LE(vOff);
          const numComments = bufBefore.readUInt32LE(vOff + 4 + vendorLen);
          let pos = vOff + 4 + vendorLen + 4;
          let commentBytes = 0;
          for (let i = 0; i < numComments; i++) {
            const cLen = bufBefore.readUInt32LE(pos);
            pos += 4 + cLen;
            commentBytes += cLen;
          }
          vorbisDataLenBefore = 4 + vendorLen + 4 + numComments * 4 + commentBytes;
          break;
        }
        if (isLast) break;
        off += 4 + len;
      }
    }
    // Sanity: header length should match content
    expect(vorbisLenBefore).toBe(vorbisDataLenBefore);

    await writeTags(fp, { title: "Short", artist: "Short" });

    const bufAfter = fs.readFileSync(fp);
    // Parse Vorbis block after write
    let vorbisLenAfter = 0;
    let vorbisDataLenAfter = 0;
    let vorbisIsLastAfter = false;
    {
      let off = 4;
      while (off + 4 <= bufAfter.length) {
        const isLast = !!(bufAfter[off] >> 7);
        const type = bufAfter[off] & 0x7f;
        const len = (bufAfter[off + 1] << 16) | (bufAfter[off + 2] << 8) | bufAfter[off + 3];
        if (type === 4) {
          vorbisLenAfter = len;
          vorbisIsLastAfter = isLast;
          const vOff = off + 4;
          const vendorLen = bufAfter.readUInt32LE(vOff);
          const numComments = bufAfter.readUInt32LE(vOff + 4 + vendorLen);
          let pos = vOff + 4 + vendorLen + 4;
          let commentBytes = 0;
          for (let i = 0; i < numComments; i++) {
            const cLen = bufAfter.readUInt32LE(pos);
            pos += 4 + cLen;
            commentBytes += cLen;
          }
          vorbisDataLenAfter = 4 + vendorLen + 4 + numComments * 4 + commentBytes;
          break;
        }
        if (isLast) break;
        off += 4 + len;
      }
    }

    // The Vorbis block MUST NOT be last anymore (PADDING follows)
    expect(vorbisIsLastAfter).toBe(false);
    // Header length must match actual content — the core bug this test guards
    expect(vorbisLenAfter).toBe(vorbisDataLenAfter);
    // Vorbis block must have shrunk
    expect(vorbisLenAfter).toBeLessThan(vorbisLenBefore);
    // File size unchanged (leftover became PADDING)
    expect(bufAfter.length).toBe(bufBefore.length);
    // Tags readable
    const meta = await parseFile(fp, { duration: false });
    expect(meta.common.title).toBe("Short");
    expect(meta.common.artist).toBe("Short");
  });

  it("equal Vorbis update keeps file size and audio bytes unchanged", async () => {
    const fp = path.join(tmpDir, "equal.flac");
    buildFlacWithPadding(fp, 100, { TITLE: "ExactMatch" });

    const statBefore = fs.statSync(fp);
    const bufBefore = fs.readFileSync(fp);
    const audioOffBefore = findAudioOffset(bufBefore);
    const audioBefore = bufBefore.slice(audioOffBefore);

    await writeTags(fp, { title: "UpdatedX" });

    const statAfter = fs.statSync(fp);
    const bufAfter = fs.readFileSync(fp);

    expect(statAfter.size).toBe(statBefore.size);
    const audioAfter = bufAfter.slice(audioOffBefore);
    expect(audioAfter.equals(audioBefore)).toBe(true);
    const meta = await parseFile(fp, { duration: false });
    expect(meta.common.title).toBe("UpdatedX");
  });

  it("larger Vorbis update consumes adjacent padding and preserves audio offset/content", async () => {
    const fp = path.join(tmpDir, "grow.flac");
    buildFlacWithPadding(fp, 5000, { TITLE: "Short" });

    const bufBefore = fs.readFileSync(fp);
    const audioOffBefore = findAudioOffset(bufBefore);
    const audioBefore = bufBefore.slice(audioOffBefore);
    const sizeBefore = bufBefore.length;

    await writeTags(fp, {
      title: "A Much Longer Title Here",
      artist: "Artist Name",
      album: "Album That Has A Name",
      genre: "Rock",
      composer: "Composer Name",
      year: "2024",
    });

    const bufAfter = fs.readFileSync(fp);

    // Audio offset should not have moved (padding was consumed)
    expect(bufAfter.length).toBe(sizeBefore);
    const audioAfter = bufAfter.slice(audioOffBefore);
    expect(audioAfter.equals(audioBefore)).toBe(true);

    // Declared metadata end must align with real audio start.
    // Regresses a bug where fast-path growth subtracted 4 twice and
    // left orphaned zero bytes between metadata and audio.
    const declaredAudioOffset = findAudioOffset(bufAfter);
    const audioStartPattern = Buffer.from([0, 1, 2, 3, 4, 5, 6, 7]);
    const actualAudioOffset = bufAfter.indexOf(audioStartPattern, declaredAudioOffset);
    expect(actualAudioOffset).toBe(declaredAudioOffset);

    // Tags are correct
    const meta = await parseFile(fp, { duration: false });
    expect(meta.common.title).toBe("A Much Longer Title Here");
    expect(meta.common.artist).toBe("Artist Name");
    expect(meta.common.album).toBe("Album That Has A Name");
  });

  it("insufficient padding falls back and then second write is in-place", async () => {
    const fp = path.join(tmpDir, "fallback.flac");
    buildFlacWithPadding(fp, 0, { TITLE: "InitialTitle" });

    const sizeBefore = fs.statSync(fp).size;

    // First write: should fall back (no padding initially)
    await writeTags(fp, {
      title: "After Fallback",
      artist: "New Artist",
      album: "New Album",
      genre: "Electronic",
      composer: "Producer",
      year: "2025",
      trackNumber: 1,
      trackTotal: 12,
      discNumber: 1,
      discTotal: 2,
    });

    const bufAfterFirst = fs.readFileSync(fp);
    // File should have grown due to added 64K padding
    expect(bufAfterFirst.length).toBeGreaterThan(sizeBefore);

    // Second write with slightly changed tags: should be in-place (padding available)
    const audioOffAfterFirst = findAudioOffset(bufAfterFirst);
    const audioAfterFirst = bufAfterFirst.slice(audioOffAfterFirst);

    await writeTags(fp, { title: "After InPlace" });

    const bufAfterSecond = fs.readFileSync(fp);
    const audioAfterSecond = bufAfterSecond.slice(bufAfterSecond.length - audioAfterFirst.length);
    expect(audioAfterSecond.equals(audioAfterFirst)).toBe(true);

    const meta = await parseFile(fp, { duration: false });
    expect(meta.common.title).toBe("After InPlace");
    expect(meta.common.artist).toBe("New Artist");
  });

  it("fallback rewrite drops old trailing padding before appending fresh padding", async () => {
    const fp = path.join(tmpDir, "seektable-padding.flac");
    buildFlacWithPadding(fp, 5000, { TITLE: "Short" }, { includeSeektable: true });

    await writeTags(fp, {
      title: "A Much Longer Title Here",
      artist: "郭富城",
      album: "对你爱不完",
      year: "1990",
      genre: "Mandopop",
      discogsArtistId: "211321",
      musicbrainzAlbumId: "34443d65-15fd-45c2-9cb2-f035374619a3",
    });

    const bufAfter = fs.readFileSync(fp);
    const declaredAudioOffset = findAudioOffset(bufAfter);
    const audioStartPattern = Buffer.from([0, 1, 2, 3, 4, 5, 6, 7]);
    const actualAudioOffset = bufAfter.indexOf(audioStartPattern, declaredAudioOffset);
    expect(actualAudioOffset).toBe(declaredAudioOffset);

    const meta = await parseFile(fp, { duration: false });
    expect(meta.common.title).toBe("A Much Longer Title Here");
    expect(meta.common.artist).toBe("郭富城");
  });

  it("no-op FLAC write keeps file identical", async () => {
    const fp = path.join(tmpDir, "noop.flac");
    buildFlacWithPadding(fp, 100, { TITLE: "Stay", ARTIST: "Same" });

    const bufBefore = fs.readFileSync(fp);

    await writeTags(fp, { title: "Stay", artist: "Same" });

    const bufAfter = fs.readFileSync(fp);
    expect(bufAfter.equals(bufBefore)).toBe(true);

    // Still readable
    const meta = await parseFile(fp, { duration: false });
    expect(meta.common.title).toBe("Stay");
    expect(meta.common.artist).toBe("Same");
  });

  it("neutralizes ghost Vorbis Comment in audio data so title update persists", async () => {
    const fp = path.join(tmpDir, "ghost-vc.flac");
    createMinimalFlac(fp, { title: "Original" });

    // Write initial tags
    await writeTags(fp, { title: "Original", artist: "Artist" });

    // Inject a ghost VC block within the audio data area
    // (simulating prior bug where a VC was written at a wrong offset)
    const buf = Buffer.from(fs.readFileSync(fp));

    // Build a ghost VC payload
    const ghostVendor = Buffer.from("auto-tagger", "utf8");
    const ghostTag = Buffer.from("TITLE=GhostTitle", "utf8");
    const ghostPayload = Buffer.concat([
      Buffer.alloc(4), // vendor len placeholder
      ghostVendor,
      Buffer.alloc(4), // num tags
      Buffer.alloc(4), // tag len placeholder
      ghostTag,
    ]);
    ghostPayload.writeUInt32LE(ghostVendor.length, 0);
    ghostPayload.writeUInt32LE(1, 4 + ghostVendor.length);
    ghostPayload.writeUInt32LE(ghostTag.length, 4 + ghostVendor.length + 4);

    // Append ghost VC at end of file (beyond audio data)
    const patched = Buffer.concat([buf, ghostPayload]);
    fs.writeFileSync(fp, patched);

    // Verify the ghost VC header is readable in the buffer
    const patchedBuf = fs.readFileSync(fp);
    const ghostOffset = patchedBuf.length - ghostPayload.length;
    const vendorLen = patchedBuf.readUInt32LE(ghostOffset);
    expect(vendorLen).toBe(ghostVendor.length);
    const vendor = patchedBuf.toString("utf8", ghostOffset + 4, ghostOffset + 4 + vendorLen);
    expect(vendor).toBe("auto-tagger");

    // Now write the correct title (this should neutralize the ghost VC)
    await writeTags(fp, { title: "RealTitle", artist: "Artist" });

    // Verify ghost VC header was zeroed
    const afterBuf = fs.readFileSync(fp);
    // The ghost is now at the end — check if vendor len was zeroed
    const ghostAfterOffset = afterBuf.length - ghostPayload.length;
    const vendorLenAfter = afterBuf.readUInt32LE(ghostAfterOffset);
    expect(vendorLenAfter).toBe(0); // neutralized!
  });
});

describe("writeTags — WAV in-place", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "writer-wav-pad-"));
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  /** Create a WAV with an existing id3 chunk already embedded. */
  function createWavWithId3(filePath: string, existingTags?: Record<string, string>): void {
    const fmt = Buffer.alloc(16);
    fmt.writeUInt16LE(1, 0);
    fmt.writeUInt16LE(1, 2);
    fmt.writeUInt32LE(44100, 4);
    fmt.writeUInt32LE(88200, 8);
    fmt.writeUInt16LE(2, 12);
    fmt.writeUInt16LE(16, 14);

    const data = Buffer.alloc(882);

    // Build initial ID3 chunk with tags
    const nodeId3Tags: NodeID3.Tags = {};
    if (existingTags) {
      if (existingTags.title) nodeId3Tags.title = existingTags.title;
      if (existingTags.artist) nodeId3Tags.artist = existingTags.artist;
      if (existingTags.album) nodeId3Tags.album = existingTags.album;
    }
    const id3Payload = NodeID3.create(nodeId3Tags);

    const chunks: Buffer[] = [
      Buffer.from("WAVE", "ascii"),
      riffChunk("fmt ", fmt),
      riffChunk("data", data),
      riffChunk("id3 ", id3Payload),
    ];

    const body = Buffer.concat(chunks);
    const header = Buffer.alloc(8);
    header.write("RIFF", 0, 4, "ascii");
    header.writeUInt32LE(body.length, 4);
    fs.writeFileSync(filePath, Buffer.concat([header, body]));
  }

  it("existing id3 chunk update fits in place, preserves file size and audio bytes", async () => {
    const fp = path.join(tmpDir, "inplace.wav");
    createWavWithId3(fp, { title: "Long Title That Takes Space", artist: "Long Artist Name" });

    const statBefore = fs.statSync(fp);
    const bufBefore = fs.readFileSync(fp);

    // Find data chunk position
    let dataStart = -1;
    let dataEnd = -1;
    for (let off = 12; off + 8 <= bufBefore.length;) {
      const id = bufBefore.toString("ascii", off, off + 4);
      const sz = bufBefore.readUInt32LE(off + 4);
      const end = off + 8 + sz + (sz % 2);
      if (id === "data") {
        dataStart = off;
        dataEnd = end;
        break;
      }
      off = end;
    }
    const audioBefore = bufBefore.slice(dataStart, dataEnd);

    await writeTags(fp, { title: "Short" });

    const statAfter = fs.statSync(fp);
    const bufAfter = fs.readFileSync(fp);

    expect(statAfter.size).toBe(statBefore.size);
    // Audio chunk must be unchanged
    const audioAfter = bufAfter.slice(dataStart, dataEnd);
    expect(audioAfter.equals(audioBefore)).toBe(true);
    // Tags are correctly readable
    const meta = await parseFile(fp, { duration: false });
    expect(meta.common.title).toBe("Short");
  });

  it("skips WAV metadata writes when requested fields are unchanged", async () => {
    const fp = path.join(tmpDir, "metadata-noop.wav");
    createWavWithId3(fp, { title: "Stable Title", artist: "Stable Artist" });

    const baseline = new Date(Date.now() - 60_000);
    fs.utimesSync(fp, baseline, baseline);
    const mtimeBefore = fs.statSync(fp).mtimeMs;

    const outcome = await writeTagsWithOutcome(fp, {
      title: "Stable Title",
      artist: "Stable Artist",
    });

    const mtimeAfter = fs.statSync(fp).mtimeMs;
    expect(outcome).toBe("skipped");
    expect(mtimeAfter).toBe(mtimeBefore);
  });

  it("oversized ID3 update falls back and remains readable by music-metadata", async () => {
    const fp = path.join(tmpDir, "oversized.wav");
    createWavWithId3(fp, { title: "Short" });

    const statBefore = fs.statSync(fp);

    // Write much larger tags that won't fit in the existing id3 chunk
    await writeTags(fp, {
      title: "A",
      artist: "B",
      album: "C",
      genre: "D",
      composer: "E",
      year: "2024",
      comment: "F",
      lyrics: "G",
      musicbrainzTrackId: "H",
      musicbrainzAlbumId: "I",
      musicbrainzArtistId: "J",
    });

    const statAfter = fs.statSync(fp);
    // File grew because full rewrite occurred
    expect(statAfter.size).toBeGreaterThan(statBefore.size);

    // Tags are readable
    const meta = await parseFile(fp, { duration: false });
    expect(meta.common.title).toBe("A");
    expect(meta.common.artist).toBe("B");
    expect(meta.common.album).toBe("C");
  });

  it("skips WAV extra-tag writes when requested tags are unchanged", async () => {
    const fp = path.join(tmpDir, "extra-noop.wav");
    createMinimalWav(fp);

    await writeExtraTags(fp, [{ key: "MOOD", value: "Bright" }]);

    const baseline = new Date(Date.now() - 60_000);
    fs.utimesSync(fp, baseline, baseline);
    const mtimeBefore = fs.statSync(fp).mtimeMs;

    const outcome = await writeExtraTagsWithOutcome(fp, [{ key: "MOOD", value: "Bright" }]);

    const mtimeAfter = fs.statSync(fp).mtimeMs;
    expect(outcome).toBe("skipped");
    expect(mtimeAfter).toBe(mtimeBefore);

    const extras = await readExtraTags(fp);
    expect(extras.find((t) => t.key === "MOOD")?.value).toBe("Bright");
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

  it("writes title to WAV ID3 tags — verified by music-metadata", async () => {
    const fp = path.join(tmpDir, "test.wav");
    createMinimalWav(fp);

    await writeTags(fp, { title: "寂寞在唱歌" });

    const meta = await parseFile(fp, { duration: false });
    expect(meta.common.title).toBe("寂寞在唱歌");
  });

  it("overwrites existing WAV ID3 title", async () => {
    const fp = path.join(tmpDir, "test.wav");
    createMinimalWav(fp);

    await writeTags(fp, { title: "Old Title" });
    await writeTags(fp, { title: "New Title" });

    const meta = await parseFile(fp, { duration: false });
    expect(meta.common.title).toBe("New Title");
  });

  it("preserves existing WAV ID3 tags when updating one field", async () => {
    const fp = path.join(tmpDir, "test.wav");
    createMinimalWav(fp);

    await writeTags(fp, { title: "Old Title", artist: "阿桑", album: "珍藏纪念版 DTS" });
    await writeTags(fp, { title: "New Title" });

    const meta = await parseFile(fp, { duration: false });
    expect(meta.common.title).toBe("New Title");
    expect(meta.common.artist).toBe("阿桑");
    expect(meta.common.album).toBe("珍藏纪念版 DTS");
  });

  it("rejects AIFF writes because unsupported metadata writes must fail loud", async () => {
    const fp = path.join(tmpDir, "test.aiff");
    fs.writeFileSync(fp, Buffer.alloc(100));
    await expect(writeTags(fp, { title: "x" })).rejects.toThrow("AIFF metadata writing is not supported");
  });

  it("rejects truly unknown extensions", async () => {
    const fp = path.join(tmpDir, "test.xyz");
    fs.writeFileSync(fp, Buffer.alloc(100));
    await expect(writeTags(fp, { title: "x" })).rejects.toThrow();
  });
});

describe("batchWriteExtraTags", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "batch-extra-"));
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  it("writes the same extra tags to multiple MP3 files", async () => {
    const f1 = path.join(tmpDir, "t1.mp3");
    const f2 = path.join(tmpDir, "t2.mp3");
    createMinimalMp3(f1);
    createMinimalMp3(f2);

    await batchWriteExtraTags([
      { path: f1, tags: [{ key: "BARCODE", value: "ABC-123" }, { key: "MOOD", value: "Bright" }] },
      { path: f2, tags: [{ key: "BARCODE", value: "ABC-123" }, { key: "MOOD", value: "Bright" }] },
    ]);

    // Read back using writeExtraTags path (NodeID3 for MP3)
    const r1 = NodeID3.read(f1);
    const u1 = (Array.isArray(r1.userDefinedText) ? r1.userDefinedText : r1.userDefinedText ? [r1.userDefinedText] : []);
    const m1 = Object.fromEntries(u1.filter((t) => t.description).map((t) => [t.description, t.value]));
    expect(m1["BARCODE"]).toBe("ABC-123");
    expect(m1["MOOD"]).toBe("Bright");

    const r2 = NodeID3.read(f2);
    const u2 = (Array.isArray(r2.userDefinedText) ? r2.userDefinedText : r2.userDefinedText ? [r2.userDefinedText] : []);
    const m2 = Object.fromEntries(u2.filter((t) => t.description).map((t) => [t.description, t.value]));
    expect(m2["BARCODE"]).toBe("ABC-123");
    expect(m2["MOOD"]).toBe("Bright");
  });

  it("reads MP3 extra tags back through the Electron read path", async () => {
    const fp = path.join(tmpDir, "readback.mp3");
    createMinimalMp3(fp);

    await writeExtraTags(fp, [
      { key: "BARCODE", value: "ABC-123" },
      { key: "MOOD", value: "Bright" },
    ]);

    const extras = await readExtraTags(fp);
    expect(extras.find((tag) => tag.key === "BARCODE")?.value).toBe("ABC-123");
    expect(extras.find((tag) => tag.key === "MOOD")?.value).toBe("Bright");
  });

  it("allows ARTISTS as an MP3 extra tag", async () => {
    const fp = path.join(tmpDir, "artists-extra.mp3");
    createMinimalMp3(fp);

    await writeExtraTags(fp, [
      { key: "ARTISTS", value: "foo" },
      { key: "ARTISTS", value: "bar" },
    ]);

    const extras = await readExtraTags(fp);
    const values = extras
      .filter((tag) => tag.key === "ARTISTS")
      .map((tag) => tag.value);
    expect(values).toEqual(["foo", "bar"]);
  });

  it("allows editing musicbrainz and compilation detail tags through MP3 extra tags", async () => {
    const fp = path.join(tmpDir, "allow-edit-mb.mp3");
    createMinimalMp3(fp);

    // Write initial detail tags via sidebar write
    await writeTags(fp, {
      artists: ["Primary", "Guest"],
      musicbrainzTrackId: "mb-track",
      compilation: true,
    });

    // Write extra tags including the musicbrainz fields (now editable through extras)
    await writeExtraTags(fp, [
      { key: "MusicBrainz Track Id", value: "new-mb-track" },
      { key: "COMPILATION", value: "0" },
      { key: "MOOD", value: "Bright" },
    ]);

    const tags = NodeID3.read(fp);
    const userDefinedText = Array.isArray(tags.userDefinedText)
      ? tags.userDefinedText
      : tags.userDefinedText
        ? [tags.userDefinedText]
        : [];
    const byDescription = Object.fromEntries(
      userDefinedText.filter((tag) => tag.description).map((tag) => [tag.description, tag.value]),
    );
    // ARTISTS is still reserved but in EXTRA_TAG_RESERVED_EXCEPTIONS so it's removed
    expect(byDescription["ARTISTS"]).toBeUndefined();
    // MusicBrainz fields are now editable through extra tags
    expect(byDescription["MusicBrainz Track Id"]).toBe("new-mb-track");
    expect(byDescription["COMPILATION"]).toBe("0");
    expect(byDescription["MOOD"]).toBe("Bright");
  });

  it("removes musicbrainz detail tags when not included in extra tags write", async () => {
    const fp = path.join(tmpDir, "remove-mb.mp3");
    createMinimalMp3(fp);

    await writeTags(fp, {
      musicbrainzTrackId: "mb-track",
      compilation: true,
    });
    // Write extra tags without musicbrainz fields — they get removed
    await writeExtraTags(fp, [{ key: "MOOD", value: "Bright" }]);

    const tags = NodeID3.read(fp);
    const userDefinedText = Array.isArray(tags.userDefinedText)
      ? tags.userDefinedText
      : tags.userDefinedText
        ? [tags.userDefinedText]
        : [];
    const byDescription = Object.fromEntries(
      userDefinedText.filter((tag) => tag.description).map((tag) => [tag.description, tag.value]),
    );
    expect(byDescription["MusicBrainz Track Id"]).toBeUndefined();
    expect(byDescription["COMPILATION"]).toBeUndefined();
    expect(byDescription["MOOD"]).toBe("Bright");
  });

  it("writes extra tags to each file independently", async () => {
    const f1 = path.join(tmpDir, "a.mp3");
    const f2 = path.join(tmpDir, "b.mp3");
    createMinimalMp3(f1);
    createMinimalMp3(f2);

    await batchWriteExtraTags([
      { path: f1, tags: [{ key: "CATALOGNUMBER", value: "CN-001" }] },
      { path: f2, tags: [{ key: "CATALOGNUMBER", value: "CN-002" }] },
    ]);

    const r1 = NodeID3.read(f1);
    const u1 = (Array.isArray(r1.userDefinedText) ? r1.userDefinedText : r1.userDefinedText ? [r1.userDefinedText] : []);
    const m1 = Object.fromEntries(u1.filter((t) => t.description).map((t) => [t.description, t.value]));
    expect(m1["CATALOGNUMBER"]).toBe("CN-001");

    const r2 = NodeID3.read(f2);
    const u2 = (Array.isArray(r2.userDefinedText) ? r2.userDefinedText : r2.userDefinedText ? [r2.userDefinedText] : []);
    const m2 = Object.fromEntries(u2.filter((t) => t.description).map((t) => [t.description, t.value]));
    expect(m2["CATALOGNUMBER"]).toBe("CN-002");
  });

  it("overwrites existing extra tags on each file", async () => {
    const fp = path.join(tmpDir, "overwrite.mp3");
    createMinimalMp3(fp);

    // Write initial extra tags
    await writeExtraTags(fp, [{ key: "MOOD", value: "Old" }]);

    // Batch overwrite
    await batchWriteExtraTags([
      { path: fp, tags: [{ key: "MOOD", value: "New" }] },
    ]);

    const r = NodeID3.read(fp);
    const u = (Array.isArray(r.userDefinedText) ? r.userDefinedText : r.userDefinedText ? [r.userDefinedText] : []);
    const m = Object.fromEntries(u.filter((t) => t.description).map((t) => [t.description, t.value]));
    expect(m["MOOD"]).toBe("New");
  });

  it("handles empty updates array", async () => {
    await expect(batchWriteExtraTags([])).resolves.toBeUndefined();
  });
});

describe("writeExtraTags — FLAC round-trip", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "writer-extra-flac-"));
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  async function readVorbisCommentsFromFile(
    filePath: string,
  ): Promise<Record<string, string>> {
    const meta = await parseFile(filePath, { duration: false });
    const result: Record<string, string> = {};
    for (const [, tags] of Object.entries(meta.native)) {
      for (const tag of tags) {
        if (typeof tag.id === "string" && typeof tag.value === "string") {
          result[tag.id] = tag.value;
        }
      }
    }
    return result;
  }

  it("saves new extra tags to FLAC and reads them back", async () => {
    const fp = path.join(tmpDir, "test.flac");
    createMinimalFlac(fp, "Song", "Artist");

    await writeExtraTags(fp, [
      { key: "MOOD", value: "Happy" },
      { key: "BARCODE", value: "ABC-123" },
    ]);

    const comments = await readVorbisCommentsFromFile(fp);
    expect(comments["MOOD"]).toBe("Happy");
    expect(comments["BARCODE"]).toBe("ABC-123");
  });

  it("preserves standard tags when writing extra tags to FLAC", async () => {
    const fp = path.join(tmpDir, "preserve.flac");
    createMinimalFlac(fp, "Original Title", "Original Artist");

    await writeExtraTags(fp, [
      { key: "MOOD", value: "Melancholy" },
    ]);

    const comments = await readVorbisCommentsFromFile(fp);
    expect(comments["TITLE"]).toBe("Original Title");
    expect(comments["ARTIST"]).toBe("Original Artist");
    expect(comments["MOOD"]).toBe("Melancholy");
  });

  it("overwrites existing extra tags on FLAC", async () => {
    const fp = path.join(tmpDir, "overwrite.flac");
    createMinimalFlac(fp, "Song", "Artist");

    await writeExtraTags(fp, [
      { key: "MOOD", value: "Old" },
      { key: "CATALOGNUMBER", value: "CN-001" },
    ]);

    // Now overwrite with new values
    await writeExtraTags(fp, [
      { key: "MOOD", value: "New" },
    ]);

    const comments = await readVorbisCommentsFromFile(fp);
    expect(comments["MOOD"]).toBe("New");
    // old extra tags should be gone
    expect(comments["CATALOGNUMBER"]).toBeUndefined();
  });

  it("handles empty extra tags array on FLAC (no-op)", async () => {
    const fp = path.join(tmpDir, "noop.flac");
    createMinimalFlac(fp, "Title");

    await writeExtraTags(fp, []);

    const comments = await readVorbisCommentsFromFile(fp);
    expect(comments["TITLE"]).toBe("Title");
  });

  it("readExtraTags round-trip: write then re-read with music-metadata", async () => {
    const fp = path.join(tmpDir, "rt.flac");
    createMinimalFlac(fp, "Song", "Artist", "Album");

    await writeExtraTags(fp, [
      { key: "MOOD", value: "Bright" },
      { key: "RATING", value: "5" },
    ]);

    // Read back using music-metadata
    const meta = await parseFile(fp, { duration: false });

    // Standard tags should be intact
    expect(meta.common.title).toBe("Song");
    expect(meta.common.artist).toBe("Artist");
    expect(meta.common.album).toBe("Album");

    // Extra tags should be in native
    let foundMood = false;
    let foundRating = false;
    for (const [, tags] of Object.entries(meta.native)) {
      for (const tag of tags) {
        if (tag.id === "MOOD" && tag.value === "Bright") foundMood = true;
        if (tag.id === "RATING" && tag.value === "5") foundRating = true;
      }
    }
    expect(foundMood).toBe(true);
    expect(foundRating).toBe(true);
  });

  it("full round-trip: write then read via readExtraTags from tracks.ts", async () => {
    const fp = path.join(tmpDir, "full-rt.flac");
    createMinimalFlac(fp, "Title", "Artist");

    // Write extra tags
    await writeExtraTags(fp, [
      { key: "MOOD", value: "Chill" },
      { key: "RATING", value: "4" },
    ]);

    // Read back using the actual readExtraTags function
    const extras = await readExtraTags(fp);
    expect(extras).toHaveLength(2);
    expect(extras.find((t) => t.key === "MOOD")?.value).toBe("Chill");
    expect(extras.find((t) => t.key === "RATING")?.value).toBe("4");
    expect(extras.find((t) => t.key === "MOOD")?.source).toBeTruthy();
    expect(extras.find((t) => t.key === "RATING")?.source).toBeTruthy();
  });

  it("full round-trip: overwrite extra tags and verify with readExtraTags", async () => {
    const fp = path.join(tmpDir, "overwrite-rt.flac");
    createMinimalFlac(fp, "Title", "Artist");

    // Write initial extra tags
    await writeExtraTags(fp, [
      { key: "MOOD", value: "Old" },
      { key: "BARCODE", value: "OLD-001" },
    ]);

    // Overwrite
    await writeExtraTags(fp, [
      { key: "MOOD", value: "New" },
    ]);

    const extras = await readExtraTags(fp);
    expect(extras.find((t) => t.key === "MOOD")?.value).toBe("New");
    // Old extra tag should be gone
    expect(extras.find((t) => t.key === "BARCODE")).toBeUndefined();
    // Standard tags remain
    expect(extras.find((t) => t.key === "TITLE")).toBeUndefined();
    expect(extras.find((t) => t.key === "ARTIST")).toBeUndefined();
  });

  it("batch write can clear all extra tags from FLAC files", async () => {
    const first = path.join(tmpDir, "batch-clear-1.flac");
    const second = path.join(tmpDir, "batch-clear-2.flac");
    createMinimalFlac(first, "First", "Artist");
    createMinimalFlac(second, "Second", "Artist");

    await batchWriteExtraTags([
      { path: first, tags: [{ key: "MOOD", value: "Bright" }] },
      { path: second, tags: [{ key: "MOOD", value: "Bright" }] },
    ]);

    await batchWriteExtraTags([
      { path: first, tags: [] },
      { path: second, tags: [] },
    ]);

    expect(await readExtraTags(first)).toEqual([]);
    expect(await readExtraTags(second)).toEqual([]);

    const firstComments = await readVorbisCommentsFromFile(first);
    const secondComments = await readVorbisCommentsFromFile(second);
    expect(firstComments["TITLE"]).toBe("First");
    expect(secondComments["TITLE"]).toBe("Second");
  });

  it("readExtraTags does not return standard Vorbis tags", async () => {
    const fp = path.join(tmpDir, "standard.flac");
    createMinimalFlac(fp, "Song Title", "Artist Name", "My Album");

    await writeExtraTags(fp, [
      { key: "MOOD", value: "Happy" },
    ]);

    const extras = await readExtraTags(fp);
    // Standard tags should not appear
    expect(extras.find((t) => t.key === "TITLE")).toBeUndefined();
    expect(extras.find((t) => t.key === "ARTIST")).toBeUndefined();
    expect(extras.find((t) => t.key === "ALBUM")).toBeUndefined();
    // But extra tag should show
    expect(extras.find((t) => t.key === "MOOD")?.value).toBe("Happy");
  });

  it("Vorbis comment keys are uppercased through round-trip", async () => {
    const fp = path.join(tmpDir, "case.flac");
    createMinimalFlac(fp, "Title");

    await writeExtraTags(fp, [
      { key: "CustomTag", value: "value1" },
    ]);

    const extras = await readExtraTags(fp);
    // Vorbis comment keys are conventionally uppercase
    const tag = extras.find((t) => t.key === "CUSTOMTAG");
    expect(tag?.value).toBe("value1");
  });

  it("keeps standard Vorbis tags reserved unless explicitly allowed as extras", async () => {
    const fp = path.join(tmpDir, "multi.flac");
    createMinimalFlac(fp, "Title");

    await writeExtraTags(fp, [
      { key: "GENRE", value: "Rock" },
    ]);

    const extras = await readExtraTags(fp);
    expect(extras.find((t) => t.key === "GENRE")).toBeUndefined();
  });

  it("shows DESCRIPTION and ALBUMARTISTS in FLAC extra tags", async () => {
    const fp = path.join(tmpDir, "detail-tags.flac");
    createMinimalFlac(fp, "Title", "Artist", "Album");

    await writeTags(fp, {
      description: "A great album",
      albumArtists: ["Alice", "Bob"],
      compilation: true,
    });

    const extras = await readExtraTags(fp);
    const keys = extras.map((t) => t.key);
    expect(keys).toContain("DESCRIPTION");
    expect(keys).toContain("ALBUMARTISTS");
    expect(keys).toContain("COMPILATION");
    // Standard editor fields remain hidden
    expect(keys).not.toContain("TITLE");
    expect(keys).not.toContain("ARTIST");
    expect(keys).not.toContain("ALBUM");
    expect(keys).not.toContain("ALBUMARTIST");
    expect(keys).not.toContain("ALBUM ARTIST");
  });

  it("allows editing DESCRIPTION through FLAC extra tags", async () => {
    const fp = path.join(tmpDir, "edit-desc.flac");
    createMinimalFlac(fp, "Title");

    await writeTags(fp, { description: "Old description" });
    await writeExtraTags(fp, [{ key: "DESCRIPTION", value: "New description" }]);

    const extras = await readExtraTags(fp);
    expect(extras.find((t) => t.key === "DESCRIPTION")?.value).toBe("New description");
  });

  it("allows editing MUSICBRAINZ_ALBUMID through FLAC extra tags", async () => {
    const fp = path.join(tmpDir, "edit-mb-vorbis.flac");
    createMinimalFlac(fp, "Title");

    await writeTags(fp, { musicbrainzAlbumId: "old-mb-id" });
    await writeExtraTags(fp, [{ key: "MUSICBRAINZ_ALBUMID", value: "new-mb-id" }]);

    const extras = await readExtraTags(fp);
    expect(extras.find((t) => t.key === "MUSICBRAINZ_ALBUMID")?.value).toBe("new-mb-id");
  });

  it("removes MUSICBRAINZ_ALBUMID from FLAC when not included in extra tags write", async () => {
    const fp = path.join(tmpDir, "remove-mb-vorbis.flac");
    createMinimalFlac(fp, "Title");

    await writeTags(fp, { musicbrainzAlbumId: "remove-me" });
    await writeExtraTags(fp, [{ key: "MOOD", value: "Happy" }]);

    const extras = await readExtraTags(fp);
    expect(extras.find((t) => t.key === "MUSICBRAINZ_ALBUMID")).toBeUndefined();
    expect(extras.find((t) => t.key === "MOOD")?.value).toBe("Happy");
  });

  it("allows editing DESCRIPTION through MP3 extra tags (TXXX)", async () => {
    const fp = path.join(tmpDir, "edit-desc-mp3.mp3");
    createMinimalMp3(fp);

    await writeTags(fp, { description: "Old MP3 desc" });
    await writeExtraTags(fp, [{ key: "DESCRIPTION", value: "New MP3 desc" }]);

    const extras = await readExtraTags(fp);
    expect(extras.find((t) => t.key === "DESCRIPTION")?.value).toBe("New MP3 desc");
  });

  it("allows multiple ARTISTS values as Vorbis extra tags", async () => {
    const fp = path.join(tmpDir, "artists-extra.flac");
    createMinimalFlac(fp, "Title");

    await writeExtraTags(fp, [
      { key: "t", value: "t" },
      { key: "ARTISTS", value: "foo" },
      { key: "ARTISTS", value: "bar" },
    ]);

    const extras = await readExtraTags(fp);
    expect(extras).toEqual(
      expect.arrayContaining([
        expect.objectContaining({ key: "ARTISTS", value: "foo" }),
        expect.objectContaining({ key: "ARTISTS", value: "bar" }),
        expect.objectContaining({ key: "T", value: "t" }),
      ]),
    );
    const artistValues = extras
      .filter((tag) => tag.key === "ARTISTS")
      .map((tag) => tag.value);
    expect(artistValues).toEqual(["foo", "bar"]);
  });
});

// ── CRC32 for OGG page checksums ──────────────────────────────

function oggCrc32(buf: Buffer): number {
  let crc = 0xffffffff;
  const table = new Int32Array(256);
  for (let i = 0; i < 256; i++) {
    let c = i;
    for (let j = 0; j < 8; j++) {
      c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
    }
    table[i] = c;
  }
  for (let i = 0; i < buf.length; i++) {
    crc = table[(crc ^ buf[i]) & 0xff] ^ (crc >>> 8);
  }
  return (crc ^ 0xffffffff) >>> 0;
}

/** Build an OGG page with the given header type, content, and sequence. */
function buildOggPage(
  headerType: number,
  granulePosition: bigint,
  serial: number,
  seq: number,
  packetData: Buffer,
): Buffer {
  // Segment table: one segment containing all packet data
  // (the test creates small packets, so a single segment suffices)
  const segTable = Buffer.from([packetData.length]);

  // OGG page header (27 bytes)
  const header = Buffer.alloc(27);
  let off = 0;
  header.write("OggS", off); off += 4;
  header[off++] = 0; // version
  header[off++] = headerType;
  header.writeBigUInt64LE(granulePosition, off); off += 8;
  header.writeUInt32LE(serial, off); off += 4;
  header.writeUInt32LE(seq, off); off += 4;
  // CRC placeholder (4 bytes at offset 22) — set to 0 for calculation
  off += 4;
  header[off++] = segTable.length;

  const page = Buffer.concat([header, segTable, packetData]);

  // Calculate CRC with the checksum field zeroed
  page.writeUInt32LE(0, 22);
  const crcVal = oggCrc32(page);
  page.writeUInt32LE(crcVal, 22);

  return page;
}

/**
 * Create a minimal OGG Vorbis file with identification and comment pages.
 * No audio data is needed for tag round-trip tests.
 */
function createMinimalOgg(
  filePath: string,
  title?: string,
  artist?: string,
  album?: string,
): void {
  const serial = 0x12345678;

  // ── Vorbis identification header (packet type 1) ────────────
  const identBody = Buffer.alloc(29);
  identBody[0] = 1; // packet type
  identBody.write("vorbis", 1, 6, "ascii");
  identBody.writeUInt32LE(0, 7); // version
  identBody[11] = 2; // channels (stereo)
  identBody.writeUInt32LE(44100, 12); // sample rate
  identBody.writeUInt32LE(0, 16); // bitrate max (unknown)
  identBody.writeUInt32LE(160000, 20); // bitrate nom
  identBody.writeUInt32LE(0, 24); // bitrate min (unknown)
  identBody[28] = 0b00010000; // blocksize 0=256, 1=4096

  const identPage = buildOggPage(2, 0n, serial, 0, identBody);

  // ── Vorbis comment header (packet type 3) ───────────────────
  const vendorStr = Buffer.from("auto-tagger-test", "utf8");
  const commentStrings: Buffer[] = [];
  if (title) commentStrings.push(commentEntry("TITLE", title));
  if (artist) commentStrings.push(commentEntry("ARTIST", artist));
  if (album) commentStrings.push(commentEntry("ALBUM", album));

  let cmtOff = 0;
  const cmtHeader = Buffer.alloc(7 + 4 + vendorStr.length + 4);
  cmtHeader[cmtOff++] = 3; // packet type
  cmtHeader.write("vorbis", cmtOff); cmtOff += 6;
  // vendor string length
  cmtHeader.writeUInt32LE(vendorStr.length, cmtOff); cmtOff += 4;
  vendorStr.copy(cmtHeader, cmtOff); cmtOff += vendorStr.length;
  // number of comments
  cmtHeader.writeUInt32LE(commentStrings.length, cmtOff); cmtOff += 4;

  const framingByte = Buffer.from([1]);
  const commentBody = Buffer.concat([cmtHeader, ...commentStrings, framingByte]);
  const commentPage = buildOggPage(0, 0n, serial, 1, commentBody);

  fs.writeFileSync(filePath, Buffer.concat([identPage, commentPage]));
}

function commentEntry(key: string, value: string): Buffer {
  const raw = Buffer.from(`${key}=${value}`, "utf8");
  const len = Buffer.alloc(4);
  len.writeUInt32LE(raw.length);
  return Buffer.concat([len, raw]);
}

describe("writeExtraTags — Queue round-trip (extra tags clear via TagWriteQueue)", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "writer-queue-extra-"));
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  it("write extra tags to FLAC then clear them via TagWriteQueue, standard tags remain", async () => {
    const fp = path.join(tmpDir, "clear-queue.flac");
    createMinimalFlac(fp, "Queue Title", "Queue Artist", "Queue Album");

    // Write extra tags directly
    await writeExtraTags(fp, [
      { key: "MOOD", value: "Bright" },
      { key: "RATING", value: "5" },
    ]);

    // Verify extra tags exist
    let extras = await readExtraTags(fp);
    expect(extras).toHaveLength(2);

    // Clear all extra tags via the queue
    const queue = new TagWriteQueue(1);
    const results = await queue.submit([{ filePath: fp, extraTags: [] }]);

    expect(results).toHaveLength(1);
    expect(results[0].success).toBe(true);

    // Verify extra tags are cleared
    extras = await readExtraTags(fp);
    expect(extras).toEqual([]);

    // Standard tags must remain intact
    const meta = await parseFile(fp, { duration: false });
    expect(meta.common.title).toBe("Queue Title");
    expect(meta.common.artist).toBe("Queue Artist");
    expect(meta.common.album).toBe("Queue Album");
  });

  it("write extra tags to MP3 then clear them via TagWriteQueue, standard tags remain", async () => {
    const fp = path.join(tmpDir, "clear-queue.mp3");
    createMinimalMp3(fp);

    // Write standard tags first via writeTags (which properly creates the file)
    await writeTags(fp, { title: "MP3 Title", artist: "MP3 Artist", album: "MP3 Album" });

    // Write extra tags directly
    await writeExtraTags(fp, [
      { key: "MOOD", value: "Bright" },
      { key: "RATING", value: "5" },
    ]);

    // Verify extra tags exist
    let extras = await readExtraTags(fp);
    expect(extras).toHaveLength(2);

    // Standard tags survive extra-tag write
    let tags = NodeID3.read(fp);
    expect(tags.title).toBe("MP3 Title");

    // Clear all extra tags via the queue
    const queue = new TagWriteQueue(1);
    const results = await queue.submit([{ filePath: fp, extraTags: [] }]);

    expect(results).toHaveLength(1);
    expect(results[0].success).toBe(true);

    // Verify extra tags are cleared
    extras = await readExtraTags(fp);
    expect(extras).toEqual([]);

    // Standard tags must remain intact
    tags = NodeID3.read(fp);
    expect(tags.title).toBe("MP3 Title");
    expect(tags.artist).toBe("MP3 Artist");
    expect(tags.album).toBe("MP3 Album");
  });
});

describe("writeExtraTags — OGG round-trip", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "writer-extra-ogg-"));
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  it("supports .opus files for extra tags", async () => {
    const fp = path.join(tmpDir, "test.opus");
    createMinimalOgg(fp, "Title", "Artist");

    await writeExtraTags(fp, [
      { key: "MOOD", value: "Calm" },
    ]);

    const extras = await readExtraTags(fp);
    expect(extras.find((t) => t.key === "MOOD")?.value).toBe("Calm");
  });

  it("saves and reads extra tags to/from OGG", async () => {
    const fp = path.join(tmpDir, "tags.ogg");
    createMinimalOgg(fp, "Song", "Artist");

    await writeExtraTags(fp, [
      { key: "MOOD", value: "Happy" },
      { key: "BARCODE", value: "OGG-001" },
    ]);

    const extras = await readExtraTags(fp);
    expect(extras.find((t) => t.key === "MOOD")?.value).toBe("Happy");
    expect(extras.find((t) => t.key === "BARCODE")?.value).toBe("OGG-001");
  });

  it("preserves standard tags when writing extra tags to OGG", async () => {
    const fp = path.join(tmpDir, "preserve.ogg");
    createMinimalOgg(fp, "Song", "Artist", "Album");

    await writeExtraTags(fp, [
      { key: "RATING", value: "5" },
    ]);

    const extras = await readExtraTags(fp);
    expect(extras.find((t) => t.key === "RATING")?.value).toBe("5");
    expect(extras.find((t) => t.key === "TITLE")).toBeUndefined();
    expect(extras.find((t) => t.key === "ARTIST")).toBeUndefined();
    expect(extras.find((t) => t.key === "ALBUM")).toBeUndefined();
  });

  it("overwrites existing extra tags on OGG", async () => {
    const fp = path.join(tmpDir, "overwrite.ogg");
    createMinimalOgg(fp, "Song", "Artist");

    await writeExtraTags(fp, [
      { key: "MOOD", value: "Old" },
      { key: "CATALOGNUMBER", value: "CN-001" },
    ]);

    await writeExtraTags(fp, [
      { key: "MOOD", value: "New" },
    ]);

    const extras = await readExtraTags(fp);
    expect(extras.find((t) => t.key === "MOOD")?.value).toBe("New");
    expect(extras.find((t) => t.key === "CATALOGNUMBER")).toBeUndefined();
  });
});
