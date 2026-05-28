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
import { writeTags, type WriteFields } from "./writer";
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
  let fixedCount = 0;
  for (const result of results) {
    if (result.field === "path") continue;
    const filePath = audioFiles[result.index];
    if (!filePath) continue;
    const fields = buildAuditWriteFields(result);
    if (!fields) continue;

    try {
      await writeTags(filePath, fields);
      fixedCount++;
      debug.info("audit", `auditAlbum: fixed ${basename(filePath)} field="${result.field}"`);
    } catch (err) {
      debug.warn("audit", `auditAlbum: failed to fix ${basename(filePath)} — ${(err as Error).message}`);
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

  const tracksMeta: Array<TrackMeta | null> = [];
  const filenames: string[] = [];

  for (let i = 0; i < audioFiles.length; i++) {
    if (signal?.aborted) throw new DOMException("Aborted", "AbortError");
    const meta = await readTrackMetadata(audioFiles[i]);
    tracksMeta.push(meta);
    const fn = basename(audioFiles[i]);
    filenames.push(fn);
    debug.debug("audit", `  [${i + 1}/${audioFiles.length}] ${fn} — ${meta ? "metadata OK" : "no metadata"}`);
  }

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

  debug.info("audit", `auditAlbum: calling LLM for ${albumName} (${validTracks.length} track(s))`);
  const messages = buildAuditMessages(artistHint, albumHint, validTracks, filenames);

  const response = await client.completeJson(messages, "AuditResponse", AUDIT_SCHEMA);
  const auditData = response.data as { tracks: AuditTrackResult[] };
  const rawTracks = (auditData.tracks ?? []).filter((t) => t.status !== "correct");
  const fixedCount = await applyAuditFixes(audioFiles, rawTracks);

  debug.info("audit", `auditAlbum: ${albumName} — ${rawTracks.length} issue(s) found, ${fixedCount} fixed`);
  if (rawTracks.length > 0) {
    for (const t of rawTracks) {
      debug.debug("audit", `  Track ${t.index + 1} field="${t.field}" status=${t.status} msg="${t.message}"`);
    }
  }

  return rawTracks;
}

/**
 * Run audit on a library path.
 * Iterates over all album directories and audits each one.
 */
async function runLibraryAudit(
  client: OpenRouterClient,
  libraryPath: string,
  signal?: AbortSignal,
): Promise<{ albums: number; issues: number }> {
  let albumsAudited = 0;
  let totalIssues = 0;

  debug.info("audit", `runLibraryAudit: scanning ${libraryPath}`);

  const albumDirs = discoverAlbumDirs(libraryPath);
  debug.info("audit", `runLibraryAudit: discovered ${albumDirs.length} album dir(s)`);

  const total = albumDirs.length;
  debug.info("audit", `runLibraryAudit: auditing ${total} album(s)`);
  auditEvents.emit("event", { type: "progress", current: 0, total });

  for (let i = 0; i < albumDirs.length; i++) {
    if (signal?.aborted) throw new DOMException("Aborted", "AbortError");

    const albumPath = albumDirs[i];
    const albumName = basename(albumPath);

    debug.info("audit", `runLibraryAudit: album ${i + 1}/${total} — ${albumName}`);

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
      debug.info("audit", `runLibraryAudit: ${albumName} — ${results.length} issue(s) (running total: ${totalIssues})`);

      auditEvents.emit("event", {
        type: "album-result",
        albumPath,
        current: i + 1,
        total,
        results,
        message: results.length > 0
          ? `${albumName}: ${results.length} issue(s)`
          : `${albumName}: OK`,
      });

      albumsAudited++;
    } catch (err) {
      if (signal?.aborted) {
        debug.warn("audit", `runLibraryAudit: ${albumName} — aborted`);
        throw err;
      }
      debug.error("audit", `runLibraryAudit: ${albumName} — error: ${(err as Error).message}`);
      auditEvents.emit("event", {
        type: "album-error",
        albumPath,
        current: i + 1,
        total,
        message: `${albumName}: ${(err as Error).message}`,
      });
    }
  }

  debug.info("audit", `runLibraryAudit: complete — ${albumsAudited} album(s), ${totalIssues} issue(s)`);
  return { albums: albumsAudited, issues: totalIssues };
}

// ── IPC handlers ────────────────────────────────────────────────────

export function registerAuditHandlers(): void {
  ipcMain.handle("audit:run", async (_event, libraryPath: string) => {
    debug.info("audit", `IPC audit:run — libraryPath="${libraryPath}"`);

    // Cancel any running audit
    cancelAudit();

    const abort = new AbortController();
    currentAbort = abort;

    const config = loadConfig();
    if (!config.llmApiKey) {
      debug.error("audit", "audit:run — LLM API key not configured");
      throw new Error("LLM API key is required for audit. Configure it in settings.");
    }
    debug.debug("audit", `audit:run — model=${config.llmModel ?? "default"}`);

    const client = new OpenRouterClient({
      apiKey: config.llmApiKey,
      model: config.llmModel ?? "deepseek/deepseek-chat",
      temperature: 0.1,
      maxTokens: 4096,
    });

    try {
      const result = await runLibraryAudit(client, libraryPath, abort.signal);
      debug.info("audit", `audit:run — completed: ${result.albums} album(s), ${result.issues} issue(s)`);
      auditEvents.emit("event", {
        type: "completed",
        current: result.albums,
        total: result.albums,
        message: `Audit complete: ${result.albums} album(s), ${result.issues} issue(s)`,
      });
      return result;
    } catch (err) {
      if (abort.signal.aborted) {
        debug.warn("audit", "audit:run — cancelled by user");
        auditEvents.emit("event", { type: "cancelled", message: "Audit cancelled" });
        return { albums: 0, issues: 0 };
      }
      debug.error("audit", `audit:run — failed: ${(err as Error).message}`);
      auditEvents.emit("event", {
        type: "failed",
        message: `Audit failed: ${(err as Error).message}`,
      });
      throw err;
    } finally {
      currentAbort = null;
    }
  });

  ipcMain.handle("audit:run-album", async (_event, albumPath: string) => {
    debug.info("audit", `IPC audit:run-album — albumPath="${albumPath}"`);

    const config = loadConfig();
    if (!config.llmApiKey) {
      debug.error("audit", "audit:run-album — LLM API key not configured");
      throw new Error("LLM API key is required for audit.");
    }

    const client = new OpenRouterClient({
      apiKey: config.llmApiKey,
      model: config.llmModel ?? "deepseek/deepseek-chat",
      temperature: 0.1,
      maxTokens: 4096,
    });

    return auditAlbum(client, albumPath);
  });

  ipcMain.handle("audit:cancel", async () => {
    debug.info("audit", "IPC audit:cancel");
    cancelAudit();
  });
}
