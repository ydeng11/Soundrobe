import { describe, it, expect } from "vitest";
import { SafeQueryService } from "../../electron/services/SafeQueryService";
import type { TrackData } from "../../electron/preload";

function makeTrack(overrides: Partial<TrackData> & { path: string }): TrackData {
  return {
    title: "Test",
    artist: "Test Artist",
    artists: ["Test Artist"],
    album: "Test Album",
    albumArtist: null,
    albumArtists: [],
    trackNumber: 1,
    trackTotal: null,
    discNumber: null,
    discTotal: null,
    year: "2024",
    genre: "Rock",
    composer: null,
    comment: null,
    lyrics: null,
    compilation: null,
    musicbrainzTrackId: null,
    musicbrainzAlbumId: null,
    musicbrainzArtistId: null,
    hasCover: false,
    sizeBytes: 0,
    bitrate: null,
    sampleRate: null,
    codec: "FLAC",
    duration: 0,
    ...overrides,
  };
}

describe("SafeQueryService", () => {
  describe("findTracks", () => {
    it("filters by title substring", () => {
      const q = new SafeQueryService();
      q.setTracks([
        makeTrack({ path: "/a", title: "Hello World" }),
        makeTrack({ path: "/b", title: "Goodbye" }),
      ]);
      const results = q.findTracks({ title: "Hello" });
      expect(results).toHaveLength(1);
      expect(results[0].path).toBe("/a");
    });

    it("filters by artist substring", () => {
      const q = new SafeQueryService();
      q.setTracks([
        makeTrack({ path: "/a", artist: "Radiohead" }),
        makeTrack({ path: "/b", artist: "The Beatles" }),
      ]);
      const results = q.findTracks({ artist: "Radio" });
      expect(results).toHaveLength(1);
    });

    it("filters by album substring", () => {
      const q = new SafeQueryService();
      q.setTracks([
        makeTrack({ path: "/a", album: "OK Computer" }),
        makeTrack({ path: "/b", album: "Abbey Road" }),
      ]);
      const results = q.findTracks({ album: "Computer" });
      expect(results).toHaveLength(1);
    });

    it("finds tracks missing title", () => {
      const q = new SafeQueryService();
      q.setTracks([
        makeTrack({ path: "/a", title: null }),
        makeTrack({ path: "/b", title: "Has Title" }),
      ]);
      const results = q.findTracks({ missingTitle: true });
      expect(results).toHaveLength(1);
      expect(results[0].path).toBe("/a");
    });

    it("finds tracks missing artist", () => {
      const q = new SafeQueryService();
      q.setTracks([
        makeTrack({ path: "/a", artist: null }),
        makeTrack({ path: "/b", artist: "Artist" }),
      ]);
      const results = q.findTracks({ missingArtist: true });
      expect(results).toHaveLength(1);
      expect(results[0].path).toBe("/a");
    });

    it("finds tracks missing album", () => {
      const q = new SafeQueryService();
      q.setTracks([
        makeTrack({ path: "/a", album: null }),
        makeTrack({ path: "/b", album: "Album" }),
      ]);
      const results = q.findTracks({ missingAlbum: true });
      expect(results).toHaveLength(1);
    });

    it("finds tracks missing year", () => {
      const q = new SafeQueryService();
      q.setTracks([
        makeTrack({ path: "/a", year: null }),
        makeTrack({ path: "/b", year: "2024" }),
      ]);
      const results = q.findTracks({ missingYear: true });
      expect(results).toHaveLength(1);
    });

    it("finds tracks missing genre", () => {
      const q = new SafeQueryService();
      q.setTracks([
        makeTrack({ path: "/a", genre: null }),
        makeTrack({ path: "/b", genre: "Rock" }),
      ]);
      const results = q.findTracks({ missingGenre: true });
      expect(results).toHaveLength(1);
    });

    it("finds tracks with missing cover", () => {
      const q = new SafeQueryService();
      q.setTracks([
        makeTrack({ path: "/a", hasCover: false }),
        makeTrack({ path: "/b", hasCover: true }),
      ]);
      const results = q.findTracks({ missingCover: true });
      expect(results).toHaveLength(1);
      expect(results[0].path).toBe("/a");
    });

    it("finds duplicates by title+artist+album", () => {
      const q = new SafeQueryService();
      q.setTracks([
        makeTrack({ path: "/a", title: "Song", artist: "A", album: "X" }),
        makeTrack({ path: "/b", title: "Song", artist: "A", album: "X" }),
        makeTrack({ path: "/c", title: "Other", artist: "B", album: "Y" }),
      ]);
      const results = q.findTracks({ hasDuplicates: true });
      expect(results).toHaveLength(2);
    });

    it("does not classify tracks with unknown identity tags as duplicates", () => {
      const q = new SafeQueryService();
      q.setTracks([
        makeTrack({ path: "/a", title: null, artist: null, album: null }),
        makeTrack({ path: "/b", title: null, artist: null, album: null }),
      ]);

      const results = q.findTracks({ hasDuplicates: true });

      expect(results).toHaveLength(0);
    });

    it("filters by codec", () => {
      const q = new SafeQueryService();
      q.setTracks([
        makeTrack({ path: "/a", codec: "FLAC" }),
        makeTrack({ path: "/b", codec: "MP3" }),
      ]);
      const results = q.findTracks({ codec: "mp3" });
      expect(results).toHaveLength(1);
    });

    it("returns empty for no matches", () => {
      const q = new SafeQueryService();
      q.setTracks([makeTrack({ path: "/a", title: "Hello" })]);
      const results = q.findTracks({ title: "Nonexistent" });
      expect(results).toHaveLength(0);
    });
  });

  describe("aggregate", () => {
    it("computes aggregate counts", () => {
      const q = new SafeQueryService();
      q.setTracks([
        makeTrack({ path: "/a", title: "A", artist: "X", album: "Album1", genre: "Rock", year: "2024", codec: "FLAC" }),
        makeTrack({ path: "/b", title: "B", artist: "Y", album: "Album2", genre: "Pop", year: "2023", codec: "FLAC" }),
        makeTrack({ path: "/c", title: "C", artist: "X", album: "Album1", genre: "Rock", year: "2024", codec: "MP3" }),
      ]);

      const agg = q.aggregate();
      expect(agg.totalTracks).toBe(3);
      expect(agg.totalAlbums).toBe(2);
      expect(agg.totalArtists).toBe(2); // X and Y
      expect(agg.totalGenres).toBe(2); // Rock and Pop
      expect(agg.byCodec).toEqual({ FLAC: 2, MP3: 1 });
      expect(agg.byAlbum).toEqual({ Album1: 2, Album2: 1 });
    });

    it("handles empty track list", () => {
      const q = new SafeQueryService();
      q.setTracks([]);
      const agg = q.aggregate();
      expect(agg.totalTracks).toBe(0);
      expect(agg.totalAlbums).toBe(0);
      expect(agg.totalArtists).toBe(0);
    });

    it("computes tag completeness percentages", () => {
      const q = new SafeQueryService();
      q.setTracks([
        makeTrack({ path: "/a", title: "A", artist: "X", album: "Album", year: "2024", genre: null }),
        makeTrack({ path: "/b", title: null, artist: "Y", album: "Album", year: null, genre: "Rock" }),
      ]);
      const agg = q.aggregate();
      expect(agg.tagCompleteness.title).toBe(50);
      expect(agg.tagCompleteness.genre).toBe(50);
    });

    it("ignores whitespace-only metadata when computing aggregate identities", () => {
      const q = new SafeQueryService();
      q.setTracks([
        makeTrack({ path: "/a", album: " ", artist: " ", albumArtist: "\t", genre: "", year: "  " }),
        makeTrack({ path: "/b", album: "Album", artist: "Artist", albumArtist: "Album Artist", genre: "Rock", year: "2024" }),
      ]);

      const agg = q.aggregate();

      expect(agg.totalAlbums).toBe(1);
      expect(agg.totalArtists).toBe(2);
      expect(agg.totalGenres).toBe(1);
      expect(agg.byAlbum).toEqual({ Album: 1 });
      expect(agg.byArtist).toEqual({ Artist: 1 });
      expect(agg.byGenre).toEqual({ Rock: 1 });
      expect(agg.byYear).toEqual({ "2024": 1 });
    });
  });
});
