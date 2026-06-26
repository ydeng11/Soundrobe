// @vitest-environment node
import { describe, it, expect, beforeEach, afterEach } from "vitest";
import fs from "fs";
import path from "path";
import os from "os";
import { parseFile } from "music-metadata";
import { writeTags } from "../../electron/handlers/writer";
import { parseAlbumWithTags, candidateFromFolder } from "../../electron/handlers/fallback";
import { splitArtistNames } from "../../electron/handlers/candidates";
import { flacHeaderWithDuration, vorbisCommentBlock } from "../helpers/flac-helpers";
import type { LookupRequest } from "../../electron/handlers/candidates";

// ── Helpers ─────────────────────────────────────────────────────────

function syntheticFlac(tags: string[]): Buffer {
  const block = vorbisCommentBlock(tags, { isLast: true });
  return Buffer.concat([
    flacHeaderWithDuration(false, 200, [block]),
    Buffer.from([0xff, 0xf8, 0x69, 0x18]),
    Buffer.alloc(100),
  ]);
}

function writeSyntheticFlac(dir: string, filename: string, tags: string[]): string {
  const fp = path.join(dir, filename);
  fs.mkdirSync(path.dirname(fp), { recursive: true });
  fs.writeFileSync(fp, syntheticFlac(tags));
  return fp;
}

// ── isCompilationFolder (no files needed) ───────────────────────────

describe("isCompilationFolder", () => {
  it("returns true for common compilation folder names", async () => {
    const { isCompilationFolder } = await import("../../electron/handlers/fallback");
    expect(isCompilationFolder("Compilations")).toBe(true);
    expect(isCompilationFolder("compilations")).toBe(true);
    expect(isCompilationFolder("COMPILATIONS")).toBe(true);
    expect(isCompilationFolder("Various Artists")).toBe(true);
    expect(isCompilationFolder("various")).toBe(true);
    expect(isCompilationFolder("VA")).toBe(true);
    expect(isCompilationFolder("Soundtracks")).toBe(true);
    expect(isCompilationFolder("OST")).toBe(true);
    expect(isCompilationFolder("Samplers")).toBe(true);
  });

  it("returns false for real artist names", async () => {
    const { isCompilationFolder } = await import("../../electron/handlers/fallback");
    expect(isCompilationFolder("郑伊健")).toBe(false);
    expect(isCompilationFolder("陈小春")).toBe(false);
    expect(isCompilationFolder("Various")).toBe(true); // edge: "Various" IS a compilation folder
  });

  it("returns false for null/undefined", async () => {
    const { isCompilationFolder } = await import("../../electron/handlers/fallback");
    expect(isCompilationFolder(null)).toBe(false);
    expect(isCompilationFolder(undefined)).toBe(false);
    expect(isCompilationFolder("")).toBe(false);
  });
});

// ── candidateFromFolder — compilation handling (no files needed) ────

describe("candidateFromFolder — compilation handling", () => {
  it("sets Various Artists for a Compilations folder", async () => {
    const { candidateFromFolder } = await import("../../electron/handlers/fallback");
    const request: LookupRequest = {
      path: "/some/Compilations/Some Album",
      artistHint: "Various Artists",
      albumHint: "Some Album",
      yearHint: "2000",
      tracks: [],
    };
    const candidate = candidateFromFolder(request);
    expect(candidate.albumArtist).toBe("Various Artists");
    expect(candidate.albumArtists).toEqual(["Various Artists"]);
  });

  it("uses real artist name for a non-compilation folder", async () => {
    const { candidateFromFolder } = await import("../../electron/handlers/fallback");
    const request: LookupRequest = {
      path: "/some/ArtistName/Some Album",
      artistHint: "郑伊健",
      albumHint: "Some Album",
      yearHint: null,
      tracks: [],
    };
    const candidate = candidateFromFolder(request);
    expect(candidate.albumArtist).toBe("郑伊健");
    expect(candidate.albumArtists).toEqual(["郑伊健"]);
  });
});

// ── Pipeline tests (use synthetic FLACs in temp dirs) ───────────────

describe("auto-tag pipeline on Chinese multi-artist track", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "auto-tag-cn-"));
    // Create album folder under "Compilations" to trigger compilation detection
    tmpDir = path.join(tmpDir, "Compilations", "古惑仔Ⅲ 只手遮天");
    fs.mkdirSync(tmpDir, { recursive: true });
  });

  afterEach(() => {
    // Clean up the mkdtemp parent
    const parent = path.dirname(path.dirname(tmpDir));
    fs.rmSync(parent, { recursive: true, force: true });
  });

  it("splits &-separated TRACK-level ARTIST into 3 separate ARTISTS entries", async () => {
    // Create 3 tracks: one has &-separated artist, the others single artist
    writeSyntheticFlac(tmpDir, "01. 战无不胜.flac", [
      "TITLE=战无不胜", "ARTIST=陈小春",
      "ALBUM=古惑仔Ⅲ 只手遮天", "ALBUMARTIST=Various Artists",
    ]);
    writeSyntheticFlac(tmpDir, "02. 甘心替代你.flac", [
      "TITLE=甘心替代你", "ARTIST=郑伊健",
      "ALBUM=古惑仔Ⅲ 只手遮天", "ALBUMARTIST=Various Artists",
    ]);
    writeSyntheticFlac(tmpDir, "03. 古古惑惑 (清清楚楚我系我).flac", [
      "TITLE=古古惑惑 (清清楚楚我系我)", "ARTIST=谢天华&朱永棠&林晓峰",
      "ARTISTS=谢天华&朱永棠&林晓峰",
      "ALBUM=古惑仔Ⅲ 只手遮天", "ALBUMARTIST=Various Artists",
    ]);

    // Verify test 3 has unsplit artists
    const before = await parseFile(path.join(tmpDir, "03. 古古惑惑 (清清楚楚我系我).flac"));
    expect(before.common.artist).toBe("谢天华&朱永棠&林晓峰");
    expect(before.common.artists).toEqual(["谢天华&朱永棠&林晓峰"]);

    // Run the pipeline
    const request = await parseAlbumWithTags(tmpDir);
    const candidate = candidateFromFolder(request);

    // Find track 3 and apply the split
    const tc3 = candidate.tracks.find(t => t.trackNumber === 3)!;
    expect(tc3).toBeDefined();
    await writeTags(path.join(tmpDir, "03. 古古惑惑 (清清楚楚我系我).flac"), {
      title: tc3.title,
      artist: tc3.artist,
      artists: tc3.artists.length > 0 ? splitArtistNames(tc3.artists) : [],
      album: candidate.album,
      albumArtist: candidate.albumArtist,
      albumArtists: candidate.albumArtists,
    });

    // Verify ARTISTS is now 3 separate entries
    const after = await parseFile(path.join(tmpDir, "03. 古古惑惑 (清清楚楚我系我).flac"));
    expect(after.common.artist).toBe("谢天华&朱永棠&林晓峰");
    expect(after.common.artists).toEqual(["谢天华", "朱永棠", "林晓峰"]);
  });

  it("does not use Compilations as album artist under compilation folder", async () => {
    // Create a minimal file so parseAlbumWithTags can read it
    writeSyntheticFlac(tmpDir, "01. 战无不胜.flac", [
      "TITLE=战无不胜", "ARTIST=陈小春",
    ]);

    const request = await parseAlbumWithTags(tmpDir);
    expect(request.artistHint).toBe("Various Artists");

    const candidate = candidateFromFolder(request);
    expect(candidate.artist).toBe("Various Artists");
    expect(candidate.artists).toEqual(["Various Artists"]);
    expect(candidate.albumArtist).toBe("Various Artists");
    expect(candidate.albumArtists).toEqual(["Various Artists"]);
    // The album hint is stripped by cleanFolderName — the tmp dir name won't match
    // The key assertions are the album artist values above.
  });

  it("collects provider artist IDs from later tracks, not only the first file", async () => {
    const artistRoot = path.dirname(tmpDir);
    const albumDir = path.join(artistRoot, "小霞2.0");
    fs.mkdirSync(albumDir, { recursive: true });

    writeSyntheticFlac(albumDir, "01. 另外三件往事 Part 1.flac", [
      "TITLE=另外三件往事 Part 1",
      "ARTIST=Xiao Xia",
      "ALBUM=小霞2.0",
    ]);
    writeSyntheticFlac(albumDir, "05. 我的美丽.flac", [
      "TITLE=我的美丽",
      "ARTIST=Xiao Xia",
      "ALBUM=小霞2.0",
      "DISCOGS_ARTIST_ID=5244238",
      "DISCOGS_RELEASE_ID=33302142",
    ]);

    const request = await parseAlbumWithTags(albumDir);

    expect(request.discogsArtistId).toBe("5244238");
    expect(request.discogsReleaseId).toBe("33302142");
  });
});

// ── 2013-友情岁月 3CD — multi-disc compilation (synthetic files) ────

describe("2013-友情岁月 3CD — multi-disc compilation", () => {
  let tmpRoot: string;
  let albumDir: string;

  beforeEach(() => {
    tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "auto-tag-multi-"));
    albumDir = path.join(tmpRoot, "Compilations", "2013-友情岁月 3CD");
    fs.mkdirSync(albumDir, { recursive: true });

    // All 51 tracks in a single flat directory, tagged with disc/discNumber
    // Disc 1 — 15 tracks (兄弟篇)
    const disc1Artists = [
      "郑伊健", "谢天华&朱永棠&林晓峰", "郑伊健",
      "郑伊健&陈小春&林晓峰", "陈小春", "郑伊健",
      "郑伊健", "陈小春", "郑伊健&陈小春",
      "郑伊健", "郑伊健", "谢天华&朱永棠&林晓峰",
      "郑伊健", "郑伊健", "郑伊健&陈小春",
    ];
    for (let i = 0; i < 15; i++) {
      const tn = i + 1;
      const num = String(tn).padStart(2, "0");
      const artist = disc1Artists[i];
      writeSyntheticFlac(albumDir, `${num}. Track ${tn}.flac`, [
        `TITLE=Track ${tn}`,
        `ARTIST=${artist}`,
        `TRACKNUMBER=${tn}`,
        `DISCNUMBER=1`,
        "ALBUM=友情岁月 3CD",
        "ALBUMARTIST=郑伊健&陈小春",
      ]);
    }

    // Disc 2 — 18 tracks (伊健篇, filenames also start at 01)
    for (let i = 0; i < 18; i++) {
      const tn = i + 1;
      const num = String(tn).padStart(2, "0");
      writeSyntheticFlac(albumDir, `D2-${num}. Track ${tn}.flac`, [
        `TITLE=Disc2 Track ${tn}`,
        "ARTIST=郑伊健",
        `TRACKNUMBER=${tn}`,
        `DISCNUMBER=2`,
        "ALBUM=友情岁月 3CD",
        "ALBUMARTIST=郑伊健&陈小春",
      ]);
    }

    // Disc 3 — 18 tracks (小春篇, filenames also start at 01)
    for (let i = 0; i < 18; i++) {
      const tn = i + 1;
      const num = String(tn).padStart(2, "0");
      writeSyntheticFlac(albumDir, `D3-${num}. Track ${tn}.flac`, [
        `TITLE=Disc3 Track ${tn}`,
        "ARTIST=陈小春",
        `TRACKNUMBER=${tn}`,
        `DISCNUMBER=3`,
        "ALBUM=友情岁月 3CD",
        "ALBUMARTIST=郑伊健&陈小春",
      ]);
    }
  });

  afterEach(() => {
    fs.rmSync(tmpRoot, { recursive: true, force: true });
  });

  it("detects compilation folder and uses Various Artists", async () => {
    const request = await parseAlbumWithTags(albumDir);
    expect(request.artistHint).toBe("Various Artists");

    const candidate = candidateFromFolder(request);
    expect(candidate.albumArtist).toBe("Various Artists");
    expect(candidate.albumArtists).toEqual(["Various Artists"]);
  });

  it("discovers all 51 tracks across 3 discs", async () => {
    const request = await parseAlbumWithTags(albumDir);
    const candidate = candidateFromFolder(request);
    expect(candidate.tracks.length).toBe(51);

    const disc1Tracks = candidate.tracks.filter(t => t.discNumber === 1);
    const disc2Tracks = candidate.tracks.filter(t => t.discNumber === 2);
    const disc3Tracks = candidate.tracks.filter(t => t.discNumber === 3);
    expect(disc1Tracks.length).toBe(15);
    expect(disc2Tracks.length).toBe(18);
    expect(disc3Tracks.length).toBe(18);
  });

  it("splits &-separated Chinese artists on multi-collaboration track", async () => {
    const track2 = path.join(albumDir, "02. Track 2.flac");
    const before = await parseFile(track2);
    expect(before.common.artist).toBe("谢天华&朱永棠&林晓峰");

    const split = splitArtistNames([before.common.artist]);
    expect(split).toEqual(["谢天华", "朱永棠", "林晓峰"]);
  });

  it("splits 3-way collaboration with &", async () => {
    const track4 = path.join(albumDir, "04. Track 4.flac");
    const before = await parseFile(track4);
    expect(before.common.artist).toBe("郑伊健&陈小春&林晓峰");

    const split = splitArtistNames([before.common.artist]);
    expect(split).toEqual(["郑伊健", "陈小春", "林晓峰"]);
  });

  it("writes split ARTISTS via pipeline for multi-artist track", async () => {
    const track2 = path.join(albumDir, "02. Track 2.flac");
    const before = await parseFile(track2);
    expect(before.common.artists).toEqual(["谢天华&朱永棠&林晓峰"]);

    const request = await parseAlbumWithTags(albumDir);
    const candidate = candidateFromFolder(request);

    const tcs = candidate.tracks.filter(t => t.trackNumber === 2 && t.discNumber === 1);
    expect(tcs.length).toBe(1);
    const tc = tcs[0];
    expect(tc.artist).toBe("谢天华&朱永棠&林晓峰");
    expect(tc.artists.length > 0 ? splitArtistNames(tc.artists) : []).toEqual(["谢天华", "朱永棠", "林晓峰"]);
  });

  it("does not have wrong musicbrainz IDs", async () => {
    // All synthetic files have no MBID tags — musicbrainz* should be undefined
    const track1 = path.join(albumDir, "01. Track 1.flac");
    const meta = await parseFile(track1);
    expect((meta as any).common.musicbrainzAlbumId).toBeUndefined();
    expect((meta as any).common.musicbrainzArtistId).toBeUndefined();
    expect((meta as any).common.musicbrainzTrackId).toBeUndefined();
  });
});
