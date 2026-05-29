/**
 * Local dataset reader — queries the SQLite index at ~/.auto-tagger/.
 * Ported from Python auto_tagger.integrations.dataset_raw.
 *
 * Reads the pre-built `dataset_lookup` table and service-specific
 * track tables created by the Python CLI's `auto-tag dataset setup`.
 */

import { existsSync } from "node:fs";
import { getBetterSqlite3, type BetterSqlite3Database } from "./native-check";
import { join } from "node:path";
import { homedir } from "node:os";
import {
  type AlbumCandidate,
  type TrackCandidate,
  artistDisplayName,
  buildLookupVariantPairs,
  makeAlbumCandidate,
  makeTrackCandidate,
  normalizeLookupText,
  splitArtistNames,
} from "./candidates";

const DEFAULT_DB_PATH = join(homedir(), ".auto-tagger", "dataset-index.sqlite");


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
  private db: BetterSqlite3Database | null = null;
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
  private open(): BetterSqlite3Database {
    if (this.db) return this.db;
    const Database = getBetterSqlite3();
    this.db = new Database(this.dbPath);
    this.db.pragma("journal_mode = WAL");
    return this.db;
  }

  /**
   * Strip trailing parenthetical suffixes that are descriptive (not years/dates).
   * E.g. "蛋堡 (Soft Lipa)" → "蛋堡", "Various Artists (VA)" → "Various Artists",
   * but "Pink Floyd (1965)" is left unchanged.
   */
  private static cleanArtistHint(hint: string): string {
    // Strip trailing parenthetical if it contains at least one non-digit, non-space char
    // (e.g. "蛋堡 (Soft Lipa)" → "蛋堡", keeps "Pink Floyd (1965)")
    return hint.replace(/\s*\([^)]*[^\d\s][^)]*\)\s*$/, "").trim();
  }

  /**
   * Deduplicate candidates: same normalized (artist, album) from multiple services
   * (MusicBrainz, Spotify, Tidal, Deezer) collapse to one. Keeps the first occurrence
   * per group since the SQL already orders by service priority (MB > Spotify > Tidal > Deezer).
   */
  private static deduplicateCandidates(
    candidates: AlbumCandidate[],
    maxCandidates: number,
  ): AlbumCandidate[] {
    if (candidates.length <= 1) return candidates;

    const seen = new Set<string>();
    const deduped: AlbumCandidate[] = [];

    for (const c of candidates) {
      const normArtist = normalizeLookupText(c.artist ?? "");
      const normAlbum = normalizeLookupText(c.album ?? "");
      // Skip entries with no artist or album name (shouldn't happen, but be safe)
      if (!normArtist || !normAlbum) continue;
      const key = `${normArtist}|||${normAlbum}`;
      if (seen.has(key)) continue;
      seen.add(key);
      deduped.push(c);
      if (deduped.length >= maxCandidates) break;
    }

    return deduped;
  }

  /**
   * Query the dataset for album candidates matching artist and album hints.
   *
   * Tries in order:
   *   1. Exact normalized match with SC/TC variants
   *   2. Progressive prefix fallback (album hint is a superset of DB name)
   *   3. Year-prefix fallback (strip leading year, retry)
   *   4. Artist-only fallback (artist hint only)
   */
  /** Regex to extract a leading year from normalized album text. */
  private static YEAR_PREFIX_RE = /^(\d{4})\s*[-–—]?\s*/;

  queryAlbum(
    artistHint: string,
    albumHint: string,
    maxCandidates = 5,
  ): AlbumCandidate[] {
    if (!this.isAvailable()) return [];

    // Clean artist hint: strip descriptive parenthetical suffixes before normalization
    const cleanArtist = DatasetReader.cleanArtistHint(artistHint);
    const normArtist = normalizeLookupText(cleanArtist);
    const normAlbum = normalizeLookupText(albumHint);
    if (!normArtist || !normAlbum) return [];

    const db = this.open();
    let result: AlbumCandidate[] = [];

    // Step 1: exact match via dataset_lookup table
    result = this.queryLookupTable(db, normArtist, normAlbum, maxCandidates);

    // Step 2: progressive prefix fallback
    if (result.length === 0) {
      result = this.progressivePrefixFallback(db, normArtist, normAlbum, maxCandidates);
    }

    // Step 3: year-prefix fallback — strip leading year, retry
    if (result.length === 0) {
      const stripped = DatasetReader.stripYearPrefix(normAlbum);
      if (stripped) {
        result = this.queryLookupTable(db, normArtist, stripped, maxCandidates);
        if (result.length === 0) {
          result = this.progressivePrefixFallback(db, normArtist, stripped, maxCandidates);
        }
      }
    }

    // Step 4: artist-only fallback — album hint failed, return albums by artist
    if (result.length === 0) {
      result = this.queryArtistOnly(db, normArtist, maxCandidates);
    }

    // Deduplicate: same album from multiple services → keep the best source
    return DatasetReader.deduplicateCandidates(result, maxCandidates);
  }

  /**
   * If the text starts with a 4-digit year (optionally followed by a separator),
   * return the remainder. Otherwise return null.
   */
  private static stripYearPrefix(text: string): string | null {
    const m = DatasetReader.YEAR_PREFIX_RE.exec(text);
    if (!m) return null;
    const rest = text.slice(m[0].length).trim();
    return rest.length > 0 ? rest : null;
  }

  /**
   * Query the dataset_lookup table for exact normalized matches.
   * Builds SC/TC variant pairs for Chinese name matching.
   */
  private queryLookupTable(
    db: BetterSqlite3Database,
    normArtist: string,
    normAlbum: string,
    maxCandidates: number,
  ): AlbumCandidate[] {
    const variants = buildLookupVariantPairs(normArtist, normAlbum);
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
          artist: artistDisplayName(splitArtistNames([artistName]), artistName),
          artists: splitArtistNames([artistName]),
          album: (row.album as string) ?? null,
          albumArtist: artistName,
          albumArtists: splitArtistNames([artistName]),
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
    db: BetterSqlite3Database,
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
            artist: artistDisplayName(splitArtistNames([artistName]), artistName),
            artists: splitArtistNames([artistName]),
            album: (row.album as string) ?? null,
            albumArtist: artistName,
            albumArtists: splitArtistNames([artistName]),
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
   * Artist-only fallback: when artist+album matching fails, find albums by artist alone.
   */
  private queryArtistOnly(
    db: BetterSqlite3Database,
    normArtist: string,
    maxCandidates: number,
  ): AlbumCandidate[] {
    const rows = db
      .prepare(
        `SELECT service, artist, album, year, album_id, artist_id
         FROM dataset_lookup
         WHERE normalized_artist = ?
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
      .all(normArtist, maxCandidates) as Record<string, unknown>[];

    return rows
      .map((row) => {
        const service = row.service as string;
        const albumId = row.album_id as string;
        const artistName = row.artist as string;
        const tracks = this.loadTracks(db, service, albumId, artistName);
        if (!tracks) return null;
        return makeAlbumCandidate({
          artist: artistDisplayName(splitArtistNames([artistName]), artistName),
          artists: splitArtistNames([artistName]),
          album: (row.album as string) ?? null,
          albumArtist: artistName,
          albumArtists: splitArtistNames([artistName]),
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
   * Load tracks for an album from service-specific track tables.
   */
  private loadTracks(
    db: BetterSqlite3Database,
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

    try {
      let rows: Record<string, unknown>[];

      if (service === "musicbrainz") {
        rows = db
          .prepare(
            `SELECT ${config.titleColumn} AS track_title,
                    releasetrackid,
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

      const trackArtistMap =
        service === "musicbrainz"
          ? this.loadMusicBrainzTrackArtists(db, rows, artistName)
          : new Map<string, string[]>();

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

          const trackArtists =
            typeof r.releasetrackid === "string"
              ? trackArtistMap.get(r.releasetrackid) ?? splitArtistNames([artistName])
              : splitArtistNames([artistName]);

          return makeTrackCandidate({
            title: (r.track_title as string) ?? null,
            artist: artistDisplayName(trackArtists, artistName),
            artists: trackArtists,
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

  private loadMusicBrainzTrackArtists(
    db: BetterSqlite3Database,
    rows: Record<string, unknown>[],
    fallbackArtist: string,
  ): Map<string, string[]> {
    const ids = rows
      .map((row) => row.releasetrackid)
      .filter((id): id is string => typeof id === "string" && id.length > 0);
    const result = new Map<string, string[]>();
    if (ids.length === 0) return result;

    const hasCredits = db
      .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name = ?")
      .get("musicbrainz_release_track_artist");
    const hasArtists = db
      .prepare("SELECT name FROM sqlite_master WHERE type='table' AND name = ?")
      .get("musicbrainz_artist");
    if (!hasCredits || !hasArtists) return result;

    const placeholders = ids.map(() => "?").join(",");
    try {
      const creditRows = db
        .prepare(
          `SELECT ta.releasetrackid AS releasetrackid,
                  a.name AS artist_name,
                  ta."index" AS artist_index
           FROM musicbrainz_release_track_artist ta
           JOIN musicbrainz_artist a ON ta.artistid = a.artistid
           WHERE ta.releasetrackid IN (${placeholders})
           ORDER BY ta.releasetrackid, ta."index"`,
        )
        .all(...ids) as Record<string, unknown>[];

      for (const row of creditRows) {
        const id = row.releasetrackid as string;
        const artists = result.get(id) ?? [];
        artists.push(String(row.artist_name ?? fallbackArtist));
        result.set(id, artists);
      }
    } catch {
      return new Map<string, string[]>();
    }

    return result;
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
