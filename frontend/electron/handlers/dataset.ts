/**
 * Local dataset reader — queries the SQLite index at ~/.auto-tagger/.
 * Ported from Python auto_tagger.integrations.dataset_raw.
 *
 * Reads the pre-built `dataset_lookup` table and service-specific
 * track tables created by the Python CLI's `auto-tag dataset setup`.
 */

import Database from "better-sqlite3";
import { existsSync } from "node:fs";
import { join } from "node:path";
import { homedir } from "node:os";
import {
  type AlbumCandidate,
  type TrackCandidate,
  makeAlbumCandidate,
  makeTrackCandidate,
  normalizeLookupText,
} from "./candidates";

const DEFAULT_DB_PATH = join(homedir(), ".auto-tagger", "dataset-index.sqlite");
const SERVICE_ORDER: Record<string, number> = {
  musicbrainz: 0,
  spotify: 1,
  tidal: 2,
  deezer: 3,
};

// Track table config: trackTable → { fkColumn, titleColumn, numberColumn, discColumn, durColumn, durIsMs }
interface TrackTableConfig {
  fkColumn: string;
  titleColumn: string;
  numberColumn: string;
  discColumn: string | null;
  durColumn: string | null;
  durIsMs: boolean;
}

const TRACK_TABLES: Record<string, TrackTableConfig> = {
  musicbrainz_release_track: {
    fkColumn: "releaseid",
    titleColumn: "COALESCE(title, recordingtitle)",
    numberColumn: "COALESCE(number, position)",
    discColumn: "mediaposition",
    durColumn: "length",
    durIsMs: false,
  },
  spotify_track: {
    fkColumn: "albumid",
    titleColumn: "name",
    numberColumn: "tracknumber",
    discColumn: "discnumber",
    durColumn: "durationms",
    durIsMs: true,
  },
  tidal_track: {
    fkColumn: "albumid",
    titleColumn: "title",
    numberColumn: "tracknumber",
    discColumn: "volumenumber",
    durColumn: "duration",
    durIsMs: false,
  },
  deezer_track: {
    fkColumn: "albumid",
    titleColumn: "title",
    numberColumn: "trackposition",
    discColumn: "disknumber",
    durColumn: "duration",
    durIsMs: false,
  },
};

export class DatasetReader {
  private db: Database | null = null;
  private dbPath: string;

  constructor(dbPath?: string) {
    this.dbPath = dbPath ?? DEFAULT_DB_PATH;
  }

  /** Check if the dataset index exists and is readable. */
  isAvailable(): boolean {
    return existsSync(this.dbPath);
  }

  /** Return the dataset path. */
  getPath(): string {
    return this.dbPath;
  }

  /** Close the database connection. */
  close(): void {
    this.db?.close();
    this.db = null;
  }

  /**
   * Open (or reuse) the database connection.
   * Throws if the database doesn't exist.
   */
  private open(): Database {
    if (this.db) return this.db;
    this.db = new Database(this.dbPath);
    this.db.pragma("journal_mode = WAL");
    return this.db;
  }

  /**
   * Query the dataset for album candidates matching artist and album hints.
   *
   * Tries in order:
   *   1. Exact normalized match with SC/TC variants
   *   2. Progressive prefix fallback (album hint is a superset of DB name)
   *   3. Track loading from service-specific track tables
   */
  queryAlbum(
    artistHint: string,
    albumHint: string,
    maxCandidates = 5,
  ): AlbumCandidate[] {
    if (!this.isAvailable()) return [];

    const normArtist = normalizeLookupText(artistHint);
    const normAlbum = normalizeLookupText(albumHint);
    if (!normArtist || !normAlbum) return [];

    const db = this.open();

    // Step 1: exact match via dataset_lookup table
    const candidates = this.queryLookupTable(db, normArtist, normAlbum, maxCandidates);
    if (candidates.length > 0) return candidates;

    // Step 2: progressive prefix fallback
    return this.progressivePrefixFallback(db, normArtist, normAlbum, maxCandidates);
  }

  /**
   * Query the dataset_lookup table for exact normalized matches.
   * Builds SC/TC variant pairs for Chinese name matching.
   */
  private queryLookupTable(
    db: Database,
    normArtist: string,
    normAlbum: string,
    maxCandidates: number,
  ): AlbumCandidate[] {
    const variants = this.buildVariantPairs(normArtist, normAlbum);
    if (variants.length === 0) return [];

    const whereClauses = variants.map(() =>
      "(normalized_artist = ? AND normalized_album = ?)",
    );
    const params: string[] = [];
    for (const [a, b] of variants) {
      params.push(a, b);
    }

    const whereSql = whereClauses.join(" OR ");
    const rows = db
      .prepare(
        `SELECT service, artist, album, year, album_id, artist_id
         FROM dataset_lookup
         WHERE (${whereSql})
         ORDER BY
           CASE service
             WHEN 'musicbrainz' THEN 0
             WHEN 'spotify' THEN 1
             WHEN 'tidal' THEN 2
             WHEN 'deezer' THEN 3
             ELSE 4
           END
         LIMIT ?`,
      )
      .all(...params, maxCandidates) as Record<string, unknown>[];

    return rows
      .map((row) => {
        const service = row.service as string;
        const albumId = row.album_id as string;
        const artistName = row.artist as string;
        const tracks = this.loadTracks(db, service, albumId, artistName);
        if (!tracks) return null;
        return makeAlbumCandidate({
          artist: artistName,
          artists: [artistName],
          album: (row.album as string) ?? null,
          albumArtist: artistName,
          albumArtists: [artistName],
          year: (row.year as string) ?? null,
          musicbrainzAlbumId: service === "musicbrainz" ? albumId : null,
          musicbrainzArtistId:
            service === "musicbrainz" ? (row.artist_id as string) ?? null : null,
          tracks,
          source: "dataset",
        });
      })
      .filter((c): c is AlbumCandidate => c !== null);
  }

  /**
   * Progressive prefix fallback: try shorter prefixes of the album hint.
   */
  private progressivePrefixFallback(
    db: Database,
    normArtist: string,
    normAlbum: string,
    maxCandidates: number,
  ): AlbumCandidate[] {
    const seenIds = new Set<string>();
    const candidates: AlbumCandidate[] = [];
    const albumWords = normAlbum.split(" ");

    for (let wordCount = albumWords.length - 1; wordCount > 0; wordCount--) {
      const prefix = albumWords.slice(0, wordCount).join(" ");
      if (prefix.length < 2) continue;

      const rows = db
        .prepare(
          `SELECT service, artist, album, year, album_id, artist_id
           FROM dataset_lookup
           WHERE normalized_artist = ?
             AND normalized_album LIKE ? || '%'
           ORDER BY
             CASE service
               WHEN 'musicbrainz' THEN 0
               WHEN 'spotify' THEN 1
               WHEN 'tidal' THEN 2
               WHEN 'deezer' THEN 3
               ELSE 4
             END
           LIMIT ?`,
        )
        .all(normArtist, prefix, maxCandidates) as Record<string, unknown>[];

      for (const row of rows) {
        const service = row.service as string;
        const albumId = row.album_id as string;
        const key = `${service}:${albumId}`;
        if (seenIds.has(key)) continue;
        seenIds.add(key);

        const artistName = row.artist as string;
        const tracks = this.loadTracks(db, service, albumId, artistName);
        if (!tracks) continue;

        candidates.push(
          makeAlbumCandidate({
            artist: artistName,
            artists: [artistName],
            album: (row.album as string) ?? null,
            albumArtist: artistName,
            albumArtists: [artistName],
            year: (row.year as string) ?? null,
            musicbrainzAlbumId: service === "musicbrainz" ? albumId : null,
            musicbrainzArtistId:
              service === "musicbrainz" ? (row.artist_id as string) ?? null : null,
            tracks,
            source: "dataset",
          }),
        );
      }
      if (candidates.length > 0) break;
    }

    return candidates.slice(0, maxCandidates);
  }

  /**
   * Load tracks for an album from service-specific track tables.
   */
  private loadTracks(
    db: Database,
    service: string,
    albumId: string,
    artistName: string,
  ): TrackCandidate[] | null {
    // Map service to track table name
    const serviceToTrack: Record<string, string> = {
      musicbrainz: "musicbrainz_release_track",
      spotify: "spotify_track",
      tidal: "tidal_track",
      deezer: "deezer_track",
    };
    const trackTableName = serviceToTrack[service];
    if (!trackTableName) return null;

    const config = TRACK_TABLES[trackTableName];
    if (!config) return null;

    // Check if table exists
    const tableExists = db
      .prepare(
        "SELECT name FROM sqlite_master WHERE type='table' AND name = ?",
      )
      .get(trackTableName);
    if (!tableExists) return null;

    // For musicbrainz, add disc total and musicbrainz_trackid
    let extraCols = "";
    let orderCol = `COALESCE(${config.discColumn ?? 1}, 1), COALESCE(${config.numberColumn}, 0)`;

    if (service === "musicbrainz") {
      extraCols = ", mediaposition AS disc_number, mediatrackcount AS disc_total, recordingid AS musicbrainz_trackid";
      // need to also get: COALESCE(mediaposition, 1)
      orderCol = "COALESCE(mediaposition, 1), COALESCE(number, position, 0)";
    }

    let durCol = "";
    if (config.durColumn) {
      durCol = `, ${config.durColumn} AS duration_raw`;
    }

    // Actually let me just implement the query differently for each service to keep it clean
    try {
      let rows: Record<string, unknown>[];

      if (service === "musicbrainz") {
        rows = db
          .prepare(
            `SELECT ${config.titleColumn} AS track_title,
                    ${config.numberColumn} AS track_number,
                    mediaposition AS disc_number,
                    mediatrackcount AS disc_total,
                    length,
                    recordingid AS musicbrainz_trackid
             FROM "${trackTableName}"
             WHERE "${config.fkColumn}" = ?
             ORDER BY COALESCE(mediaposition, 1), COALESCE(number, position, 0)`,
          )
          .all(albumId) as Record<string, unknown>[];
      } else {
        rows = db
          .prepare(
            `SELECT ${config.titleColumn} AS track_title,
                    ${config.numberColumn} AS track_number,
                    ${config.discColumn ?? 1} AS disc_number,
                    ${config.durColumn ?? "NULL"} AS duration_raw
             FROM "${trackTableName}"
             WHERE "${config.fkColumn}" = ?
             ORDER BY COALESCE(${config.discColumn ?? 1}, 1), COALESCE(${config.numberColumn}, 0)`,
          )
          .all(albumId) as Record<string, unknown>[];
      }

      return rows
        .filter((r) => r.track_title)
        .map((r) => {
          let length: number | null = null;
          if (r.duration_raw != null) {
            const raw = r.duration_raw;
            if (typeof raw === "number") {
              length = config.durIsMs ? raw / 1000 : raw;
            } else if (typeof raw === "string") {
              // HH:MM:SS or MM:SS format
              const parts = raw.split(":");
              if (parts.length === 3) {
                length =
                  parseInt(parts[0]) * 3600 +
                  parseInt(parts[1]) * 60 +
                  parseInt(parts[2]);
              } else if (parts.length === 2) {
                length = parseInt(parts[0]) * 60 + parseInt(parts[1]);
              }
            }
          }

          return makeTrackCandidate({
            title: (r.track_title as string) ?? null,
            artist: artistName,
            artists: [artistName],
            trackNumber: r.track_number != null ? Number(r.track_number) : null,
            trackTotal: rows.length,
            discNumber:
              r.disc_number != null ? Number(r.disc_number) : null,
            discTotal:
              service === "musicbrainz" && r.disc_total != null
                ? Number(r.disc_total)
                : null,
            musicbrainzTrackId:
              service === "musicbrainz"
                ? (r.musicbrainz_trackid as string) ?? null
                : null,
            length,
          });
        });
    } catch {
      return null;
    }
  }

  /**
   * Build (artist, album) variant pairs for SC/TC matching.
   * Tries original, simplified, and traditional variants.
   */
  private buildVariantPairs(
    artist: string,
    album: string,
  ): Array<[string, string]> {
    const pairs: Array<[string, string]> = [[artist, album]];

    // Add SC/TC variants using opencc-js
    // We use a simple heuristic: if the text contains CJK characters,
    // generate SC → TC and TC → SC variants
    try {
      const { Converter } = require("opencc-js");
      const s2t = Converter({ from: "cn", to: "tw" });
      const t2s = Converter({ from: "tw", to: "cn" });

      const addPair = (a: string, b: string) => {
        const key: [string, string] = [a, b];
        if (!pairs.some((p) => p[0] === key[0] && p[1] === key[1])) {
          pairs.push(key);
        }
      };

      const aSc = t2s(artist);
      const bSc = t2s(album);
      const aTc = s2t(artist);
      const bTc = s2t(album);

      if (aSc !== artist || bSc !== album) addPair(aSc, bSc);
      if (aTc !== artist || bTc !== album) addPair(aTc, bTc);
    } catch {
      // opencc-js not available — use original only
    }

    return pairs;
  }

  /**
   * Check if the dataset has a dataset_lookup table.
   */
  hasLookupTable(): boolean {
    if (!this.isAvailable()) return false;
    try {
      const db = this.open();
      const row = db
        .prepare(
          "SELECT name FROM sqlite_master WHERE type='table' AND name='dataset_lookup'",
        )
        .get();
      return !!row;
    } catch {
      return false;
    }
  }

  /**
   * Get dataset status info.
   */
  getStatus(): {
    available: boolean;
    musicbrainz: boolean;
    totalRecords: number;
    lastUpdated: string | null;
  } {
    if (!this.isAvailable()) {
      return { available: false, musicbrainz: false, totalRecords: 0, lastUpdated: null };
    }
    try {
      const db = this.open();
      const count = (
        db.prepare("SELECT COUNT(*) AS cnt FROM dataset_lookup").get() as {
          cnt: number;
        }
      ).cnt;
      const mbCount = (
        db
          .prepare(
            "SELECT COUNT(*) AS cnt FROM dataset_lookup WHERE service = 'musicbrainz'",
          )
          .get() as { cnt: number }
      ).cnt;
      return {
        available: true,
        musicbrainz: mbCount > 0,
        totalRecords: count,
        lastUpdated: null, // not tracked in current schema
      };
    } catch {
      return { available: false, musicbrainz: false, totalRecords: 0, lastUpdated: null };
    }
  }
}
