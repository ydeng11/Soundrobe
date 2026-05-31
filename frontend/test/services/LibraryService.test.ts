import { describe, it, expect } from "vitest";
import { LibraryService } from "../../electron/services/LibraryService";
import type { TrackData, AlbumInfo } from "../../electron/preload";

function makeTrack(overrides: Partial<TrackData> = {}): TrackData {
  return {
    path: "/test/album/track1.flac",
    title: "Test Track",
    artist: "Test Artist",
    artists: ["Test Artist"],
    album: "Test Album",
    albumArtist: null,
    albumArtists: [],
    trackNumber: 1,
    trackTotal: 10,
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
    sizeBytes: 5000000,
    bitrate: 320,
    sampleRate: 44100,
    codec: "FLAC",
    duration: 180,
    ...overrides,
  };
}

function makeAlbum(overrides: Partial<AlbumInfo> = {}): AlbumInfo {
  return {
    path: "/test/album",
    name: "Test Album",
    artistHint: "Test Artist",
    albumHint: "Test Album",
    trackCount: 1,
    ...overrides,
  };
}

describe("LibraryService", () => {
  it("summarizes library correctly", () => {
    const service = new LibraryService();
    const tracks = [
      makeTrack({ path: "/a/1.flac", title: "A", artist: "X", album: "Album1", genre: "Rock", year: "2024", codec: "FLAC", sizeBytes: 1000, duration: 100 }),
      makeTrack({ path: "/a/2.flac", title: "B", artist: "Y", album: "Album2", genre: "Pop", year: "2023", codec: "FLAC", sizeBytes: 2000, duration: 200 }),
      makeTrack({ path: "/a/3.mp3", title: "C", artist: null, album: "Album1", genre: "Rock", year: "2024", codec: "MP3", sizeBytes: 3000, duration: 150 }),
    ];
    const albums = [
      makeAlbum({ path: "/a", name: "A", artistHint: "X", trackCount: 2 }),
      makeAlbum({ path: "/b", name: "B", artistHint: "Y", trackCount: 1 }),
    ];

    const summary = service.summarizeLibrary(albums, tracks);

    expect(summary.albumCount).toBe(2);
    expect(summary.trackCount).toBe(3);
    expect(summary.totalSizeBytes).toBe(6000);
    expect(summary.totalDurationSeconds).toBe(450);
    expect(summary.artistCount).toBe(2); // X and Y
    expect(summary.genreCount).toBe(2); // Rock and Pop
    expect(summary.missingArtist).toBe(1);
    expect(summary.byCodec).toEqual({ FLAC: 2, MP3: 1 });
  });

  it("summarizes library with zero tracks", () => {
    const service = new LibraryService();
    const summary = service.summarizeLibrary([], []);
    expect(summary.albumCount).toBe(0);
    expect(summary.trackCount).toBe(0);
    expect(summary.artistCount).toBe(0);
    expect(summary.missingTitle).toBe(0);
  });

  it("builds app context with selection and active album", () => {
    const service = new LibraryService();
    const tracks = [
      makeTrack({ path: "/a/1.flac", title: "A", artist: "X", album: "Album1" }),
      makeTrack({ path: "/a/2.flac", title: "B", artist: "Y", album: "Album1" }),
    ];
    const albums = [makeAlbum({ path: "/a", name: "Album1", trackCount: 2 })];

    const ctx = service.buildAppContext({
      libraryPath: "/test",
      activeAlbumPath: "/a",
      selectedTrackPaths: ["/a/1.flac"],
      tracks,
      albums,
      assistantAutonomous: false,
    });

    expect(ctx.libraryPath).toBe("/test");
    expect(ctx.activeAlbumPath).toBe("/a");
    expect(ctx.selectedTrackPaths).toHaveLength(1);
    expect(ctx.selectedTrackSummaries).toHaveLength(1);
    expect(ctx.selectedTrackSummaries[0].title).toBe("A");
    expect(ctx.activeAlbumSummary).not.toBeNull();
    expect(ctx.activeAlbumSummary!.name).toBe("Album1");
    expect(ctx.activeAlbumSummary!.trackCount).toBe(2);
  });

  it("does not include active album path-prefix siblings in app context", () => {
    const service = new LibraryService();
    const tracks = [
      makeTrack({ path: "/a/1.flac", title: "A", album: "Album1" }),
      makeTrack({ path: "/a deluxe/2.flac", title: "B", album: "Album2" }),
    ];
    const albums = [makeAlbum({ path: "/a", name: "Album1", trackCount: 1 })];

    const ctx = service.buildAppContext({
      libraryPath: "/test",
      activeAlbumPath: "/a",
      selectedTrackPaths: [],
      tracks,
      albums,
      assistantAutonomous: false,
    });

    expect(ctx.activeAlbumSummary!.trackCount).toBe(1);
  });

  it("builds app context with empty library", () => {
    const service = new LibraryService();
    const ctx = service.buildAppContext({
      libraryPath: null,
      activeAlbumPath: null,
      selectedTrackPaths: [],
      tracks: [],
      albums: [],
      assistantAutonomous: false,
    });
    expect(ctx.libraryPath).toBeNull();
    expect(ctx.selectedTrackSummaries).toHaveLength(0);
    expect(ctx.activeAlbumSummary).toBeNull();
  });

  it("assertInsideLibrary rejects paths outside the library", () => {
    const service = new LibraryService();
    service.setLibraryPath("/lib");
    expect(() => service.assertInsideLibrary("/other/file.txt")).toThrow(
      "outside the current library",
    );
  });

  it("assertInsideLibrary accepts paths inside the library", () => {
    const service = new LibraryService();
    service.setLibraryPath("/lib");
    expect(service.assertInsideLibrary("/lib/album/track.flac")).toBe(
      "/lib/album/track.flac",
    );
  });

  it("assertInsideLibrary throws when library is not set", () => {
    const service = new LibraryService();
    expect(() => service.assertInsideLibrary("/any/path")).toThrow(
      "No library selected",
    );
  });
});
