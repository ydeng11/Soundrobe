import { describe, it, expect } from "vitest";
import {
  makeTrackCandidate,
  makeAlbumCandidate,
  makeLookupRequest,
  trackCandidateToJson,
  trackCandidateFromJson,
  albumCandidateToJson,
  albumCandidateFromJson,
  candidatesToJson,
  candidatesFromJson,
  lookupRequestToJson,
  lookupRequestFromJson,
  queryHash,
  ALBUM_TITLE_MATCH_THRESHOLD,
  normalizeLookupText,
  scoreAlbumTitleMatch,
  buildLookupVariantPairs,
  splitArtistNames,
  verifyAlbumName,
  type TrackCandidate,
  type AlbumCandidate,
  type LookupRequest,
} from "../../electron/handlers/candidates";

describe("TrackCandidate", () => {
  it("creates with defaults", () => {
    const t = makeTrackCandidate();
    expect(t.title).toBeNull();
    expect(t.matchTitles).toEqual([]);
    expect(t.artist).toBeNull();
    expect(t.artists).toEqual([]);
    expect(t.trackNumber).toBeNull();
    expect(t.length).toBeNull();
  });

  it("creates with overrides", () => {
    const t = makeTrackCandidate({
      title: "Song One",
      artist: "Artist A",
      trackNumber: 1,
      length: 180.5,
    });
    expect(t.title).toBe("Song One");
    expect(t.artist).toBe("Artist A");
    expect(t.trackNumber).toBe(1);
    expect(t.length).toBe(180.5);
  });

  it("round-trips through JSON", () => {
    const original = makeTrackCandidate({
      title: "Test",
      matchTitles: ["Test recording title"],
      artist: "Tester",
      artists: ["Tester", "Another"],
      trackNumber: 3,
      trackTotal: 10,
      discNumber: 1,
      discTotal: 2,
      musicbrainzTrackId: "mbid-123",
      length: 200,
    });
    const json = trackCandidateToJson(original);
    const restored = trackCandidateFromJson(json);
    expect(restored).toEqual(original);
  });
});

describe("AlbumCandidate", () => {
  it("creates with defaults", () => {
    const c = makeAlbumCandidate();
    expect(c.artist).toBeNull();
    expect(c.album).toBeNull();
    expect(c.tracks).toEqual([]);
    expect(c.source).toBe("beets");
    expect(c.verification).toBeNull();
  });

  it("creates with tracks", () => {
    const c = makeAlbumCandidate({
      artist: "Beatles",
      album: "Abbey Road",
      year: "1969",
      tracks: [makeTrackCandidate({ title: "Come Together", trackNumber: 1 })],
    });
    expect(c.artist).toBe("Beatles");
    expect(c.tracks).toHaveLength(1);
    expect(c.tracks[0].title).toBe("Come Together");
  });

  it("round-trips through JSON", () => {
    const original = makeAlbumCandidate({
      artist: "Artist",
      artists: ["Artist"],
      album: "Album",
      albumArtist: "Artist",
      albumArtists: ["Artist"],
      year: "2020",
      genre: "Rock",
      musicbrainzAlbumId: "mb-album-1",
      musicbrainzArtistId: "mb-artist-1",
      tracks: [makeTrackCandidate({ title: "Track 1", trackNumber: 1 })],
      distance: 0.5,
      source: "musicbrainz",
      verification: "match",
    });
    const json = albumCandidateToJson(original);
    const restored = albumCandidateFromJson(json);
    expect(restored).toEqual(original);
  });

  it("serializes and deserializes a list via JSON string", () => {
    const list = [
      makeAlbumCandidate({ artist: "A", album: "Album 1", source: "musicbrainz" }),
      makeAlbumCandidate({ artist: "B", album: "Album 2", source: "dataset" }),
    ];
    const json = candidatesToJson(list);
    const restored = candidatesFromJson(json);
    expect(restored).toHaveLength(2);
    expect(restored[0].artist).toBe("A");
    expect(restored[1].source).toBe("dataset");
  });
});

describe("LookupRequest", () => {
  it("creates with defaults", () => {
    const r = makeLookupRequest();
    expect(r.path).toBe("");
    expect(r.artistHint).toBeNull();
    expect(r.albumHint).toBeNull();
  });

  it("round-trips through JSON", () => {
    const original = makeLookupRequest({
      path: "/music/Artist/Album",
      artistHint: "Artist",
      albumHint: "Album",
      yearHint: "2020",
      tracks: [makeTrackCandidate({ title: "T1", trackNumber: 1 })],
    });
    const json = lookupRequestToJson(original);
    const restored = lookupRequestFromJson(json);
    expect(restored).toEqual(original);
  });

  it("produces a stable hash", () => {
    const r1 = makeLookupRequest({
      path: "/a",
      artistHint: "X",
      albumHint: "Y",
      tracks: [makeTrackCandidate({ title: "T", trackNumber: 1 })],
    });
    const r2 = makeLookupRequest({
      path: "/b", // path differs but hash excludes path
      artistHint: "X",
      albumHint: "Y",
      tracks: [makeTrackCandidate({ title: "T", trackNumber: 1 })],
    });
    expect(queryHash(r1)).toBe(queryHash(r2));
  });

  it("produces different hashes for different data", () => {
    const r1 = makeLookupRequest({ artistHint: "A", albumHint: "1" });
    const r2 = makeLookupRequest({ artistHint: "B", albumHint: "2" });
    expect(queryHash(r1)).not.toBe(queryHash(r2));
  });
});

describe("normalizeLookupText", () => {
  it("lowercases and strips punctuation", () => {
    expect(normalizeLookupText("Hello, World!")).toBe("hello world");
  });

  it("returns empty string for null", () => {
    expect(normalizeLookupText(null)).toBe("");
  });

  it("preserves CJK characters", () => {
    expect(normalizeLookupText("蔡健雅")).toBe("蔡健雅");
  });

  it("normalizes Unicode fullwidth latin", () => {
    // Fullwidth latin gets NFKC-normalized to ASCII
    expect(normalizeLookupText("ＡＢＣ")).toBe("abc");
  });

  it("strips spaces between CJK characters", () => {
    expect(normalizeLookupText("就是红 光辉全记录")).toBe("就是红光辉全记录");
    expect(normalizeLookupText("七里 香")).toBe("七里香");
    expect(normalizeLookupText("就是紅 光輝全紀錄")).toBe("就是紅光輝全紀錄");
  });

  it("handles consecutive CJK-space-CJK sequences", () => {
    expect(normalizeLookupText("就是红 光 辉全记录")).toBe("就是红光辉全记录");
  });

  it("does not strip spaces between Latin characters", () => {
    expect(normalizeLookupText("Test  Album")).toBe("test album");
  });

  it("converts standalone Roman numerals to Arabic", () => {
    expect(normalizeLookupText("Part II")).toBe("part 2");
    expect(normalizeLookupText("Vol.VI")).toBe("vol 6"); // period becomes space
    expect(normalizeLookupText("III")).toBe("3");
    expect(normalizeLookupText("Chapter IV")).toBe("chapter 4");
  });

  it("does not convert Roman numerals inside words", () => {
    expect(normalizeLookupText("live")).toBe("live");
    expect(normalizeLookupText("civil")).toBe("civil");
    expect(normalizeLookupText("give")).toBe("give");
    expect(normalizeLookupText("active")).toBe("active");
  });

  it("converts Roman numerals adjacent to CJK characters", () => {
    expect(normalizeLookupText("第II章")).toBe("第2章");
    expect(normalizeLookupText("Vol.II精选")).toBe("vol 2精选"); // period becomes space
  });

  it("strips diacritical marks", () => {
    expect(normalizeLookupText("Café")).toBe("cafe");
    expect(normalizeLookupText("naïve")).toBe("naive");
    expect(normalizeLookupText("über")).toBe("uber");
  });

  it("normalizes fullwidth digits via NFKD", () => {
    expect(normalizeLookupText("０１２")).toBe("012");
  });
});

describe("scoreAlbumTitleMatch", () => {
  it("matches Simplified and Traditional Chinese album titles", async () => {
    const result = await scoreAlbumTitleMatch("到底有谁能够告诉我", "到底有誰能夠告訴我");
    expect(result.score).toBeGreaterThanOrEqual(100);
    expect(result.reason).toBe("exact");
  });

  it("matches Japanese shinjitai embedded in a Chinese provider title", async () => {
    const result = await scoreAlbumTitleMatch("一个任贤齐", "一個任贤斉");

    expect(result.score).toBeGreaterThanOrEqual(100);
    expect(result.reason).toBe("exact");
  });

  it("matches gendered Chinese pronouns before testing album containment", async () => {
    const result = await scoreAlbumTitleMatch(
      "Need U Most 最需要你 K歌情人",
      "Need U Most（最需要妳）",
    );

    expect(result.reason).toBe("local-contains-remote");
    expect(result.score).toBeGreaterThanOrEqual(70);
  });

  it("strips ellipsis and fullwidth punctuation before matching", async () => {
    const result = await scoreAlbumTitleMatch("Goodbye Hello", "Goodbye…Hello！");
    expect(result.score).toBeGreaterThanOrEqual(100);
  });

  it("rejects short CJK containment matches", async () => {
    const result = await scoreAlbumTitleMatch("爱", "爱在深秋");
    expect(result.score).toBe(0);
  });

  it("adds year bonus to accepted title matches", async () => {
    const result = await scoreAlbumTitleMatch("幻象波普星", "幻象波普星", {
      localYear: "2013",
      remoteYear: 2013,
    });
    expect(result.score).toBe(110);
  });

  it("matches CJK title with space against title without space (regression: 就是红 光辉全记录)", async () => {
    // CJK space stripped + fuzzy match handles 记/纪 variant pair
    const result = await scoreAlbumTitleMatch("就是红 光辉全记录", "就是紅光輝全紀錄");
    expect(result.score).toBeGreaterThanOrEqual(75);
  });

  it("matches fuzzy CJK variants (记 vs 紀)", async () => {
    const result = await scoreAlbumTitleMatch("就是红光辉全记录", "就是紅光輝全紀錄");
    expect(result.score).toBeGreaterThanOrEqual(75);
    expect(result.reason).toBe("fuzzy");
  });

  it("does not accept Latin title typos through fuzzy matching", async () => {
    const result = await scoreAlbumTitleMatch("The Dark Side of the Moon", "The Dark Side of the Noon");
    expect(result.score).toBeLessThan(ALBUM_TITLE_MATCH_THRESHOLD);
    expect(result.reason).toBe("none");
  });
});

describe("splitArtistNames", () => {
  it("splits common collaboration separators", () => {
    expect(splitArtistNames(["Alice & Bob / Carol feat. Dave; Eve, Frank"])).toEqual([
      "Alice",
      "Bob",
      "Carol",
      "Dave",
      "Eve",
      "Frank",
    ]);
  });

  it("splits ampersand separator between Chinese artist names", () => {
    expect(splitArtistNames(["谢天华&朱永棠&林晓峰"])).toEqual([
      "谢天华",
      "朱永棠",
      "林晓峰",
    ]);
  });

  it("splits semicolon separator between Chinese artist names", () => {
    expect(splitArtistNames(["谢天华;朱永棠;林晓峰"])).toEqual([
      "谢天华",
      "朱永棠",
      "林晓峰",
    ]);
  });

  it("splits CJK dot and punctuation separators", () => {
    expect(splitArtistNames(["陈慧琳.陈小春、郑伊健；许志安"])).toEqual([
      "陈慧琳",
      "陈小春",
      "郑伊健",
      "许志安",
    ]);
  });
});

describe("buildLookupVariantPairs", () => {
  it("queries simplified, traditional, then original text", () => {
    expect(buildLookupVariantPairs("张学友", "吻别")).toEqual([
      ["张学友", "吻别"],
      ["張學友", "吻別"],
    ]);
    expect(buildLookupVariantPairs("張學友", "吻別")).toEqual([
      ["张学友", "吻别"],
      ["張學友", "吻別"],
    ]);
  });
});

describe("verifyAlbumName", () => {
  it("returns match when identical", async () => {
    const c = makeAlbumCandidate({ album: "Abbey Road" });
    expect(await verifyAlbumName("Abbey Road", c)).toBe("match");
  });

  it("returns match when null hint", async () => {
    const c = makeAlbumCandidate({ album: "Something" });
    expect(await verifyAlbumName(null, c)).toBe("match");
  });

  it("returns close for substring", async () => {
    const c = makeAlbumCandidate({ album: "The Dark Side of the Moon" });
    expect(await verifyAlbumName("Dark Side", c)).toBe("close");
  });

  it("returns mismatch for different", async () => {
    const c = makeAlbumCandidate({ album: "Revolver" });
    expect(await verifyAlbumName("Abbey Road", c)).toBe("mismatch");
  });

  it("returns close for fuzzy CJK match (记 vs 紀 variant)", async () => {
    const c = makeAlbumCandidate({ album: "就是紅光輝全紀錄" });
    expect(await verifyAlbumName("就是红 光辉全记录", c)).toBe("close");
  });

  it("does not return close for Latin title typos through fuzzy matching", async () => {
    const c = makeAlbumCandidate({ album: "The Dark Side of the Noon" });
    expect(await verifyAlbumName("The Dark Side of the Moon", c)).toBe("mismatch");
  });
});
