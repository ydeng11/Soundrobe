/**
 * SQLite cache for lookup candidates and album processing state.
 * Ported from Python auto_tagger.integrations.cache.
 *
 * Uses better-sqlite3 for synchronous SQLite access from the main process.
 */

import { createHash } from "node:crypto";
import { mkdirSync } from "node:fs";
import { readdirSync, statSync } from "node:fs";
import { dirname, join } from "node:path";
import {
  type AlbumCandidate,
  type LookupRequest,
  candidatesToJson,
  candidatesFromJson,
  lookupRequestToJson,
  queryHash,
} from "./candidates";
import { getBetterSqlite3, type BetterSqlite3Database } from "./native-check";

export interface ReleaseMeta {
  id: string;
  title: string;
  year: number | null;
  type: "master" | "release" | null;
  artistName: string | null;
}

const VALID_STATUSES = new Set(["pending", "llm_parsed", "tagged_ok", "error"]);

function sha256(text: string): string {
  return createHash("sha256").update(text).digest("hex");
}

function pathHash(filePath: string): string {
  return sha256(filePath);
}

function folderNameHash(name: string): string {
  return sha256(name.trim());
}

/** Hash of (sorted filenames + sizes) for change detection. */
function contentHash(albumPath: string): string {
  try {
    if (!statSync(albumPath).isDirectory()) return "";
    const entries: string[] = [];
    for (const f of readdirSync(albumPath).sort()) {
      const fullPath = join(albumPath, f);
      try {
        const stat = statSync(fullPath);
        if (stat.isFile()) {
          entries.push(`${f}:${stat.size}`);
        }
      } catch {
        // skip unreadable files
      }
    }
    return sha256(entries.join("|"));
  } catch {
    return "";
  }
}

export class MatchCache {
  private db: BetterSqlite3Database;

  constructor(cachePath: string) {
    try {
      mkdirSync(dirname(cachePath), { recursive: true });
    } catch {
      // directory may already exist
    }
    const Database = getBetterSqlite3();
    this.db = new Database(cachePath);
    this.db.pragma("journal_mode = WAL");
    this.initSchema();
  }

  close(): void {
    this.db.close();
  }

  // ── lookup cache ──────────────────────────────────────────────────

  get(request: LookupRequest): AlbumCandidate[] | null {
    const hash = queryHash(request);
    const row = this.db
      .prepare("SELECT response_json FROM lookup_cache WHERE query_hash = ?")
      .get(hash) as { response_json: string } | undefined;
    if (!row) return null;
    return candidatesFromJson(row.response_json);
  }

  set(request: LookupRequest, candidates: AlbumCandidate[]): void {
    if (candidates.length === 0) return;
    const hash = queryHash(request);
    const now = new Date().toISOString();
    const source = candidates[0].source;
    this.db
      .prepare(
        `INSERT OR REPLACE INTO lookup_cache
         (query_hash, query_json, response_json, created_at, source)
         VALUES (?, ?, ?, ?, ?)`,
      )
      .run(
        hash,
        JSON.stringify(lookupRequestToJson(request)),
        candidatesToJson(candidates),
        now,
        source,
      );
  }

  // ── album state ledger ────────────────────────────────────────────

  getAlbumState(
    albumPath: string,
  ): Record<string, string | number | null> | null {
    const ph = pathHash(albumPath);
    const row = this.db
      .prepare(
        `SELECT status, content_hash, folder_name_hash, llm_extraction,
                disc_count, error, processed_at
         FROM album_state WHERE path_hash = ?`,
      )
      .get(ph) as Record<string, unknown> | undefined;
    if (!row) return null;
    return {
      status: row.status as string,
      pathHash: ph,
      contentHash: row.content_hash as string,
      folderNameHash: (row.folder_name_hash as string) ?? null,
      llmExtraction: row.llm_extraction
        ? JSON.parse(row.llm_extraction as string)
        : null,
      discCount: (row.disc_count as number) ?? 0,
      error: (row.error as string) ?? null,
      processedAt: (row.processed_at as string) ?? null,
    };
  }

  setAlbumState(
    albumPath: string,
    status: string,
    discCount = 0,
    error: string | null = null,
  ): void {
    if (!VALID_STATUSES.has(status)) {
      throw new Error(
        `Invalid album status: ${status}. Valid: ${[...VALID_STATUSES].sort().join(", ")}`,
      );
    }
    const ph = pathHash(albumPath);
    const ch = contentHash(albumPath);
    const now = new Date().toISOString();
    const parent = dirname(albumPath);
    const fnh = parent ? folderNameHash(parent) : null;

    this.db
      .prepare(
        `INSERT OR REPLACE INTO album_state
         (path_hash, status, content_hash, folder_name_hash,
          disc_count, error, processed_at)
         VALUES (?, ?, ?, ?, ?, ?, ?)`,
      )
      .run(ph, status, ch, fnh, discCount, error, now);
  }

  clearAlbumState(albumPath: string): void {
    const ph = pathHash(albumPath);
    this.db.prepare("DELETE FROM album_state WHERE path_hash = ?").run(ph);
  }

  // ── LLM folder extraction cache ──────────────────────────────────

  getLlmExtraction(folderName: string): Record<string, string | null> | null {
    const fnh = folderNameHash(folderName);
    const row = this.db
      .prepare(
        `SELECT llm_extraction FROM album_state
         WHERE folder_name_hash = ? AND llm_extraction IS NOT NULL
         LIMIT 1`,
      )
      .get(fnh) as { llm_extraction: string } | undefined;
    if (!row) return null;
    return JSON.parse(row.llm_extraction);
  }

  setLlmExtraction(
    folderName: string,
    extraction: Record<string, string | null>,
  ): void {
    const fnh = folderNameHash(folderName);
    const raw = JSON.stringify(extraction);
    const now = new Date().toISOString();
    this.db
      .prepare(
        `INSERT OR REPLACE INTO album_state
         (path_hash, status, content_hash, folder_name_hash,
          llm_extraction, disc_count, error, processed_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?)`,
      )
      .run(`_llm_${fnh}`, "llm_parsed", "", fnh, raw, 0, null, now);
  }

  // ── schema ────────────────────────────────────────────────────────

  private initSchema(): void {
    this.db.exec(`
      CREATE TABLE IF NOT EXISTS lookup_cache (
        query_hash TEXT PRIMARY KEY,
        query_json TEXT NOT NULL,
        response_json TEXT NOT NULL,
        created_at TEXT NOT NULL,
        source TEXT NOT NULL
      );
      CREATE TABLE IF NOT EXISTS album_state (
        path_hash TEXT PRIMARY KEY,
        status TEXT NOT NULL,
        content_hash TEXT NOT NULL,
        folder_name_hash TEXT,
        llm_extraction TEXT,
        disc_count INTEGER DEFAULT 0,
        error TEXT,
        processed_at TEXT
      );
    `);
  }
}

export class ReleaseCache {
  private db: BetterSqlite3Database;

  constructor(cachePath: string) {
    try {
      mkdirSync(dirname(cachePath), { recursive: true });
    } catch {
      // directory may already exist
    }
    const Database = getBetterSqlite3();
    this.db = new Database(cachePath);
    this.db.pragma("journal_mode = WAL");
    this.initSchema();
  }

  close(): void {
    this.db.close();
  }

  getArtistReleaseList(
    provider: string,
    artistId: string,
    page: number,
  ): ReleaseMeta[] | null {
    const row = this.db
      .prepare(
        `SELECT releases_json FROM artist_release_cache
         WHERE provider = ? AND artist_id = ? AND page = ?`,
      )
      .get(provider, artistId, page) as { releases_json: string } | undefined;
    if (!row) return null;
    return JSON.parse(row.releases_json) as ReleaseMeta[];
  }

  setArtistReleaseList(
    provider: string,
    artistId: string,
    page: number,
    releases: ReleaseMeta[],
  ): void {
    const now = new Date().toISOString();
    this.db
      .prepare(
        `INSERT OR REPLACE INTO artist_release_cache
         (provider, artist_id, page, releases_json, fetched_at)
         VALUES (?, ?, ?, ?, ?)`,
      )
      .run(provider, artistId, page, JSON.stringify(releases), now);
  }

  getReleaseDetail(provider: string, releaseId: string): AlbumCandidate | null {
    const row = this.db
      .prepare(
        `SELECT detail_json FROM release_detail_cache
         WHERE provider = ? AND release_id = ?`,
      )
      .get(provider, releaseId) as { detail_json: string } | undefined;
    if (!row) return null;
    return candidatesFromJson(row.detail_json)[0] ?? null;
  }

  setReleaseDetail(
    provider: string,
    releaseId: string,
    candidate: AlbumCandidate,
  ): void {
    const now = new Date().toISOString();
    this.db
      .prepare(
        `INSERT OR REPLACE INTO release_detail_cache
         (provider, release_id, detail_json, fetched_at)
         VALUES (?, ?, ?, ?)`,
      )
      .run(provider, releaseId, candidatesToJson([candidate]), now);
  }

  prune(maxAgeHours: number): void {
    const cutoff = new Date(Date.now() - maxAgeHours * 60 * 60 * 1000).toISOString();
    this.db
      .prepare("DELETE FROM artist_release_cache WHERE fetched_at < ?")
      .run(cutoff);
    this.db
      .prepare("DELETE FROM release_detail_cache WHERE fetched_at < ?")
      .run(cutoff);
  }

  private initSchema(): void {
    this.db.exec(`
      CREATE TABLE IF NOT EXISTS artist_release_cache (
        provider TEXT NOT NULL,
        artist_id TEXT NOT NULL,
        page INTEGER NOT NULL DEFAULT 1,
        releases_json TEXT NOT NULL,
        fetched_at TEXT NOT NULL,
        PRIMARY KEY (provider, artist_id, page)
      );
      CREATE TABLE IF NOT EXISTS release_detail_cache (
        provider TEXT NOT NULL,
        release_id TEXT NOT NULL,
        detail_json TEXT NOT NULL,
        fetched_at TEXT NOT NULL,
        PRIMARY KEY (provider, release_id)
      );
    `);
  }
}
