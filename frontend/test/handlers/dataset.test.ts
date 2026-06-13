import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { mkdtempSync, rmSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { createRequire } from "node:module";
import { DatasetReader } from "../../electron/handlers/dataset";

let tmpDir: string;
let dbPath: string;

type NativeDatabaseConstructor = new (path: string) => {
  pragma(sql: string): unknown;
  exec(sql: string): void;
  prepare(sql: string): { run(...params: unknown[]): unknown };
  close(): void;
};

function tryLoadNativeDatabase(): NativeDatabaseConstructor | null {
  try {
    const Database = createRequire(import.meta.url)("better-sqlite3") as NativeDatabaseConstructor;
    const probe = new Database(":memory:");
    probe.close();
    return Database;
  } catch {
    return null;
  }
}

const Database = tryLoadNativeDatabase();
const describeDatasetReader = Database ? describe : describe.skip;

function createFixtureDb(): void {
  if (!Database) {
    throw new Error("better-sqlite3 is not available under the shell Node ABI");
  }
  const db = new Database(dbPath);
  db.pragma("journal_mode = WAL");

  // Create dataset_lookup table
  db.exec(`
    CREATE TABLE IF NOT EXISTS dataset_lookup (
      service TEXT NOT NULL,
      artist TEXT NOT NULL,
      album TEXT NOT NULL,
      year TEXT,
      album_id TEXT NOT NULL,
      artist_id TEXT NOT NULL,
      normalized_artist TEXT NOT NULL,
      normalized_album TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS musicbrainz_release_track (
      releasetrackid TEXT NOT NULL,
      releaseid TEXT NOT NULL,
      title TEXT,
      recordingtitle TEXT,
      number INTEGER,
      position INTEGER,
      mediaposition INTEGER,
      mediatrackcount INTEGER,
      length INTEGER,
      recordingid TEXT
    );
    CREATE TABLE IF NOT EXISTS musicbrainz_artist (
      artistid TEXT NOT NULL,
      name TEXT
    );
    CREATE TABLE IF NOT EXISTS musicbrainz_release_track_artist (
      releasetrackid TEXT NOT NULL,
      artistid TEXT NOT NULL,
      joinphrase TEXT NOT NULL,
      "index" INTEGER NOT NULL
    );
    CREATE TABLE IF NOT EXISTS spotify_track (
      albumid TEXT NOT NULL,
      name TEXT,
      tracknumber INTEGER,
      discnumber INTEGER,
      durationms INTEGER
    );
  `);

  const insertLookup = db.prepare(`
    INSERT INTO dataset_lookup (service, artist, album, year, album_id, artist_id, normalized_artist, normalized_album)
    VALUES (?, ?, ?, ?, ?, ?, ?, ?)
  `);

  const insertTrack = db.prepare(`
    INSERT INTO musicbrainz_release_track (releasetrackid, releaseid, title, number, mediaposition, mediatrackcount, length, recordingid)
    VALUES (?, ?, ?, ?, ?, ?, ?, ?)
  `);
  const insertArtist = db.prepare(`
    INSERT INTO musicbrainz_artist (artistid, name) VALUES (?, ?)
  `);
  const insertTrackArtist = db.prepare(`
    INSERT INTO musicbrainz_release_track_artist (releasetrackid, artistid, joinphrase, "index")
    VALUES (?, ?, ?, ?)
  `);

  // Beatles - Abbey Road (musicbrainz)
  insertLookup.run(
    "musicbrainz", "The Beatles", "Abbey Road", "1969",
    "mb-abbey-1", "mb-beatles-1",
    "the beatles", "abbey road",
  );
  insertTrack.run("rt-1", "mb-abbey-1", "Come Together", 1, 1, 17, 259000, "mb-track-1");
  insertTrack.run("rt-2", "mb-abbey-1", "Something", 2, 1, 17, 182000, "mb-track-2");
  insertArtist.run("mb-lennon", "John Lennon");
  insertArtist.run("mb-mccartney", "Paul McCartney");
  insertTrackArtist.run("rt-1", "mb-lennon", " & ", 0);
  insertTrackArtist.run("rt-1", "mb-mccartney", "", 1);

  // Beatles - Sgt. Pepper (second album for artist-only fallback)
  insertLookup.run(
    "musicbrainz", "The Beatles", "Sgt. Pepper", "1967",
    "mb-pepper-1", "mb-beatles-1",
    "the beatles", "sgt pepper",
  );
  insertTrack.run("rt-p-1", "mb-pepper-1", "Sgt. Peppers Lonely Hearts Club Band", 1, 1, 13, 120000, "mb-tp-1");

  // Spotify entry with same album name as MB entry (for cross-service dedup test)
  insertLookup.run(
    "spotify", "The Beatles", "Abbey Road", "2019",
    "sp-abbey-1", "sp-beatles-1",
    "the beatles", "abbey road",
  );
  // Insert spotify tracks so loadTracks succeeds for spotify entries
  const insertSpotifyTrack = db.prepare(`
    INSERT INTO spotify_track (albumid, name, tracknumber, discnumber, durationms)
    VALUES (?, ?, ?, ?, ?)
  `);
  insertSpotifyTrack.run("sp-abbey-1", "Come Together", 1, 1, 259000);
  insertSpotifyTrack.run("sp-abbey-1", "Something", 2, 1, 182000);

  // 蔡健雅 - 达尔文 (simpler album name without punctuation)
  insertLookup.run(
    "musicbrainz", "蔡健雅", "达尔文", "2007",
    "mb-darwin-1", "mb-tanya-1",
    "蔡健雅", "达尔文",
  );
  insertTrack.run("rt-3", "mb-darwin-1", "达尔文", 1, 1, 12, 245000, "mb-gt-1");
  insertTrack.run("rt-4", "mb-darwin-1", "空白格", 2, 1, 12, 234000, "mb-gt-2");

  // 陈洁仪 - 心碎
  insertLookup.run(
    "musicbrainz", "陈洁仪", "心碎", "1994",
    "mb-xinsui-1", "mb-chenjieyi-1",
    "陈洁仪", "心碎",
  );

  // 蛋堡 - Winter Sweet (for artist hint cleanup test)
  insertLookup.run(
    "musicbrainz", "蛋堡", "Winter Sweet", "2019",
    "mb-danbao-1", "mb-danbao-id",
    "蛋堡", "winter sweet",
  );
  insertTrack.run("rt-db-1", "mb-danbao-1", "Soft Lintro", 1, 1, 15, 240000, "mb-tdb-1");

  // Progressive prefix test: folder name is superset of DB album name
  insertLookup.run(
    "musicbrainz", "Various Artists", "T-time",
    "2000", "mb-ttime-1", "mb-various-1",
    "various artists", "t time",
  );

  db.close();
}

beforeEach(() => {
  tmpDir = mkdtempSync(join(tmpdir(), "dataset-test-"));
  dbPath = join(tmpDir, "dataset-index.sqlite");
  createFixtureDb();
});

afterEach(() => {
  rmSync(tmpDir, { recursive: true, force: true });
});

describeDatasetReader("DatasetReader — setup", () => {
  it("detects available database", () => {
    const reader = new DatasetReader(dbPath);
    expect(reader.isAvailable()).toBe(true);
    reader.close();
  });

  it("detects unavailable database", () => {
    const reader = new DatasetReader("/nonexistent/path.sqlite");
    expect(reader.isAvailable()).toBe(false);
    reader.close();
  });

  it("detects lookup table", () => {
    const reader = new DatasetReader(dbPath);
    expect(reader.hasLookupTable()).toBe(true);
    reader.close();
  });

  it("reports status", () => {
    const reader = new DatasetReader(dbPath);
    const status = reader.getStatus();
    expect(status.available).toBe(true);
    expect(status.musicbrainz).toBe(true);
    expect(status.totalRecords).toBeGreaterThan(0);
    reader.close();
  });
});

describeDatasetReader("DatasetReader — queries", () => {
  it("returns results for exact match with musicbrainz IDs", () => {
    const reader = new DatasetReader(dbPath);
    const results = reader.queryAlbum("The Beatles", "Abbey Road");
    expect(results).toHaveLength(1);
    expect(results[0].artist).toBe("The Beatles");
    expect(results[0].album).toBe("Abbey Road");
    expect(results[0].year).toBe("1969");
    expect(results[0].source).toBe("dataset");
    expect(results[0].musicbrainzAlbumId).toBe("mb-abbey-1");
    expect(results[0].musicbrainzArtistId).toBe("mb-beatles-1");
    expect(results[0].discogsReleaseId).toBeNull();
    expect(results[0].discogsArtistId).toBeNull();
    reader.close();
  });

  it("returns tracks with album", () => {
    const reader = new DatasetReader(dbPath);
    const results = reader.queryAlbum("The Beatles", "Abbey Road");
    expect(results[0].tracks).toHaveLength(2);
    expect(results[0].tracks[0].title).toBe("Come Together");
    expect(results[0].tracks[0].trackNumber).toBe(1);
    expect(results[0].tracks[1].title).toBe("Something");
    reader.close();
  });

  it("uses MusicBrainz track artist credits when join tables are present", () => {
    const reader = new DatasetReader(dbPath);
    const results = reader.queryAlbum("The Beatles", "Abbey Road");
    expect(results[0].tracks[0].artist).toBe("John Lennon & Paul McCartney");
    expect(results[0].tracks[0].artists).toEqual(["John Lennon", "Paul McCartney"]);
    reader.close();
  });

  it("returns results for Chinese artist", () => {
    const reader = new DatasetReader(dbPath);
    const results = reader.queryAlbum("蔡健雅", "达尔文");
    expect(results).toHaveLength(1);
    expect(results[0].artist).toBe("蔡健雅");
    expect(results[0].album).toBe("达尔文");
    reader.close();
  });

  it("is case-insensitive", () => {
    const reader = new DatasetReader(dbPath);
    const results = reader.queryAlbum("the beatles", "abbey road");
    expect(results).toHaveLength(1);
    expect(results[0].artist).toBe("The Beatles");
    reader.close();
  });

  it("handles no matches gracefully", () => {
    const reader = new DatasetReader(dbPath);
    const results = reader.queryAlbum("Nonexistent", "Unknown Album");
    expect(results).toHaveLength(0);
    reader.close();
  });

  it("handles empty hints", () => {
    const reader = new DatasetReader(dbPath);
    expect(reader.queryAlbum("", "")).toHaveLength(0);
    expect(reader.queryAlbum("Artist", "")).toHaveLength(0);
    expect(reader.queryAlbum("", "Album")).toHaveLength(0);
    reader.close();
  });

  it("returns empty when database doesn't exist", () => {
    const reader = new DatasetReader("/nonexistent/path.sqlite");
    expect(reader.queryAlbum("The Beatles", "Abbey Road")).toHaveLength(0);
    reader.close();
  });
});

describeDatasetReader("DatasetReader — progressive prefix fallback", () => {
  it("matches when folder name is a superset of DB album", () => {
    const reader = new DatasetReader(dbPath);
    // User's folder might be "T-Time 新歌+精选" but DB has "T-time"
    const results = reader.queryAlbum("Various Artists", "T-Time 新歌+精选");
    expect(results.length).toBeGreaterThanOrEqual(1);
    expect(results[0].album).toBe("T-time");
    reader.close();
  });
});

describeDatasetReader("DatasetReader — artist hint cleanup", () => {
  it("strips parenthetical suffix from artist hint", () => {
    const reader = new DatasetReader(dbPath);
    // DB has "蛋堡", query with "蛋堡 (Soft Lipa)"
    const results = reader.queryAlbum("蛋堡 (Soft Lipa)", "Winter Sweet");
    expect(results).toHaveLength(1);
    expect(results[0].artist).toBe("蛋堡");
    expect(results[0].album).toBe("Winter Sweet");
    reader.close();
  });

  it("keeps parenthetical suffixes that are only digits (birth years)", () => {
    const reader = new DatasetReader(dbPath);
    // No artist with parens exists, but the cleanup should NOT strip "(1969)"
    // so it won't accidentally match a wrong artist
    const results = reader.queryAlbum("Some Artist (1969)", "Abbey Road");
    expect(results).toHaveLength(0);
    reader.close();
  });
});

describeDatasetReader("DatasetReader — year-prefix fallback", () => {
  it("strips leading year from album hint and finds match", () => {
    const reader = new DatasetReader(dbPath);
    // DB has "Abbey Road", query with "1969 Abbey Road"
    const results = reader.queryAlbum("The Beatles", "1969 Abbey Road");
    expect(results).toHaveLength(1);
    expect(results[0].album).toBe("Abbey Road");
    reader.close();
  });

  it("strips year prefix with dash separator", () => {
    const reader = new DatasetReader(dbPath);
    const results = reader.queryAlbum("The Beatles", "1969 - Abbey Road");
    expect(results).toHaveLength(1);
    expect(results[0].album).toBe("Abbey Road");
    reader.close();
  });
});

describeDatasetReader("DatasetReader — cross-service deduplication", () => {
  it("returns one result when same album exists in multiple services", () => {
    const reader = new DatasetReader(dbPath);
    // "Abbey Road" exists in both musicbrainz and spotify with the same name
    const results = reader.queryAlbum("The Beatles", "Abbey Road");
    expect(results).toHaveLength(1);
    expect(results[0].artist).toBe("The Beatles");
    expect(results[0].album).toBe("Abbey Road");
    // Should prefer musicbrainz over spotify
    reader.close();
  });
});

describeDatasetReader("DatasetReader — artist-only fallback", () => {
  it("returns albums by artist when album hint does not match", () => {
    const reader = new DatasetReader(dbPath);
    // Artist matches "The Beatles", album "Nonexistent" won't match
    const results = reader.queryAlbum("The Beatles", "Nonexistent Album");
    expect(results.length).toBeGreaterThanOrEqual(2);
    const albums = results.map(r => r.album);
    expect(albums).toContain("Abbey Road");
    expect(albums).toContain("Sgt. Pepper");
    reader.close();
  });

  it("returns nothing when artist also does not exist", () => {
    const reader = new DatasetReader(dbPath);
    const results = reader.queryAlbum("Nonexistent Artist", "Some Album");
    expect(results).toHaveLength(0);
    reader.close();
  });

  it("artist-only does not fire when album match succeeds", () => {
    const reader = new DatasetReader(dbPath);
    // Exact match on both should not trigger artist-only fallback
    const results = reader.queryAlbum("The Beatles", "Abbey Road");
    expect(results).toHaveLength(1);
    expect(results[0].album).toBe("Abbey Road");
    reader.close();
  });
});
