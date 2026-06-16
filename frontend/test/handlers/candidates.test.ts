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
  normalizeLookupText,
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
});
