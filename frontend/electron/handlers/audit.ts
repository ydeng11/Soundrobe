/**
 * LLM audit — verifies metadata correctness against file paths.
 *
 * Core principle: the file path (album folder, artist folder, filename)
 * should match the metadata fields (album, album_artist, title).
 */

import { ipcMain } from "electron";
import { EventEmitter } from "node:events";
import { readdirSync, statSync } from "node:fs";
import { basename, dirname, extname, join } from "node:path";
import { parseFile } from "music-metadata";
import { OpenRouterClient } from "./openrouter";
import { buildAuditMessages } from "./prompts";
import { loadConfig } from "./auto-tag";
import { saveAlias, isChineseName } from "./aliases";
import type { WriteFields } from "./writer";
import { getDefaultWriteQueue } from "../services/TagWriteQueue";
import { AUDIT_ALBUM_CONCURRENCY, LOCAL_READ_CONCURRENCY, mapConcurrent } from "../services/concurrency";
import debug from "./debug";

const AUDIO_EXTENSIONS = new Set([".mp3", ".flac", ".m4a", ".mp4", ".wav", ".ogg", ".opus", ".aiff"]);

// ── Event types ─────────────────────────────────────────────────────

export interface AuditEvent {
  type:
    | "progress"
    | "album-start"
    | "album-result"
    | "album-error"
    | "completed"
    | "cancelled"
    | "failed";
  albumPath?: string;
  current?: number;
  total?: number;
  message?: string;
  results?: AuditTrackResult[];
}

export interface AuditTrackResult {
  index: number;
  field: string;
  status: "correct" | "warning" | "error";
  message: string;
  suggestion?: string | null;
  corrected?: CorrectedTrack | null;
}

export interface CorrectedTrack {
  title?: string | null;
  artist?: string | null;
  artists?: string[] | null;
  album?: string | null;
  albumArtist?: string | null;
  year?: string | null;
  genre?: string | null;
}

export interface TrackMeta {
  title: string | null;
  artist: string | null;
  artists: string[];
  album: string | null;
  albumArtist: string | null;
  year: string | null;
  genre: string | null;
  trackNumber: number | null;
  trackTotal: number | null;
  discNumber: number | null;
  discTotal: number | null;
}

const AUDIT_SCHEMA = {
  type: "object",
  properties: {
    tracks: {
      type: "array",
      items: {
        type: "object",
        properties: {
          index: { type: "integer", minimum: 0 },
          field: { type: "string" },
          status: { type: "string", enum: ["correct", "warning", "error"] },
          message: { type: "string" },
          suggestion: { type: "string" },
          corrected: {
            type: "object",
            properties: {
              title: { type: "string" },
              artist: { type: "string" },
              artists: { type: "array", items: { type: "string" } },
              album: { type: "string" },
              albumArtist: { type: "string" },
              year: { type: "string" },
              genre: { type: "string" },
            },
          },
        },
        required: ["index", "field", "status", "message"],
      },
    },
  },
  required: ["tracks"],
};

// ── Discogs alias resolution ────────────────────────────────────────

const DISCOGS_BASE = "https://api.discogs.com";

/**
 * Result of resolving a Discogs artist alias.
 * Returns both the alias title string and the numeric Discogs artist ID.
 */
export interface DiscogsAliasResult {
  alias: string;
  artistId: number;
}

/**
 * Check if an artist exists on Discogs by name.
 * First tries the precise artist=<name> search, then falls back to
 * generic q=<name>&type=artist (handles non-Latin names where the
 * Discogs artist title is in Latin script).
 *
 * Returns { alias, artistId } if the artist was found via generic
 * search but NOT via precise. Returns null if the artist was found
 * via precise search (no alias needed) or not found at all.
 */
export async function resolveDiscogsArtistAlias(
  artistName: string,
  discogsToken: string | undefined,
): Promise<DiscogsAliasResult | null> {
  if (!discogsToken) return null;

  const headers: Record<string, string> = {
    "User-Agent": "auto-tagger/0.1.0",
    Authorization: `Discogs token=${discogsToken}`,
  };

  // Step 1: Try precise artist=<name> search
  try {
    const preciseUrl = `${DISCOGS_BASE}/database/search?type=artist&artist=${encodeURIComponent(artistName)}&per_page=5`;
    const preciseRes = await fetch(preciseUrl, { headers, signal: AbortSignal.timeout(10_000) });

    if (preciseRes.ok) {
      const preciseData = (await preciseRes.json()) as { results?: Array<{ title?: string }> };
      const preciseResults = preciseData.results ?? [];

      // If precise search returns results, the artist resolves directly
      if (preciseResults.length > 0) {
        debug.debug("audit", `resolveDiscogsArtistAlias: precise search found "${artistName}" — no alias needed`);
        return null;
      }
    }
  } catch {
    // fall through to generic search
  }

  // Step 2: Try generic q=<name>&type=artist search (for non-Latin names)
  try {
    const genericUrl = `${DISCOGS_BASE}/database/search?type=artist&q=${encodeURIComponent(artistName)}&per_page=5`;
    const genericRes = await fetch(genericUrl, { headers, signal: AbortSignal.timeout(10_000) });

    if (!genericRes.ok) return null;

    const genericData = (await genericRes.json()) as { results?: Array<{ title?: string; id?: number }> };
    const genericResults = genericData.results ?? [];

    if (genericResults.length > 0) {
      const first = genericResults[0];
      const discogsTitle: string | null = (first.title as string) ?? null;
      const discogsId: number | null = first.id != null ? Number(first.id) : null;
      if (discogsTitle && discogsTitle !== artistName && discogsId != null) {
        debug.debug("audit", `resolveDiscogsArtistAlias: generic search found alias "${discogsTitle}" (id=${discogsId})`);
        return { alias: discogsTitle, artistId: discogsId };
      }
      debug.debug("audit", `resolveDiscogsArtistAlias: generic search found same title "${discogsTitle}"`);
    }
  } catch {
    // ignore
  }

  return null;
}

/**
 * Ask the LLM to suggest likely Discogs search aliases for a CJK artist
 * name that didn't resolve directly on Discogs.
 * Only makes sense for Chinese/Japanese/Korean names.
 */
export async function suggestDiscogsAliases(
  artistName: string,
  client: OpenRouterClient,
): Promise<string[]> {
  if (!isChineseName(artistName)) return [];

  debug.debug("audit", `suggestDiscogsAliases: asking LLM for aliases for "${artistName}"`);

  const messages = [
    {
      role: "system" as const,
      content:
        "You suggest artist name aliases for Discogs search. " +
        "The input is a Chinese artist name that doesn't resolve on Discogs. " +
        "Suggest 1-3 English/Latin aliases that might work on Discogs. " +
        "For example, for '刺猬' (a Chinese rock band), suggest 'Hedgehog' " +
        "because Discogs lists them as 'Hedgehog (4)'.\n\n" +
        "Return as JSON: { \"aliases\": [\"alias1\", \"alias2\"] }\n" +
        "If no alias makes sense, return { \"aliases\": [] }",
    },
    {
      role: "user" as const,
      content: JSON.stringify({ artist: artistName }),
    },
  ];

  const ALIAS_SCHEMA = {
    type: "object",
    properties: {
      aliases: {
        type: "array",
        items: { type: "string" },
      },
    },
    required: ["aliases"],
  };

  try {
    const response = await client.completeJson(messages, "AliasSuggestions", ALIAS_SCHEMA);
    const raw = (response.data as { aliases?: unknown[] })?.aliases ?? [];
    const aliases = raw
      .filter((a): a is string => typeof a === "string" && a.trim().length > 0)
      .map((a) => a.trim());
    debug.debug("audit", `suggestDiscogsAliases: LLM suggested [${aliases.join(", ")}]`);
    return aliases;
  } catch (err) {
    debug.warn("audit", `suggestDiscogsAliases: LLM call failed`, err);
    return [];
  }
}

// ── Task manager ────────────────────────────────────────────────────

const auditEvents = new EventEmitter();
let currentAbort: AbortController | null = null;

export function onAuditEvent(listener: (event: AuditEvent) => void): () => void {
  auditEvents.on("event", listener);
  debug.debug("audit", `onAuditEvent subscriber registered`);
  return () => {
    auditEvents.off("event", listener);
    debug.debug("audit", `onAuditEvent subscriber removed`);
  };
}

export function cancelAudit(): void {
  if (currentAbort) {
    debug.info("audit", "Cancelling running audit via AbortController");
    currentAbort.abort();
    currentAbort = null;
  } else {
    debug.debug("audit", "cancelAudit called but no audit running");
  }
}

// ── Core audit logic ────────────────────────────────────────────────

async function readTrackMetadata(filePath: string): Promise<TrackMeta | null> {
  try {
    debug.debug("audit", `Reading metadata: ${basename(filePath)}`);
    const metadata = await parseFile(filePath);
    const common = metadata.common;
    debug.debug("audit", `Metadata read: title="${common.title ?? "(null)"}" artist="${common.artist ?? "(null)"}" album="${common.album ?? "(null)"}"`);

    return {
      title: common.title ?? null,
      artist: common.artist ?? null,
      artists: common.artists ?? [],
      album: common.album ?? null,
      albumArtist: (common as any).albumArtist ?? (common as any).albumartist ?? null,
      year: common.year ? String(common.year) : null,
      genre: common.genre?.length ? common.genre[0] : null,
      trackNumber: common.track?.no ?? null,
      trackTotal: common.track?.of ?? null,
      discNumber: common.disk?.no ?? null,
      discTotal: common.disk?.of ?? null,
    };
  } catch (err) {
    debug.warn("audit", `Failed to read metadata: ${basename(filePath)} — ${(err as Error).message}`);
    return null;
  }
}

export function collectAudioFilesForAudit(albumPath: string): string[] {
  try {
    const entries = readdirSync(albumPath);
    const audioFiles = entries
      .filter((entry) => AUDIO_EXTENSIONS.has(extname(entry).toLowerCase()))
      .filter((entry) => statSync(join(albumPath, entry)).isFile())
      .map((entry) => join(albumPath, entry))
      .sort();
    debug.debug("audit", `collectAudioFiles: ${audioFiles.length} audio file(s) in ${basename(albumPath)}`);
    return audioFiles;
  } catch (err) {
    debug.warn("audit", `collectAudioFiles: could not read ${albumPath} — ${(err as Error).message}`);
    return [];
  }
}

export function discoverAlbumDirs(libraryPath: string): string[] {
  const albumDirs = new Set<string>();
  const addAlbumIfAudio = (dirPath: string): number => {
    const audioFiles = collectAudioFilesForAudit(dirPath);
    if (audioFiles.length > 0) albumDirs.add(dirPath);
    return audioFiles.length;
  };

  const rootAudioCount = addAlbumIfAudio(libraryPath);
  if (rootAudioCount > 0) {
    debug.info("audit", `runLibraryAudit: flat layout — ${rootAudioCount} audio file(s) at root`);
  }

  try {
    const topLevelDirs = readdirSync(libraryPath, { withFileTypes: true })
      .filter((entry) => !entry.name.startsWith(".") && entry.isDirectory())
      .map((entry) => entry.name)
      .sort();
    debug.debug("audit", `runLibraryAudit: ${topLevelDirs.length} top-level dir(s)`);

    for (const topLevelDir of topLevelDirs) {
      const topLevelPath = join(libraryPath, topLevelDir);
      if (addAlbumIfAudio(topLevelPath) > 0) continue;

      for (const entry of readdirSync(topLevelPath, { withFileTypes: true })) {
        if (entry.name.startsWith(".") || !entry.isDirectory()) continue;
        addAlbumIfAudio(join(topLevelPath, entry.name));
      }
    }
  } catch (err) {
    debug.warn("audit", `runLibraryAudit: album discovery failed — ${(err as Error).message}`);
  }

  return Array.from(albumDirs);
}

function buildAuditWriteFields(result: AuditTrackResult): WriteFields | null {
  const corrected = result.corrected as (CorrectedTrack & { album_artist?: string | null }) | null | undefined;
  const fields: WriteFields = {};

  if (corrected) {
    if (corrected.title !== undefined) fields.title = corrected.title;
    if (corrected.artist !== undefined) fields.artist = corrected.artist;
    if (corrected.artists !== undefined) fields.artists = corrected.artists;
    if (corrected.album !== undefined) fields.album = corrected.album;
    const albumArtist = corrected.albumArtist ?? corrected.album_artist;
    if (albumArtist !== undefined) fields.albumArtist = albumArtist;
    if (corrected.year !== undefined) fields.year = corrected.year;
    if (corrected.genre !== undefined) fields.genre = corrected.genre;
  } else if (result.suggestion) {
    if (result.field === "title") fields.title = result.suggestion;
    else if (result.field === "artist") fields.artist = result.suggestion;
    else if (result.field === "artists") fields.artists = [result.suggestion];
    else if (result.field === "album") fields.album = result.suggestion;
    else if (result.field === "album_artist" || result.field === "albumArtist") fields.albumArtist = result.suggestion;
    else if (result.field === "year") fields.year = result.suggestion;
    else if (result.field === "genre") fields.genre = result.suggestion;
  }

  return Object.keys(fields).length > 0 ? fields : null;
}

async function applyAuditFixes(audioFiles: string[], results: AuditTrackResult[]): Promise<number> {
  const jobs: Array<{ filePath: string; fields: WriteFields }> = [];
  for (const result of results) {
    if (result.field === "path") continue;
    const filePath = audioFiles[result.index];
    if (!filePath) continue;
    const fields = buildAuditWriteFields(result);
    if (!fields) continue;
    jobs.push({ filePath, fields });
  }

  if (jobs.length === 0) return 0;

  // Submit through the concurrent write queue
  const writeResults = await getDefaultWriteQueue().submit(jobs);
  const fixedCount = writeResults.filter((r) => r.success).length;

  for (const r of writeResults) {
    if (r.success) {
      debug.info("audit", `auditAlbum: fixed ${basename(r.filePath)}`);
    } else {
      debug.warn("audit", `auditAlbum: failed to fix ${basename(r.filePath)} — ${r.error}`);
    }
  }

  return fixedCount;
}

/**
 * Audit a single album: read all track metadata, build LLM prompt, call OpenRouter.
 */
async function auditAlbum(
  client: OpenRouterClient,
  albumPath: string,
  signal?: AbortSignal,
): Promise<AuditTrackResult[]> {
  const albumName = basename(albumPath);
  debug.info("audit", `auditAlbum: starting — ${albumName}`);

  const audioFiles = collectAudioFilesForAudit(albumPath);
  if (audioFiles.length === 0) {
    debug.debug("audit", `auditAlbum: ${albumName} — no audio files, skipping`);
    return [];
  }
  debug.debug("audit", `auditAlbum: ${albumName} — ${audioFiles.length} audio file(s)`);

  if (signal?.aborted) throw new DOMException("Aborted", "AbortError");

  // Read track metadata concurrently using the shared read concurrency limit
  const filenames = audioFiles.map((f) => basename(f));
  const readResults = await mapConcurrent(
    audioFiles,
    LOCAL_READ_CONCURRENCY,
    async (filePath, i) => {
      if (signal?.aborted) throw new DOMException("Aborted", "AbortError");
      const meta = await readTrackMetadata(filePath);
      debug.debug("audit", `  [${i + 1}/${audioFiles.length}] ${basename(filePath)} — ${meta ? "metadata OK" : "no metadata"}`);
      return meta;
    },
  );
  const tracksMeta = readResults;

  const artistHint = basename(dirname(albumPath));
  const albumHint = basename(albumPath);
  debug.debug("audit", `  artist_hint="${artistHint}" album_hint="${albumHint}"`);

  const hasData = tracksMeta.some((m) => m && (m.title || m.artist));
  if (!hasData) {
    debug.warn("audit", `auditAlbum: ${albumName} — no tracks have meaningful metadata, skipping`);
    return [];
  }

  if (signal?.aborted) throw new DOMException("Aborted", "AbortError");

  const defaultMeta: TrackMeta = {
    title: null, artist: null, artists: [], album: null,
    albumArtist: null, year: null, genre: null,
    trackNumber: null, trackTotal: null, discNumber: null, discTotal: null,
  };

  const validTracks = tracksMeta.map((m) => m ?? defaultMeta);

  // ── Discogs alias preflight ──────────────────────────────────
  // Before calling the LLM for corrections, check if the artist name
  // resolves on Discogs. If not, try to find an alias so the audit
  // prompt can include it as context while still preferring Chinese names.
  let discogsAlias: string | null = null;
  let aliasWasSuggested = false;

  const config = loadConfig();
  const discogsToken = config.discogsToken;
  // Track discogsArtistId for writing to file tags after audit fix
  let discogsArtistId: string | null = null;

  if (discogsToken && artistHint && isChineseName(artistHint)) {
    debug.debug("audit", `${albumName}: checking Discogs for artist="${artistHint}"`);
    const aliasResult = await resolveDiscogsArtistAlias(artistHint, discogsToken);

    if (aliasResult === null) {
      // Precise search found the artist, no alias needed
      debug.debug("audit", `${albumName}: artist resolves directly on Discogs`);
    } else {
      // Generic search found an alias with Discogs artist ID
      debug.info("audit", `${albumName}: Discogs alias found: "${artistHint}" → "${aliasResult.alias}" (id=${aliasResult.artistId})`);
      discogsAlias = aliasResult.alias;
      discogsArtistId = String(aliasResult.artistId);
      aliasWasSuggested = false;

      // Persist the alias for future lookups
      saveAlias(artistHint, aliasResult.alias);

      // Ask LLM for alias suggestions if neither search found it
    }

    if (!discogsAlias) {
      debug.debug("audit", `${albumName}: artist not found on Discogs, asking LLM for aliases`);
      const suggested = await suggestDiscogsAliases(artistHint, client);

      for (const suggestedAlias of suggested) {
        const confirmed = await resolveDiscogsArtistAlias(suggestedAlias, discogsToken);
        if (confirmed) {
          debug.info("audit", `${albumName}: LLM-suggested alias "${suggestedAlias}" confirmed as "${confirmed.alias}" (id=${confirmed.artistId})`);
          discogsAlias = confirmed.alias;
          discogsArtistId = String(confirmed.artistId);
          aliasWasSuggested = true;

          // Persist the confirmed alias
          saveAlias(artistHint, confirmed.alias);
          break;
        } else {
          debug.debug("audit", `${albumName}: LLM-suggested alias "${suggestedAlias}" not confirmed on Discogs`);
        }
      }
    }
  }

  debug.info("audit", `auditAlbum: calling LLM for ${albumName} (${validTracks.length} track(s))`);
  const messages = buildAuditMessages(artistHint, albumHint, validTracks, filenames, { discogsAlias });

  const response = await client.completeJson(messages, "AuditResponse", AUDIT_SCHEMA);
  const auditData = response.data as { tracks: AuditTrackResult[] };
  const rawTracks = (auditData.tracks ?? []).filter((t) => t.status !== "correct");
  const fixedCount = await applyAuditFixes(audioFiles, rawTracks);

  // ── Write discogsArtistId if Discogs alias was found ──────
  // Persist the Discogs artist ID to file tags so the auto-tag
  // pipeline can reuse it for direct ID lookups on future scans.
  if (discogsArtistId && audioFiles.length > 0) {
    const discogsIdJobs = audioFiles.map((filePath) => ({
      filePath,
      fields: { discogsArtistId } as WriteFields,
    }));
    await getDefaultWriteQueue().submit(discogsIdJobs);
    debug.info("audit", `auditAlbum: wrote discogsArtistId=${discogsArtistId} to ${audioFiles.length} file(s)`);
  }

  debug.info("audit", `auditAlbum: ${albumName} — ${rawTracks.length} issue(s) found, ${fixedCount} fixed`);
  if (rawTracks.length > 0) {
    for (const t of rawTracks) {
      debug.debug("audit", `  Track ${t.index + 1} field="${t.field}" status=${t.status} msg="${t.message}"`);
    }
  }

  return rawTracks;
}

/**
 * Audit a specific list of album directories, emitting progress events.
 * Albums are processed concurrently up to AUDIT_ALBUM_CONCURRENCY.
 */
async function auditSpecificAlbums(
  client: OpenRouterClient,
  albumPaths: string[],
  signal?: AbortSignal,
): Promise<{ albums: number; issues: number }> {
  const total = albumPaths.length;
  let albumsAudited = 0;
  let totalIssues = 0;

  debug.info("audit", `auditSpecificAlbums: auditing ${total} album(s) with concurrency ${AUDIT_ALBUM_CONCURRENCY}`);
  auditEvents.emit("event", { type: "progress", current: 0, total });

  // Process albums concurrently using a bounded pool
  const pool = Array.from({ length: Math.min(AUDIT_ALBUM_CONCURRENCY, total) });
  let nextAlbumIndex = 0;

  const worker = async (): Promise<void> => {
    while (nextAlbumIndex < albumPaths.length) {
      if (signal?.aborted) throw new DOMException("Aborted", "AbortError");

      const i = nextAlbumIndex++;
      const albumPath = albumPaths[i];
      const albumName = basename(albumPath);

      debug.info("audit", `auditSpecificAlbums: album ${i + 1}/${total} — ${albumName}`);

      auditEvents.emit("event", {
        type: "album-start",
        albumPath,
        current: i + 1,
        total,
        message: `Auditing: ${albumName}`,
      });

      try {
        const results = await auditAlbum(client, albumPath, signal);
        totalIssues += results.length;

        debug.info("audit", `auditSpecificAlbums: ${albumName} — ${results.length} issue(s) (running total: ${totalIssues})`);

        auditEvents.emit("event", {
          type: "album-result",
          albumPath,
          current: albumsAudited + 1,
          total,
          results,
          message: results.length > 0
            ? `${albumName}: ${results.length} issue(s)`
            : `${albumName}: OK`,
        });

        albumsAudited++;
      } catch (err) {
        if (signal?.aborted) {
          debug.warn("audit", `auditSpecificAlbums: ${albumName} — aborted`);
          return; // Worker exits gracefully on abort
        }
        debug.error("audit", `auditSpecificAlbums: ${albumName} — error: ${(err as Error).message}`);
        auditEvents.emit("event", {
          type: "album-error",
          albumPath,
          current: albumsAudited + 1,
          total,
          message: `${albumName}: ${(err as Error).message}`,
        });
      }
    }
  };

  await Promise.all(pool.map(() => worker()));

  debug.info("audit", `auditSpecificAlbums: complete — ${albumsAudited} album(s), ${totalIssues} issue(s)`);
  return { albums: albumsAudited, issues: totalIssues };
}

/**
 * Run audit on a library path.
 * Discovers all album directories and audits each one.
 */
async function runLibraryAudit(
  client: OpenRouterClient,
  libraryPath: string,
  signal?: AbortSignal,
): Promise<{ albums: number; issues: number }> {
  debug.info("audit", `runLibraryAudit: scanning ${libraryPath}`);

  const albumDirs = discoverAlbumDirs(libraryPath);
  debug.info("audit", `runLibraryAudit: discovered ${albumDirs.length} album dir(s)`);

  return auditSpecificAlbums(client, albumDirs, signal);
}

// ── Helpers ─────────────────────────────────────────────────────────

/** Create an audit OpenRouterClient. Throws if API key is missing. */
function createAuditClient(): OpenRouterClient {
  const config = loadConfig();
  if (!config.llmApiKey) {
    throw new Error("LLM API key is required for audit. Configure it in settings.");
  }
  return new OpenRouterClient({
    apiKey: config.llmApiKey,
    model: config.llmModel ?? "",
    temperature: 0.1,
    maxTokens: 4096,
  });
}

/**
 * Run an audit operation with standard abort handling, event emission,
 * and finally cleanup.
 */
async function runAuditWithHandling<T extends { albums: number; issues: number }>(
  operationName: string,
  operation: (signal: AbortSignal) => Promise<T>,
  abort: AbortController,
): Promise<T> {
  try {
    const result = await operation(abort.signal);
    debug.info("audit", `${operationName} — completed: ${result.albums} album(s), ${result.issues} issue(s)`);
    auditEvents.emit("event", {
      type: "completed",
      current: result.albums,
      total: result.albums,
      message: `Audit complete: ${result.albums} album(s), ${result.issues} issue(s)`,
    });
    return result;
  } catch (err) {
    if (abort.signal.aborted) {
      debug.warn("audit", `${operationName} — cancelled by user`);
      auditEvents.emit("event", { type: "cancelled", message: "Audit cancelled" });
      return { albums: 0, issues: 0 } as T;
    }
    debug.error("audit", `${operationName} — failed: ${(err as Error).message}`);
    auditEvents.emit("event", { type: "failed", message: `Audit failed: ${(err as Error).message}` });
    throw err;
  } finally {
    currentAbort = null;
  }
}

// ── IPC handlers ────────────────────────────────────────────────────

export function registerAuditHandlers(): void {
  ipcMain.handle("audit:run", async (_event, libraryPath: string) => {
    debug.info("audit", `IPC audit:run — libraryPath="${libraryPath}"`);
    cancelAudit();

    const abort = new AbortController();
    currentAbort = abort;

    const client = createAuditClient();
    return runAuditWithHandling("audit:run", (signal) => runLibraryAudit(client, libraryPath, signal), abort);
  });

  /**
   * Run audit on a specified set of albums or tracks.
   * When trackPaths is provided, groups them by album directory.
   * When albumPaths is provided, audits those albums directly.
   */
  ipcMain.handle("audit:run-specified", async (_event, options: { albumPaths?: string[]; trackPaths?: string[] }) => {
    debug.info("audit", `IPC audit:run-specified — trackPaths=${options.trackPaths?.length ?? 0} albumPaths=${options.albumPaths?.length ?? 0}`);
    cancelAudit();

    const abort = new AbortController();
    currentAbort = abort;

    const client = createAuditClient();

    // Determine which albums to audit
    let albumPaths: string[];
    if (options.trackPaths && options.trackPaths.length > 0) {
      const dirs = new Set(options.trackPaths.map((p) => dirname(p)));
      albumPaths = Array.from(dirs);
      debug.info("audit", `audit:run-specified — ${options.trackPaths.length} track(s) in ${albumPaths.length} album dir(s)`);
    } else if (options.albumPaths && options.albumPaths.length > 0) {
      albumPaths = options.albumPaths;
    } else {
      throw new Error("No tracks or albums specified for audit");
    }

    return runAuditWithHandling("audit:run-specified", (signal) => auditSpecificAlbums(client, albumPaths, signal), abort);
  });

  ipcMain.handle("audit:run-album", async (_event, albumPath: string) => {
    debug.info("audit", `IPC audit:run-album — albumPath="${albumPath}"`);

    const client = createAuditClient();
    return auditAlbum(client, albumPath);
  });

  ipcMain.handle("audit:cancel", async () => {
    debug.info("audit", "IPC audit:cancel");
    cancelAudit();
  });
}
