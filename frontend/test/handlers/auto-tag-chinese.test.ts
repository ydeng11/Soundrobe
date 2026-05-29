// @vitest-environment node
import { describe, it, expect } from "vitest";
import { parseFile } from "music-metadata";
import { writeTags } from "../../electron/handlers/writer";
import { parseAlbumWithTags, candidateFromFolder } from "../../electron/handlers/fallback";
import { splitArtistNames } from "../../electron/handlers/candidates";

const TEST_ALBUM = "/tmp/auto-tagger-test/Compilations/1996-古惑仔Ⅲ 只手遮天";

describe("auto-tag pipeline on real Chinese multi-artist track", () => {
  it("splits &-separated TRACK-level ARTIST into 3 separate ARTISTS entries", async () => {
    const track3 = TEST_ALBUM + "/03. 古古惑惑 (清清楚楚我系我).flac";
    const before = await parseFile(track3);
    expect(before.common.artist).toBe("谢天华&朱永棠&林晓峰");
    expect(before.common.artists).toEqual(["谢天华&朱永棠&林晓峰"]);

    const request = await parseAlbumWithTags(TEST_ALBUM);
    const candidate = candidateFromFolder(request);

    const trackMap = new Map<number, any>();
    for (const tc of candidate.tracks) {
      if (tc.trackNumber != null) {
        const fields: any = {};
        if (tc.title !== undefined) fields.title = tc.title;
        if (tc.artist !== undefined) fields.artist = tc.artist;
        if (tc.artists.length > 0) fields.artists = splitArtistNames(tc.artists);
        fields.trackNumber = tc.trackNumber;
        fields.trackTotal = candidate.tracks.length;
        trackMap.set(tc.trackNumber, fields);
      }
    }

    const albumFields: any = {};
    albumFields.artists = candidate.artists.length > 0
      ? candidate.artists
      : splitArtistNames([candidate.artist]);
    if (candidate.album !== undefined) albumFields.album = candidate.album;
    if (candidate.year !== undefined) albumFields.year = candidate.year;

    const tf = trackMap.get(3) ?? {};
    tf.title = "古古惑惑 (清清楚楚我系我)";
    await writeTags(track3, { ...albumFields, ...tf });

    const after = await parseFile(track3);
    expect(after.common.artist).toBe("谢天华&朱永棠&林晓峰");
    expect(after.common.artists).toEqual(["谢天华", "朱永棠", "林晓峰"]);
  });

  it("does not use Compilations as album artist under compilation folder", async () => {
    const request = await parseAlbumWithTags(TEST_ALBUM);
    expect(request.artistHint).toBe("Various Artists");

    const candidate = candidateFromFolder(request);
    expect(candidate.artist).toBe("Various Artists");
    expect(candidate.artists).toEqual(["Various Artists"]);
    expect(candidate.albumArtist).toBe("Various Artists");
    expect(candidate.albumArtists).toEqual(["Various Artists"]);
    expect(candidate.album).toBe("古惑仔Ⅲ 只手遮天");
    expect(candidate.tracks.length).toBe(5);
  });
});

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

describe("candidateFromFolder — compilation handling", () => {
  it("sets Various Artists for a Compilations folder", async () => {
    const { parseAlbumWithTags, candidateFromFolder } = await import("../../electron/handlers/fallback");
    const album = "/tmp/auto-tagger-test/Compilations/1996-古惑仔Ⅲ 只手遮天";
    const request = await parseAlbumWithTags(album);
    const candidate = candidateFromFolder(request);
    expect(candidate.albumArtist).toBe("Various Artists");
    expect(candidate.albumArtists).toEqual(["Various Artists"]);
  });

  it("uses real artist name for a non-compilation folder", async () => {
    // Scan the album normally, but override artistHint to simulate a real artist folder
    const { parseAlbumWithTags, candidateFromFolder } = await import("../../electron/handlers/fallback");
    const album = "/tmp/auto-tagger-test/Compilations/1996-古惑仔Ⅲ 只手遮天";
    const request = await parseAlbumWithTags(album);
    request.artistHint = "郑伊健"; // simulate what would happen under /郑伊健/ folder
    const candidate = candidateFromFolder(request);
    expect(candidate.albumArtist).toBe("郑伊健");
    expect(candidate.albumArtists).toEqual(["郑伊健"]);
  });
});
