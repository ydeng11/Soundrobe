import { describe, it, expect, beforeEach, afterEach } from "vitest";
import fs from "fs";
import path from "path";
import os from "os";

// Import pure functions directly
import {
  isAudioFile,
  parseArtistAlbumHint,
  scanDirectory,
  collectAudioFiles,
} from "../../electron/handlers/library";

describe("isAudioFile", () => {
  it("returns true for supported audio extensions", () => {
    expect(isAudioFile("/path/to/song.mp3")).toBe(true);
    expect(isAudioFile("/path/to/song.flac")).toBe(true);
    expect(isAudioFile("/path/to/song.m4a")).toBe(true);
    expect(isAudioFile("/path/to/song.wav")).toBe(true);
    expect(isAudioFile("/path/to/song.ogg")).toBe(true);
    expect(isAudioFile("/path/to/song.opus")).toBe(true);
    expect(isAudioFile("/path/to/song.aiff")).toBe(true);
  });

  it("returns false for non-audio extensions", () => {
    expect(isAudioFile("/path/to/song.txt")).toBe(false);
    expect(isAudioFile("/path/to/song.jpg")).toBe(false);
    expect(isAudioFile("/path/to/song.png")).toBe(false);
    expect(isAudioFile("/path/to/song.lrc")).toBe(false);
    expect(isAudioFile("/path/to/song")).toBe(false);
  });

  it("is case-insensitive", () => {
    expect(isAudioFile("/path/to/song.MP3")).toBe(true);
    expect(isAudioFile("/path/to/song.FLAC")).toBe(true);
    expect(isAudioFile("/path/to/song.M4A")).toBe(true);
  });
});

describe("parseArtistAlbumHint", () => {
  it("parses standard Artist/Album hierarchy", () => {
    const result = parseArtistAlbumHint(
      "/music/Artist Name/Album Name",
      "Artist Name"
    );
    expect(result.artistHint).toBe("Artist Name");
    expect(result.albumHint).toBe("Album Name");
  });

  it("parses flat 'Artist - Album' pattern", () => {
    const result = parseArtistAlbumHint(
      "/music/Radiohead - OK Computer",
      "music"
    );
    expect(result.artistHint).toBe("Radiohead");
    expect(result.albumHint).toBe("OK Computer");
  });

  it("handles single-album directory without artist parent", () => {
    const result = parseArtistAlbumHint("/music/Some Album", "");
    expect(result.artistHint).toBe("");
    expect(result.albumHint).toBe("Some Album");
  });

  it("handles 'Year - Album' with explicit artist parent", () => {
    const result = parseArtistAlbumHint(
      "/music/The Beatles/1969 - Abbey Road",
      "The Beatles"
    );
    // Dash matches but "1969" looks like a year — use parent dir
    expect(result.artistHint).toBe("The Beatles");
    expect(result.albumHint).toBe("1969 - Abbey Road");
  });
});

describe("collectAudioFiles", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "auto-tagger-test-"));
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  it("returns sorted audio files from a directory", () => {
    fs.writeFileSync(path.join(tmpDir, "01 song.mp3"), "data");
    fs.writeFileSync(path.join(tmpDir, "02 song.flac"), "data");
    fs.writeFileSync(path.join(tmpDir, "cover.jpg"), "data");
    fs.writeFileSync(path.join(tmpDir, "info.txt"), "data");

    const result = collectAudioFiles(tmpDir);
    expect(result).toHaveLength(2);
    expect(result[0]).toContain("01 song.mp3");
    expect(result[1]).toContain("02 song.flac");
  });

  it("ignores hidden entries", () => {
    fs.writeFileSync(path.join(tmpDir, ".hidden.mp3"), "data");
    fs.writeFileSync(path.join(tmpDir, "visible.mp3"), "data");

    const result = collectAudioFiles(tmpDir);
    expect(result).toHaveLength(1);
    expect(result[0]).toContain("visible.mp3");
  });

  it("returns empty array for empty directory", () => {
    const result = collectAudioFiles(tmpDir);
    expect(result).toHaveLength(0);
  });
});

describe("scanDirectory", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "auto-tagger-scan-"));
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  it("groups audio files by album directory", () => {
    const albumDir = path.join(tmpDir, "Artist", "Album Name");
    fs.mkdirSync(albumDir, { recursive: true });
    fs.writeFileSync(path.join(albumDir, "01 track.mp3"), "data");
    fs.writeFileSync(path.join(albumDir, "02 track.flac"), "data");

    const result = scanDirectory(tmpDir);
    expect(result.albums.size).toBe(1);
    const album = result.albums.get(albumDir);
    expect(album).toBeDefined();
    expect(album!.name).toBe("Album Name");
    expect(album!.artistHint).toBe("Artist");
    expect(album!.trackCount).toBe(2);
  });

  it("returns multiple albums from the library", () => {
    const album1 = path.join(tmpDir, "Artist1", "Album1");
    const album2 = path.join(tmpDir, "Artist2", "Album2");
    fs.mkdirSync(album1, { recursive: true });
    fs.mkdirSync(album2, { recursive: true });
    fs.writeFileSync(path.join(album1, "track.mp3"), "data");
    fs.writeFileSync(path.join(album2, "track.mp3"), "data");

    const result = scanDirectory(tmpDir);
    expect(result.albums.size).toBe(2);
  });

  it("handles single file input", () => {
    const songPath = path.join(tmpDir, "song.mp3");
    fs.writeFileSync(songPath, "data");

    const result = scanDirectory(songPath);
    expect(result.albums.size).toBe(1);
  });

  it("groups root-level audio files as one album", () => {
    fs.writeFileSync(path.join(tmpDir, "01 track.mp3"), "data");
    fs.writeFileSync(path.join(tmpDir, "02 track.flac"), "data");

    const result = scanDirectory(tmpDir);
    expect(result.albums.size).toBe(1);
    const album = result.albums.get(tmpDir);
    expect(album).toBeDefined();
    expect(album!.trackCount).toBe(2);
  });

  it("throws on non-existent path", () => {
    expect(() => scanDirectory("/nonexistent/path")).toThrow(
      "Library path not found"
    );
  });
});

/**
 * Differential parity baseline: Electron generates this exact normalized
 * library:scan response from the committed fixture tree. Rust's matching test
 * consumes the same expected.json after running `scan_directory`, so neither
 * runtime can silently drift in shape, grouping, or normalized path order.
 */
describe("library:scan shared Electron/Rust fixture", () => {
  const fixtureRoot = path.resolve(
    process.cwd(),
    "test/fixtures/tauri/library-scan",
  );

  function normalize(
    albums: Array<{
      path: string;
      name: string;
      artistHint: string;
      albumHint: string;
      trackCount: number;
    }>,
  ) {
    return albums
      .map((album) => ({
        ...album,
        path: path.relative(fixtureRoot, album.path) || ".",
      }))
      .sort((a, b) => a.path.localeCompare(b.path));
  }

  it("matches the committed normalized baseline consumed by Rust", () => {
    const expected = JSON.parse(
      fs.readFileSync(path.join(fixtureRoot, "expected.json"), "utf-8"),
    );
    const actual = normalize([...scanDirectory(fixtureRoot).albums.values()]);
    expect(actual).toEqual(expected);
  });
});
