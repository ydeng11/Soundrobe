import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { mkdtempSync, rmSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import Database from "better-sqlite3";
import { DatasetReader } from "../../electron/handlers/dataset";

let tmpDir: string;
let dbPath: string;

function createFixtureDb(): void {
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

  // Beatles - Abbey Road
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

  // Progressive prefix test: folder name is superset of DB album name
  insertLookup.run(
    "musicbrainz", "Various Artists", "T-time",
    "2000", "mb-ttime-1", "mb-various-1",
    "various artists", "t time",
  );

  // Spotify entry for multi-service test
  insertLookup.run(
    "spotify", "The Beatles", "Abbey Road Remastered", "2019",
    "sp-abbey-1", "sp-beatles-1",
    "the beatles", "abbey road remastered",
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

describe("DatasetReader — setup", () => {
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

describe("DatasetReader — queries", () => {
  it("returns results for exact match", () => {
    const reader = new DatasetReader(dbPath);
    const results = reader.queryAlbum("The Beatles", "Abbey Road");
    expect(results).toHaveLength(1);
    expect(results[0].artist).toBe("The Beatles");
    expect(results[0].album).toBe("Abbey Road");
    expect(results[0].year).toBe("1969");
    expect(results[0].source).toBe("dataset");
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

describe("DatasetReader — progressive prefix fallback", () => {
  it("matches when folder name is a superset of DB album", () => {
    const reader = new DatasetReader(dbPath);
    // User's folder might be "T-Time 新歌+精选" but DB has "T-time"
    const results = reader.queryAlbum("Various Artists", "T-Time 新歌+精选");
    expect(results.length).toBeGreaterThanOrEqual(1);
    expect(results[0].album).toBe("T-time");
    reader.close();
  });
});
