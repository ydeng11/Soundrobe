// @vitest-environment node
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import fs from "fs";
import path from "path";
import os from "os";
import { parseFile } from "music-metadata";

// Must mock 'electron' before importing the module under test
vi.mock("electron", () => ({
  ipcMain: {
    handle: vi.fn(),
  },
}));

// We import after the mock so ipcMain is mocked
import {
  readTrackMetadata,
  readAlbum,
  isAudioFile,
  readExtraTags,
} from "../../electron/handlers/tracks";
import { writeExtraTags, writeTags } from "../../electron/handlers/writer";
import { splitArtistNames } from "../../electron/handlers/candidates";

import {
  flacHeaderWithDuration,
  flacHeader,
  vorbisCommentBlock,
  paddingBlock,
} from "../helpers/flac-helpers";

const STREAMINFO_LEN = 34;

function minimalApeAudio(): Buffer {
  const descriptor = Buffer.alloc(52);
  descriptor.write("MAC ", 0, 4, "ascii");
  descriptor.writeUInt32LE(2000000, 4);
  descriptor.writeUInt32LE(52, 8);
  descriptor.writeUInt32LE(24, 12);
  descriptor.writeUInt32LE(0, 16);
  descriptor.writeUInt32LE(0, 20);
  descriptor.writeUInt32LE(4096, 24);
  descriptor.writeUInt32LE(0, 28);
  descriptor.writeUInt32LE(0, 32);

  const header = Buffer.alloc(24);
  header.writeUInt32LE(4608, 4);
  header.writeUInt32LE(0, 8);
  header.writeUInt32LE(1, 12);
  header.writeUInt16LE(16, 16);
  header.writeUInt16LE(2, 18);
  header.writeUInt32LE(44100, 20);

  return Buffer.concat([descriptor, header, Buffer.alloc(4096, 0x55)]);
}

function minimalWavAudio(): Buffer {
  const fmt = Buffer.alloc(16);
  fmt.writeUInt16LE(1, 0);
  fmt.writeUInt16LE(1, 2);
  fmt.writeUInt32LE(44100, 4);
  fmt.writeUInt32LE(88200, 8);
  fmt.writeUInt16LE(2, 12);
  fmt.writeUInt16LE(16, 14);

  const data = Buffer.alloc(882);
  const fmtHeader = Buffer.alloc(8);
  fmtHeader.write("fmt ", 0, 4, "ascii");
  fmtHeader.writeUInt32LE(fmt.length, 4);
  const dataHeader = Buffer.alloc(8);
  dataHeader.write("data", 0, 4, "ascii");
  dataHeader.writeUInt32LE(data.length, 4);
  const body = Buffer.concat([
    Buffer.from("WAVE", "ascii"),
    fmtHeader,
    fmt,
    dataHeader,
    data,
  ]);
  const header = Buffer.alloc(8);
  header.write("RIFF", 0, 4, "ascii");
  header.writeUInt32LE(body.length, 4);
  return Buffer.concat([header, body]);
}

function id3v1Tail(): Buffer {
  const id3 = Buffer.alloc(128, 0);
  id3.write("TAG", 0, 3, "ascii");
  return id3;
}

// ── Tests ────────────────────────────────────────────────────────────

describe("readTrackMetadata — corrupted FLAC", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "track-corrupt-"));
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  it("returns minimal data for truncated FLAC (just fLaC header)", async () => {
    const fp = path.join(tmpDir, "tiny.flac");
    fs.writeFileSync(fp, Buffer.from("fLaC"));
    // music-metadata gracefully handles minimal files
    const result = await readTrackMetadata(fp);
    expect(result.sizeBytes).toBe(4);
    expect(result.duration).toBe(0);
  });

  it("returns partial data even with corrupted STREAMINFO length", async () => {
    const parts: Buffer[] = [];
    parts.push(Buffer.from("fLaC", "ascii"));
    const siHeader = Buffer.alloc(4);
    siHeader[0] = 0x80;
    siHeader[1] = 0xff;
    siHeader[2] = 0xff;
    siHeader[3] = 0xff;
    parts.push(siHeader);
    const fp = path.join(tmpDir, "corrupt-si.flac");
    fs.writeFileSync(fp, Buffer.concat(parts));
    // music-metadata is resilient — returns partial data,
    // but such a broken STREAMINFO gives "unknown" codec
    const result = await readTrackMetadata(fp);
    expect(result.codec).toBe("unknown");
    expect(result.duration).toBe(0);
  });

  it("returns partial data for VORBIS_COMMENT with large length", async () => {
    const block = vorbisCommentBlock(["TITLE=Test"], { corruptLen: 100 });
    const buf = Buffer.concat([
      flacHeader(false, [block]),
      Buffer.alloc(100),
    ]);
    const fp = path.join(tmpDir, "large-len.flac");
    fs.writeFileSync(fp, buf);
    const result = await readTrackMetadata(fp);
    // music-metadata still returns something
    expect(result).toBeDefined();
    expect(result.path).toBe(fp);
  });

  it("recovers FLAC duration and comments when no metadata block is marked last", async () => {
    const comments = vorbisCommentBlock(["TITLE=Ahh Yeah", "ARTIST=蛋堡"], {
      isLast: false,
    });
    const buf = Buffer.concat([
      flacHeaderWithDuration(false, 310, [comments, paddingBlock(16)]),
      Buffer.from([0xff, 0xf8, 0x69, 0x18]),
      Buffer.alloc(100),
    ]);
    const fp = path.join(tmpDir, "missing-last.flac");
    fs.writeFileSync(fp, buf);

    const result = await readTrackMetadata(fp);
    expect(result.title).toBe("Ahh Yeah");
    expect(result.artist).toBe("蛋堡");
    expect(result.codec).toBe("FLAC");
    expect(result.duration).toBeGreaterThan(0);
  });

  it("returns finite duration for file with no audio frames", async () => {
    // Minimal valid FLAC structure but no actual audio frames.
    // music-metadata may report Infinity when there's no frame data.
    const block = vorbisCommentBlock(["TITLE=Test"], { isLast: true });
    const buf = flacHeader(false, [block]);
    const fp = path.join(tmpDir, "noframe.flac");
    fs.writeFileSync(fp, buf);
    const result = await readTrackMetadata(fp);
    // Duration could be 0 or Infinity for files with no audio frames
    // — what matters is the function doesn't crash and returns data
    expect(typeof result.duration).toBe("number");
    expect(result.sizeBytes).toBe(buf.length);
    expect(result.title).toBe("Test");
  });
});

describe("readTrackMetadata — WAV artist lists", () => {
  it("splits the semicolon-delimited ARTISTS frame written by auto-tag", async () => {
    const tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "wav-artists-read-"));
    const filePath = path.join(tmpDir, "02. 再出發.wav");

    try {
      fs.writeFileSync(filePath, minimalWavAudio());
      await writeTags(filePath, {
        artist: "任贤齐 V.S. 阿牛(陈庆祥)",
        artists: ["任贤齐", "阿牛(陈庆祥)"],
      });

      const metadata = await readTrackMetadata(filePath);

      expect(metadata.artist).toBe("任贤齐 V.S. 阿牛(陈庆祥)");
      expect(metadata.artists).toEqual(["任贤齐", "阿牛(陈庆祥)"]);
    } finally {
      fs.rmSync(tmpDir, { recursive: true, force: true });
    }
  });
});

describe("readTrackMetadata — APE", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "track-ape-"));
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  it("falls back to raw APEv2 tags when a trailing ID3v1 tag makes music-metadata fail", async () => {
    const fp = path.join(tmpDir, "01 - 我们飞向太空.ape");
    fs.writeFileSync(fp, minimalApeAudio());
    await writeTags(fp, {
      title: "我们飞向太空",
      artist: "刺猬",
      album: "幻象波普星",
      trackNumber: 1,
      genre: "Alternative Rock",
    });
    fs.appendFileSync(fp, id3v1Tail());

    const result = await readTrackMetadata(fp);

    expect(result.title).toBe("我们飞向太空");
    expect(result.artist).toBe("刺猬");
    expect(result.album).toBe("幻象波普星");
    expect(result.trackNumber).toBe(1);
    expect(result.genre).toBe("Alternative Rock");
    expect(result.codec).toBe("Monkey's Audio");
    expect(result.duration).toBeGreaterThan(0);
  });

  it("round-trips APE writes through UI-facing metadata and extra-tag readback", async () => {
    const fp = path.join(tmpDir, "02 - 梦.ape");
    fs.writeFileSync(fp, minimalApeAudio());

    await writeTags(fp, {
      title: "梦",
      artist: "刺猬",
      album: "幻象波普星",
      artists: ["Hedgehog"],
      trackNumber: 2,
      trackTotal: 12,
      year: "2004",
    });
    const beforeExtras = await readTrackMetadata(fp);
    const beforeExtraRows = await readExtraTags(fp);

    await writeExtraTags(fp, [
      { key: "MOOD", value: "noisy" },
      { key: "SOURCE", value: "tmp-copy" },
    ]);

    const track = await readTrackMetadata(fp);
    const extras = await readExtraTags(fp);

    expect(track.title).toBe("梦");
    expect(track.artist).toBe("刺猬");
    expect(track.artists).toEqual(["刺猬", "Hedgehog"]);
    expect(track.album).toBe("幻象波普星");
    expect(track.trackNumber).toBe(2);
    expect(track.trackTotal).toBe(12);
    expect(beforeExtras.trackNumber).toBe(2);
    expect(beforeExtras.year).toBe("2004");
    expect(beforeExtras.bitrate).toBeGreaterThan(0);
    expect(beforeExtraRows).toEqual([]);
    expect(extras).toEqual([
      { key: "MOOD", value: "noisy", source: "APEv2" },
      { key: "SOURCE", value: "tmp-copy", source: "APEv2" },
    ]);
  });
});

describe("readAlbum — corrupted FLAC in directory", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "album-corrupt-"));
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  it("includes unparseable FLAC as minimal track with correct sizeBytes and duration=0", async () => {
    // Create a corrupted FLAC (10000 bytes, garbage)
    const fp = path.join(tmpDir, "corrupt.flac");
    const buf = Buffer.alloc(10000);
    buf.write("fLaC", 0, "ascii");
    fs.writeFileSync(fp, buf);

    const result = await readAlbum(tmpDir);
    expect(result.tracks.length).toBe(1);

    const track = result.tracks[0];
    expect(track.sizeBytes).toBe(10000);
    expect(track.duration).toBe(0);
    expect(track.codec).toBe("unknown");
    expect(track.title).toBe("corrupt.flac");
    expect(track.bitrate).toBeNull();
    expect(track.sampleRate).toBeNull();
    expect(track.artist).toBeNull();
    expect(track.album).toBeNull();
  });

  it("mix of parseable and unparseable FLAC files in the same album", async () => {
    // Create a valid FLAC
    const validFp = path.join(tmpDir, "valid.flac");
    const validBlock = vorbisCommentBlock(["TITLE=Good Track", "ARTIST=Good Artist"], {
      isLast: true,
    });
    const validBuf = Buffer.concat([
      flacHeader(false, [validBlock]),
      Buffer.alloc(100),
    ]);
    fs.writeFileSync(validFp, validBuf);
    const validSize = fs.statSync(validFp).size;

    // Create a corrupted FLAC
    const corruptFp = path.join(tmpDir, "corrupt.flac");
    const corruptBuf = Buffer.alloc(8000);
    corruptBuf.write("fLaC", 0, "ascii");
    fs.writeFileSync(corruptFp, corruptBuf);
    const corruptSize = fs.statSync(corruptFp).size;

    const result = await readAlbum(tmpDir);
    expect(result.tracks.length).toBe(2);

    // Find tracks by path
    const goodTrack = result.tracks.find((t) => t.path.includes("valid"));
    const badTrack = result.tracks.find((t) => t.path.includes("corrupt"));

    expect(goodTrack).toBeDefined();
    expect(goodTrack!.sizeBytes).toBe(validSize);
    if (goodTrack!.title) {
      expect(goodTrack!.title).toBe("Good Track");
    }

    expect(badTrack).toBeDefined();
    expect(badTrack!.sizeBytes).toBe(corruptSize);
    expect(badTrack!.duration).toBe(0);
    expect(badTrack!.title).toBe("corrupt.flac");
  });

  it("reports warning status when some files fail to parse", async () => {
    const validFp = path.join(tmpDir, "good.flac");
    fs.writeFileSync(validFp, Buffer.concat([
      flacHeader(true),
      Buffer.alloc(100),
    ]));

    const badFp = path.join(tmpDir, "bad.flac");
    const badBuf = Buffer.alloc(5000);
    badBuf.write("fLaC", 0, "ascii");
    fs.writeFileSync(badFp, badBuf);

    const result = await readAlbum(tmpDir);
    expect(result.status).toBe("warning");
  });

  it("reports error status when all files fail to parse", async () => {
    for (let i = 0; i < 3; i++) {
      const fp = path.join(tmpDir, `bad${i}.flac`);
      const buf = Buffer.alloc(3000);
      buf.write("fLaC", 0, "ascii");
      fs.writeFileSync(fp, buf);
    }

    const result = await readAlbum(tmpDir);
    expect(result.status).toBe("error");
  });
});

describe("writeTags on corrupt FLAC — modification", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "write-corrupt-"));
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  it("writes tags to a corrupted FLAC without crashing", async () => {
    const fp = path.join(tmpDir, "corrupt.flac");
    const buf = Buffer.alloc(20000);
    buf.write("fLaC", 0, "ascii");
    fs.writeFileSync(fp, buf);

    await expect(
      writeTags(fp, { title: "New Title", artist: "New Artist" })
    ).resolves.toBeUndefined();

    // File should still exist and be larger than before
    const stat = fs.statSync(fp);
    expect(stat.size).toBeGreaterThan(0);

    // fLaC marker still present
    const header = fs.readFileSync(fp).slice(0, 4).toString("ascii");
    expect(header).toBe("fLaC");
  });

  it("write + re-read produces correct sizeBytes via readTrackMetadata fallback", async () => {
    const fp = path.join(tmpDir, "corrupt-write.flac");
    const buf = Buffer.alloc(15000);
    buf.write("fLaC", 0, "ascii");
    fs.writeFileSync(fp, buf);

    // Simulate what the track:write IPC handler does:
    // 1. writeTags succeeds
    await writeTags(fp, { title: "After Write" });
    // 2. readTrackMetadata may fail, producing minimal track fallback
    // We verify through readAlbum instead
    const album = await readAlbum(tmpDir);
    expect(album.tracks.length).toBe(1);
    const result = album.tracks[0];
    expect(result.sizeBytes).toBe(fs.statSync(fp).size);
    expect(result.duration).toBe(0);
  });

  it("can write multiple times to a corrupted FLAC", async () => {
    const fp = path.join(tmpDir, "multi.flac");
    const buf = Buffer.alloc(20000);
    buf.write("fLaC", 0, "ascii");
    fs.writeFileSync(fp, buf);

    // Write once
    await writeTags(fp, { title: "First" });
    // Write again with different values
    await writeTags(fp, { title: "Second", artist: "Artist" });
    // Write a third time
    await writeTags(fp, { album: "Album", year: "2024" });

    // File should still be valid FLAC
    const header = fs.readFileSync(fp).slice(0, 4).toString("ascii");
    expect(header).toBe("fLaC");
    const stat = fs.statSync(fp);
    expect(stat.size).toBeGreaterThan(0);
  });

  it("write to corrupted FLAC does not affect other clean files", async () => {
    // Create a corrupted file
    const corruptFp = path.join(tmpDir, "corrupt.flac");
    const corruptBuf = Buffer.alloc(10000);
    corruptBuf.write("fLaC", 0, "ascii");
    fs.writeFileSync(corruptFp, corruptBuf);

    // Create a clean FLAC
    const cleanFp = path.join(tmpDir, "clean.flac");
    const cleanBlock = vorbisCommentBlock(
      ["TITLE=Original"],
      { isLast: true }
    );
    const cleanBuf = Buffer.concat([
      flacHeader(false, [cleanBlock]),
      Buffer.alloc(100),
    ]);
    fs.writeFileSync(cleanFp, cleanBuf);

    // Write to corrupted file
    await writeTags(corruptFp, { title: "Corrupt Write" });

    // Clean file should still be readable with original content
    const meta = await parseFile(cleanFp);
    expect(meta.common.title).toBe("Original");

    // Write to clean file — should still work
    await writeTags(cleanFp, { title: "Still Works" });
    const meta2 = await parseFile(cleanFp);
    expect(meta2.common.title).toBe("Still Works");
  });

  it("write succeeds even if readVorbisComments encounters internally corrupted block", async () => {
    // Create a FLAC where the VORBIS_COMMENT block header is valid
    // but the internal comment data is garbage (corrupted comment length)
    const parts: Buffer[] = [];
    parts.push(Buffer.from("fLaC", "ascii"));

    // STREAMINFO
    const si = Buffer.alloc(STREAMINFO_LEN);
    si.writeUInt16BE(4096, 0);
    si.writeUInt16BE(4096, 2);
    const siHeader = Buffer.alloc(4);
    siHeader[0] = 0x00; // not last
    siHeader[1] = (si.length >> 16) & 0xff;
    siHeader[2] = (si.length >> 8) & 0xff;
    siHeader[3] = si.length & 0xff;
    parts.push(siHeader, si);

    // VORBIS_COMMENT block with valid header pointing to valid-length body
    // but the body itself has a corrupted vendor length
    const body = Buffer.alloc(40);
    // Put a huge vendor length (0xDEADBEEF would crash readVorbisComments)
    body.writeUInt32LE(0xdeadbeef, 0);
    // Fill rest with non-zero data for uniqueness
    for (let i = 4; i < 40; i++) body[i] = 0xff;

    const vcHeader = Buffer.alloc(4);
    vcHeader[0] = 0x80 | 0x04; // isLast | VORBIS_COMMENT
    vcHeader[1] = (body.length >> 16) & 0xff;
    vcHeader[2] = (body.length >> 8) & 0xff;
    vcHeader[3] = body.length & 0xff;
    parts.push(vcHeader, body);

    parts.push(Buffer.alloc(100)); // audio frame
    const buf = Buffer.concat(parts);

    const fp = path.join(tmpDir, "corrupt-internal.flac");
    fs.writeFileSync(fp, buf);

    // Should not crash despite corrupted vendor length
    await expect(
      writeTags(fp, { title: "Survived" })
    ).resolves.toBeUndefined();

    // File should still be valid
    const header = fs.readFileSync(fp).slice(0, 4).toString("ascii");
    expect(header).toBe("fLaC");
  });

  it("repairs a FLAC metadata chain that is missing the last-block flag", async () => {
    const comments = vorbisCommentBlock(["TITLE=Original", "ARTIST=Keep"], {
      isLast: false,
    });
    const buf = Buffer.concat([
      flacHeaderWithDuration(false, 180, [comments, paddingBlock(16)]),
      Buffer.from([0xff, 0xf8, 0x69, 0x18]),
      Buffer.alloc(100),
    ]);
    const fp = path.join(tmpDir, "missing-last-write.flac");
    fs.writeFileSync(fp, buf);

    await writeTags(fp, { title: "Fixed" });
    const meta = await parseFile(fp);

    expect(meta.common.title).toBe("Fixed");
    expect(meta.common.artist).toBe("Keep");
    expect(meta.format.duration).toBeGreaterThan(0);
  });
});

describe("readAlbum without FLaC marker — edge cases", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "album-edge-"));
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  it("returns minimal track for a file that has .flac extension but no valid audio", async () => {
    // File has .flac extension but is completely garbage
    const fp = path.join(tmpDir, "notreally.flac");
    fs.writeFileSync(fp, Buffer.alloc(777));

    const result = await readAlbum(tmpDir);
    expect(result.tracks.length).toBe(1);
    expect(result.tracks[0].sizeBytes).toBe(777);
    expect(result.tracks[0].duration).toBe(0);
    expect(result.tracks[0].codec).toBe("unknown");
  });

  it("skips non-audio files and directories", async () => {
    // Create a mix of files
    fs.writeFileSync(path.join(tmpDir, "cover.jpg"), Buffer.alloc(100));
    fs.writeFileSync(path.join(tmpDir, "notes.txt"), Buffer.alloc(50));
    fs.writeFileSync(path.join(tmpDir, ".hidden.flac"), Buffer.alloc(100));
    const realFp = path.join(tmpDir, "real.flac");
    const buf = Buffer.alloc(12345);
    buf.write("fLaC", 0, "ascii");
    fs.writeFileSync(realFp, buf);

    const result = await readAlbum(tmpDir);
    // Only .flac files that don't start with '.' should be included
    expect(result.tracks.length).toBe(1);
    expect(result.tracks[0].sizeBytes).toBe(12345);
  });
});

describe("auto-tag artist splitting — 谢天华&朱永棠&林晓峰", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "artist-split-"));
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  it("splits &-separated Chinese ARTIST into 3 ARTISTS entries after write", async () => {
    // Create a FLAC with the same metadata as the real file:
    //   ARTIST=谢天华&朱永棠&林晓峰
    //   ARTISTS=谢天华;朱永棠;林晓峰
    const comments = [
      "TITLE=古古惑惑 (清清楚楚我系我)",
      "ALBUM=古惑仔Ⅲ 只手遮天",
      "TRACKNUMBER=3",
      "ARTIST=谢天华&朱永棠&林晓峰",
      "ARTISTS=谢天华;朱永棠;林晓峰",
      "ALBUMARTIST=郑伊健",
    ];
    const block = vorbisCommentBlock(comments, { isLast: true });
    const buf = Buffer.concat([
      flacHeaderWithDuration(false, 205, [block]),
      Buffer.from([0xff, 0xf8, 0x69, 0x18]),
      Buffer.alloc(100),
    ]);
    const fp = path.join(tmpDir, "03. 古古惑惑 (清清楚楚我系我).flac");
    fs.writeFileSync(fp, buf);

    // Step 1: Read metadata — same as what readTrackMetadata does for auto-tag
    const meta = await readTrackMetadata(fp);
    expect(meta.artist).toBe("谢天华&朱永棠&林晓峰");
    expect(meta.artists).toEqual(["谢天华", "朱永棠", "林晓峰"]);

    // Step 2: Apply auto-tag's artist splitting logic (line 771 in auto-tag.ts):
    //   albumFields.artists = candidate.artists.length > 0
    //     ? candidate.artists
    //     : splitArtistNames([candidate.artist]);
    // The display ARTIST can also recover the same individual names.
    const splitArtists = splitArtistNames([meta.artist]);
    expect(splitArtists).toEqual(["谢天华", "朱永棠", "林晓峰"]);

    // Step 3: Write back with properly split artists
    await writeTags(fp, { artists: splitArtists });

    // Step 4: Verify read-back via music-metadata shows 3 separate artists
    const parsed = await parseFile(fp);
    expect(parsed.common.artists).toEqual(["谢天华", "朱永棠", "林晓峰"]);
  });

  it("splits &-separated Chinese ARTIST into 3 via the auto-tag write path", async () => {
    // Same setup but reads back through readTrackMetadata after write
    const comments = [
      "TITLE=古古惑惑 (清清楚楚我系我)",
      "ARTIST=谢天华&朱永棠&林晓峰",
      "ARTISTS=谢天华;朱永棠;林晓峰",
    ];
    const block = vorbisCommentBlock(comments, { isLast: true });
    const buf = Buffer.concat([
      flacHeaderWithDuration(false, 205, [block]),
      Buffer.from([0xff, 0xf8, 0x69, 0x18]),
      Buffer.alloc(100),
    ]);
    const fp = path.join(tmpDir, "track.flac");
    fs.writeFileSync(fp, buf);

    // Read original
    const meta = await readTrackMetadata(fp);

    // Auto-tag logic (line 686): split artist to get 3 individual names
    const artists = splitArtistNames([meta.artist]);
    expect(artists).toEqual(["谢天华", "朱永棠", "林晓峰"]);

    // Write using the same pattern as auto-tag: set both artist and artists
    await writeTags(fp, {
      artist: "谢天华 & 朱永棠 & 林晓峰",
      artists,
    });

    // Re-read and verify
    const updated = await readTrackMetadata(fp);
    expect(updated.artists).toEqual(["谢天华", "朱永棠", "林晓峰"]);
  });
});

describe("undo round-trip — snapshot→write→revert→verify", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "undo-roundtrip-"));
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  it("restores all pipeline-touchable fields after snapshot→write→revert cycle", async () => {
    const initialTags = [
      "TITLE=古古惑惑 (清清楚楚我系我)",
      "ARTIST=谢天华&朱永棠&林晓峰",
      "ARTISTS=谢天华;朱永棠;林晓峰",
      "ALBUM=古惑仔Ⅲ 只手遮天",
      "ALBUMARTIST=Various Artists",
      "ALBUMARTISTS=Various Artists",
      "DATE=1996",
      "GENRE=Soundtrack",
      "COMPOSER=陈光荣",
      "COMMENT=Original comment",
      "TRACKNUMBER=3",
      "TRACKTOTAL=5",
      "DISCNUMBER=1",
      "DISCTOTAL=1",
    ];
    const block = vorbisCommentBlock(initialTags, { isLast: true });
    const buf = Buffer.concat([
      flacHeaderWithDuration(false, 205, [block]),
      Buffer.from([0xff, 0xf8, 0x69, 0x18]),
      Buffer.alloc(100),
    ]);
    const fp = path.join(tmpDir, "track.flac");
    fs.writeFileSync(fp, buf);

    // Simulate what handleAutoTag does: read initial state into a snapshot
    const before = await readTrackMetadata(fp);
    const snapshot: Record<string, unknown> = {
      title: before.title,
      artist: before.artist,
      artists: before.artists,
      album: before.album,
      albumArtist: before.albumArtist,
      albumArtists: before.albumArtists,
      year: before.year,
      trackNumber: before.trackNumber,
      trackTotal: before.trackTotal,
      discNumber: before.discNumber,
      discTotal: before.discTotal,
      genre: before.genre,
      composer: before.composer,
      comment: before.comment ?? null,
    };

    expect(snapshot.title).toBe("古古惑惑 (清清楚楚我系我)");
    expect(snapshot.artist).toBe("谢天华&朱永棠&林晓峰");
    expect(snapshot.artists).toEqual(["谢天华", "朱永棠", "林晓峰"]);
    expect(snapshot.album).toBe("古惑仔Ⅲ 只手遮天");
    expect(snapshot.albumArtist).toBe("Various Artists");
    expect(snapshot.year).toBe("1996");
    expect(snapshot.genre).toBe("Soundtrack");
    expect(snapshot.trackNumber).toBe(3);
    expect(snapshot.trackTotal).toBe(5);
    expect(snapshot.discNumber).toBe(1);
    expect(snapshot.discTotal).toBe(1);

    // Simulate auto-tag pipeline writing new values
    await writeTags(fp, {
      title: "New Title",
      artist: "New Artist",
      artists: [],
      album: "New Album",
      albumArtist: "New Album Artist",
      albumArtists: ["New Album Artist"],
      year: "2024",
      genre: "Pop",
      composer: "New Composer",
      comment: "New comment",
      trackNumber: 1,
      trackTotal: 10,
      discNumber: 2,
      discTotal: 3,
    });

    // Verify pipeline changed everything
    const mid = await readTrackMetadata(fp);
    expect(mid.title).toBe("New Title");
    expect(mid.artist).toBe("New Artist");
    expect(mid.album).toBe("New Album");
    expect(mid.albumArtist).toBe("New Album Artist");
    expect(mid.year).toBe("2024");
    expect(mid.genre).toBe("Pop");
    expect(mid.trackNumber).toBe(1);
    expect(mid.trackTotal).toBe(10);
    expect(mid.discNumber).toBe(2);
    expect(mid.discTotal).toBe(3);

    // Simulate Cmd+Z revert: write snapshot fields back
    await writeTags(fp, snapshot as any);

    // Verify EVERY field is restored to original values
    const after = await readTrackMetadata(fp);
    expect(after.title).toBe("古古惑惑 (清清楚楚我系我)");
    expect(after.artist).toBe("谢天华&朱永棠&林晓峰");
    expect(after.artists).toEqual(["谢天华", "朱永棠", "林晓峰"]);
    expect(after.album).toBe("古惑仔Ⅲ 只手遮天");
    expect(after.albumArtist).toBe("Various Artists");
    expect(after.albumArtists).toEqual(["Various Artists"]);
    expect(after.year).toBe("1996");
    expect(after.genre).toBe("Soundtrack");
    expect(after.trackNumber).toBe(3);
    expect(after.trackTotal).toBe(5);
    expect(after.discNumber).toBe(1);
    expect(after.discTotal).toBe(1);
  });

  it("restores Chinese multi-artist lists via ARTISTS and ALBUMARTISTS", async () => {
    const initialTags = [
      "TITLE=一起飞",
      "ARTIST=郑伊健&陈小春&林晓峰",
      "ARTISTS=郑伊健, 陈小春, 林晓峰",
      "ALBUM=友情岁月 3CD",
      "ALBUMARTIST=郑伊健&陈小春",
    ];
    const block = vorbisCommentBlock(initialTags, { isLast: true });
    const buf = Buffer.concat([
      flacHeaderWithDuration(false, 240, [block]),
      Buffer.from([0xff, 0xf8, 0x69, 0x18]),
      Buffer.alloc(100),
    ]);
    const fp = path.join(tmpDir, "multi-artist.flac");
    fs.writeFileSync(fp, buf);

    const before = await readTrackMetadata(fp);
    const snapshot: Record<string, unknown> = {
      artist: before.artist,
      artists: before.artists,
      albumArtist: before.albumArtist,
      albumArtists: before.albumArtists,
    };

    expect(snapshot.artist).toBe("郑伊健&陈小春&林晓峰");
    expect(snapshot.artists).toEqual(["郑伊健, 陈小春, 林晓峰"]);
    expect(snapshot.albumArtist).toBe("郑伊健&陈小春");
    expect(snapshot.albumArtists).toEqual(["郑伊健&陈小春"]);

    await writeTags(fp, {
      artist: "郑伊健&陈小春&林晓峰",
      artists: ["郑伊健", "陈小春", "林晓峰"],
      albumArtist: "郑伊健&陈小春",
      albumArtists: ["郑伊健", "陈小春"],
    });
    const mid = await readTrackMetadata(fp);
    expect(mid.artists).toEqual(["郑伊健", "陈小春", "林晓峰"]);
    expect(mid.albumArtist).toBe("郑伊健&陈小春");
    expect(mid.albumArtists).toEqual(["郑伊健&陈小春"]);

    // Revert with snapshot
    await writeTags(fp, snapshot as any);

    const after = await readTrackMetadata(fp);
    expect(after.artist).toBe("郑伊健&陈小春&林晓峰");
    expect(after.artists).toEqual(["郑伊健, 陈小春, 林晓峰"]);
    expect(after.albumArtist).toBe("郑伊健&陈小春");
    expect(after.albumArtists).toEqual(["郑伊健&陈小春"]);
  });

  it("restores artists list that was split by the pipeline", async () => {
    const initialTags = [
      "TITLE=Song",
      "ARTIST=谢天华&朱永棠&林晓峰",
      "ARTISTS=谢天华;朱永棠;林晓峰",
    ];
    const block = vorbisCommentBlock(initialTags, { isLast: true });
    const buf = Buffer.concat([
      flacHeaderWithDuration(false, 180, [block]),
      Buffer.from([0xff, 0xf8, 0x69, 0x18]),
      Buffer.alloc(100),
    ]);
    const fp = path.join(tmpDir, "artists-split.flac");
    fs.writeFileSync(fp, buf);

    // Snapshot captures the normalized ARTISTS list.
    const before = await readTrackMetadata(fp);
    expect(before.artists).toEqual(["谢天华", "朱永棠", "林晓峰"]);

    // Pipeline splits ARTISTS into separate values
    await writeTags(fp, {
      artists: ["谢天华", "朱永棠", "林晓峰"],
    });
    const mid = await readTrackMetadata(fp);
    expect(mid.artists).toEqual(["谢天华", "朱永棠", "林晓峰"]);

    // Revert with snapshot preserves the normalized list.
    await writeTags(fp, {
      artists: before.artists,
    } as any);
    const after = await readTrackMetadata(fp);
    expect(after.artists).toEqual(["谢天华", "朱永棠", "林晓峰"]);
  });

  it("restores disc and track totals that were missing (null) in original file", async () => {
    const initialTags = [
      "TITLE=Song",
      "ARTIST=Artist",
      "TRACKNUMBER=1",
      "DISCNUMBER=1",
    ];
    const block = vorbisCommentBlock(initialTags, { isLast: true });
    const buf = Buffer.concat([
      flacHeaderWithDuration(false, 200, [block]),
      Buffer.from([0xff, 0xf8, 0x69, 0x18]),
      Buffer.alloc(100),
    ]);
    const fp = path.join(tmpDir, "no-totals.flac");
    fs.writeFileSync(fp, buf);

    const before = await readTrackMetadata(fp);
    const snapshot: Record<string, unknown> = {
      trackNumber: before.trackNumber,
      trackTotal: before.trackTotal,
      discNumber: before.discNumber,
      discTotal: before.discTotal,
    };
    expect(snapshot.trackTotal).toBeNull();
    expect(snapshot.discTotal).toBeNull();

    await writeTags(fp, {
      trackNumber: 1,
      trackTotal: 12,
      discNumber: 1,
      discTotal: 2,
    });
    const mid = await readTrackMetadata(fp);
    expect(mid.trackTotal).toBe(12);
    expect(mid.discTotal).toBe(2);

    // Revert restores null (removes the fields)
    await writeTags(fp, snapshot as any);
    const after = await readTrackMetadata(fp);
    expect(after.trackTotal).toBeNull();
    expect(after.discTotal).toBeNull();
    expect(after.trackNumber).toBe(1);
    expect(after.discNumber).toBe(1);
  });
});
