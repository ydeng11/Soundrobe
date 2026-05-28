/**
 * Auto-tag orchestrator — implements the full lookup chain.
 *
 * Lookup chain:
 *   Parse hints → LLM enhancement → Dataset → Cache → MusicBrainz
 *   → Discogs release lookup → Folder fallback → Cache write → LLM selection
 */

import { EventEmitter } from "node:events";
import {
  type AlbumCandidate,
  type TrackCandidate,
  artistDisplayName,
  buildLookupVariantPairs,
  makeAlbumCandidate,
  makeLookupRequest,
  splitArtistNames,
} from "./candidates";
import { MatchCache } from "./cache";
import { DatasetReader } from "./dataset";
import { MusicBrainzClient } from "./musicbrainz";
import { DiscogsClient } from "./discogs";
import { OpenRouterClient } from "./openrouter";
import { parseAlbumWithTags, candidateFromFolder } from "./fallback";
import { buildSelectionMessages, buildFolderExtractionMessages } from "./prompts";
import { writeTags } from "./writer";
import type { WriteFields } from "./writer";
import { existsSync, readFileSync, writeFileSync, mkdirSync, readdirSync, statSync } from "node:fs";
import { basename, dirname, join, extname } from "node:path";

const AUDIO_EXTENSIONS = new Set([".mp3", ".flac", ".m4a", ".mp4", ".wav", ".ogg", ".opus", ".aiff"]);
import { homedir } from "node:os";
import debug from "./debug";

// ── Task types ──────────────────────────────────────────────────────

export interface TaskProgress {
  taskId: string;
  status: "running" | "completed" | "failed" | "cancelled";
  progress: number;
  total: number;
  message: string;
  result: unknown;
}

export interface AutoTagEvent {
  taskId: string;
  type:
    | "progress"
    | "lookup"
    | "source"
    | "merge"
    | "write"
    | "warning"
    | "completed"
    | "failed"
    | "cancelled";
  message: string;
  progress: number;
  total: number;
  data?: unknown;
}

export interface AutoTagConfig {
  llmApiKey?: string;
  llmModel?: string;
  datasetPath?: string;
  cachePath?: string;
  discogsToken?: string;
  remoteLookupEnabled?: boolean;
  discogsEnabled?: boolean;
  debug?: boolean;
}

const taskEvents = new EventEmitter();

export function onAutoTagEvent(
  listener: (event: AutoTagEvent) => void,
): () => void {
  taskEvents.on("event", listener);
  return () => taskEvents.off("event", listener);
}

// ── Config loading ──────────────────────────────────────────────────

/**
 * Resolve config file paths based on current HOME.
 * Must be a function (not a constant) so tests can override HOME.
 */
function getConfigPaths(): string[] {
  const home = process.env.HOME || homedir();
  return [
    join(home, ".auto-tagger", "config.yaml"),
    join(home, ".config", "auto-tagger", "config.yaml"),
    join(home, ".auto-tagger.yaml"),
  ];
}

/**
 * Load config from YAML file (simple key-value parse, no YAML dep).
 * Falls back to environment variables AUTO_TAG_*.
 */
export function loadConfig(): AutoTagConfig {
  const config: AutoTagConfig = {
    remoteLookupEnabled: true,
    discogsEnabled: true,
  };

  // Map config file keys to config object setters
  const configSetters: Record<
    string,
    (value: string) => void
  > = {
    llm_api_key: (v) => { config.llmApiKey = v; },
    llm_model: (v) => { config.llmModel = v; },
    discogs_token: (v) => { config.discogsToken = v; },
    dataset_path: (v) => { config.datasetPath = v; },
    remote_lookup_enabled: (v) => {
      if (v === "false" || v === "true") config.remoteLookupEnabled = v === "true";
    },
    remote_lookup: (v) => {
      if (v === "false" || v === "true") config.remoteLookupEnabled = v === "true";
    },
    discogs_enabled: (v) => {
      if (v === "false" || v === "true") config.discogsEnabled = v === "true";
    },
    debug: (v) => {
      if (v === "false" || v === "true") config.debug = v === "true";
    },
  };

  // 1. Config file first (lowest priority)
  for (const cfgPath of getConfigPaths()) {
    if (existsSync(cfgPath)) {
      try {
        const text = readFileSync(cfgPath, "utf-8");
        const lines = text.split("\n");
        for (const line of lines) {
          const trimmed = line.trim();
          if (!trimmed || trimmed.startsWith("#")) continue;
          const colonIdx = trimmed.indexOf(":");
          if (colonIdx === -1) continue;
          const key = trimmed.slice(0, colonIdx).trim();
          let value = trimmed.slice(colonIdx + 1).trim();
          // Remove quotes
          if (value.startsWith('"') && value.endsWith('"')) {
            value = value.slice(1, -1);
          } else if (value.startsWith("'") && value.endsWith("'")) {
            value = value.slice(1, -1);
          }
          configSetters[key]?.(value);
        }
      } catch {
        // skip unreadable config
      }
      break; // Use first found config file
    }
  }

  // 2. Environment variables override config file (highest priority)
  if (process.env.AUTO_TAG_LLM_API_KEY) config.llmApiKey = process.env.AUTO_TAG_LLM_API_KEY;
  if (process.env.AUTO_TAG_LLM_MODEL) config.llmModel = process.env.AUTO_TAG_LLM_MODEL;
  if (process.env.AUTO_TAG_DISCOGS_TOKEN) config.discogsToken = process.env.AUTO_TAG_DISCOGS_TOKEN;
  if (process.env.AUTO_TAG_REMOTE_LOOKUP === "false") config.remoteLookupEnabled = false;
  if (process.env.AUTO_TAG_DISCOGS_ENABLED === "false") config.discogsEnabled = false;
  if (process.env.AUTO_TAG_DEBUG === "true") config.debug = true;

  // Sync debug logger with config
  debug.setEnabled(!!config.debug);

  return config;
}

// ── Task Manager ────────────────────────────────────────────────────

class TaskManager {
  private tasks = new Map<
    string,
    {
      progress: TaskProgress;
      abort: AbortController;
    }
  >();
  private counter = 0;
  private config: AutoTagConfig;

  constructor(config?: AutoTagConfig) {
    this.config = config ?? loadConfig();
  }

  /** Refresh config from file/env. */
  refreshConfig(): void {
    this.config = loadConfig();
  }

  getConfig(): AutoTagConfig {
    return this.config;
  }

  /**
   * Start auto-tagging an album.
   * Returns a taskId that can be used to poll progress or cancel.
   */
  startAutoTag(albumPath: string): string {
    const taskId = `auto-tag-${++this.counter}-${Date.now()}`;
    const abort = new AbortController();

    this.tasks.set(taskId, {
      progress: {
        taskId,
        status: "running",
        progress: 0,
        total: 9,
        message: "Starting...",
        result: null,
      },
      abort,
    });

    // Start the async processing (don't await — runs in background)
    this.processAlbum(taskId, albumPath, abort.signal).catch((err) => {
      const msg = err instanceof Error ? err.message : "Unknown error";
      debug.error("auto-tag", `Task ${taskId} failed: ${msg}`, err);
      this.updateTask(taskId, {
        status: "failed",
        message: msg,
      });
      this.emitTask(taskId, "failed", msg, { error: msg });
    });

    return taskId;
  }

  getTaskProgress(taskId: string): TaskProgress | null {
    return this.tasks.get(taskId)?.progress ?? null;
  }

  cancelTask(taskId: string): void {
    const task = this.tasks.get(taskId);
    if (task) {
      task.abort.abort();
      this.updateTask(taskId, {
        status: "cancelled",
        message: "Cancelled",
      });
    }
  }

  // ── The lookup chain ────────────────────────────────────────────

  private async processAlbum(
    taskId: string,
    albumPath: string,
    signal: AbortSignal,
  ): Promise<void> {
    const update = (msg: string, step: number) =>
      this.updateTask(taskId, { message: msg, progress: step });

    debug.info("auto-tag", `processAlbum start: ${albumPath} [task=${taskId}]`);
    debug.startTimer(`process-${taskId}`);

    try {
      // Step 1: Parse folder hints
      debug.info("auto-tag", "Step 1/9: Parsing folder hints...");
      update("Parsing folder hints...", 1);
      if (signal.aborted) return this.failCancelled(taskId);
      const request = await parseAlbumWithTags(albumPath);
      debug.info("auto-tag", `Parsed hints: artist="${request.artistHint}" album="${request.albumHint}" year="${request.yearHint}"`);

      // Step 2: LLM hint enhancement (if ambiguous)
      debug.info("auto-tag", "Step 2/9: Checking folder name...");
      update("Checking folder name...", 2);
      if (signal.aborted) return this.failCancelled(taskId);
      const enhancedRequest = await this.enhanceHints(request, signal);
      const hintsChanged = request.artistHint !== enhancedRequest.artistHint || request.albumHint !== enhancedRequest.albumHint;
      if (hintsChanged) {
        debug.info("auto-tag", `LLM enhanced: artist="${enhancedRequest.artistHint}" album="${enhancedRequest.albumHint}"`);
      } else {
        debug.debug("auto-tag", "Hints unchanged after LLM check");
      }

      // Step 3: Local staging-derived dataset index must be checked first.
      debug.info("auto-tag", "Step 3/9: Querying local dataset...");
      update("Querying local dataset...", 3);
      if (signal.aborted) return this.failCancelled(taskId);
      let allCandidates: AlbumCandidate[] = [];
      const lookupVariants = buildLookupVariantPairs(
        enhancedRequest.artistHint,
        enhancedRequest.albumHint,
      );
      const dataset = new DatasetReader(this.config.datasetPath);
      if (dataset.isAvailable() && dataset.hasLookupTable()) {
        debug.debug("auto-tag", `Dataset available at: ${dataset.getPath()}`);
        try {
          const datasetCandidates = dataset.queryAlbum(
            enhancedRequest.artistHint ?? "",
            enhancedRequest.albumHint ?? "",
          );
          debug.info("auto-tag", `Dataset returned ${datasetCandidates.length} candidates`);
          this.emitTask(taskId, "source", `Local dataset: ${datasetCandidates.length} candidate(s)`, {
            source: "dataset",
            path: dataset.getPath(),
            count: datasetCandidates.length,
          });
          allCandidates.push(...datasetCandidates);
        } finally {
          dataset.close();
        }
      } else {
        const msg = `Local dataset index unavailable at ${dataset.getPath()}`;
        debug.warn("auto-tag", msg);
        this.emitTask(taskId, "warning", msg, { source: "dataset", path: dataset.getPath() });
      }
      if (signal.aborted) return this.failCancelled(taskId);

      // Step 4: Cache check after local dataset, so stale cache never hides
      // newly indexed local data.
      debug.info("auto-tag", "Step 4/9: Checking cache...");
      update("Checking cache...", 4);
      if (signal.aborted) return this.failCancelled(taskId);
      const cachePath = this.config.cachePath ?? join(homedir(), ".auto-tagger", "cache.db");
      debug.debug("auto-tag", `Cache path: ${cachePath}`);
      const cache = new MatchCache(cachePath);
      try {
        const cached = cache.get(enhancedRequest);
        if (cached) {
          debug.info("auto-tag", `Cache HIT: ${cached.length} candidates`);
          this.emitTask(taskId, "source", `Cache: ${cached.length} candidate(s)`, {
            source: "cache",
            count: cached.length,
          });
          allCandidates.push(...cached);
        } else {
          debug.debug("auto-tag", "Cache MISS — proceeding with remote lookups");
        }
        if (signal.aborted) {
          cache.close();
          return this.failCancelled(taskId);
        }

        // Step 5: MusicBrainz
        if (this.config.remoteLookupEnabled !== false) {
          debug.info("auto-tag", "Step 5/9: Searching MusicBrainz...");
          update("Searching MusicBrainz...", 5);
          try {
            const mb = new MusicBrainzClient();
            debug.debug("auto-tag", `MusicBrainz search: artist="${enhancedRequest.artistHint}" album="${enhancedRequest.albumHint}"`);
            const mbCandidates = await this.searchVariants(mb, lookupVariants);
            debug.info("auto-tag", `MusicBrainz returned ${mbCandidates.length} candidates`);
            this.emitTask(taskId, "source", `MusicBrainz: ${mbCandidates.length} candidate(s)`, {
              source: "musicbrainz",
              count: mbCandidates.length,
            });
            allCandidates.push(...mbCandidates);
          } catch (err) {
            debug.warn("auto-tag", `MusicBrainz lookup failed (non-fatal)`, err);
            this.emitTask(taskId, "warning", "MusicBrainz lookup failed", { error: String(err) });
          }
        } else {
          debug.info("auto-tag", "Remote lookups disabled — skipping MusicBrainz");
        }
        if (signal.aborted) {
          cache.close();
          return this.failCancelled(taskId);
        }

        // Step 6: Discogs
        if (this.config.discogsEnabled !== false) {
          debug.info("auto-tag", "Step 6/9: Searching Discogs releases...");
          update("Searching Discogs...", 6);
          try {
            const discogs = new DiscogsClient({
              token: this.config.discogsToken,
            });
            debug.debug("auto-tag", `Discogs search: artist="${enhancedRequest.artistHint}" album="${enhancedRequest.albumHint}"`);
            const discogsCandidates = await this.searchVariants(discogs, lookupVariants);
            debug.info("auto-tag", `Discogs returned ${discogsCandidates.length} candidates`);
            this.emitTask(taskId, "source", `Discogs releases: ${discogsCandidates.length} candidate(s)`, {
              source: "discogs",
              count: discogsCandidates.length,
            });
            allCandidates.push(...discogsCandidates);
          } catch (err) {
            debug.warn("auto-tag", `Discogs lookup failed (non-fatal)`, err);
            this.emitTask(taskId, "warning", "Discogs lookup failed", { error: String(err) });
          }
        } else {
          debug.info("auto-tag", "Discogs disabled — skipping");
        }
        if (signal.aborted) {
          cache.close();
          return this.failCancelled(taskId);
        }

        // Step 7: Folder fallback (always included as safety net)
        debug.info("auto-tag", "Step 7/9: Building fallback candidate...");
        update("Building fallback...", 7);
        const folderCandidate = candidateFromFolder(enhancedRequest);
        allCandidates.push(folderCandidate);

        // Apply verification status
        for (const c of allCandidates) {
          c.verification = "match";
        }

        debug.info("auto-tag", `Total candidates across all sources: ${allCandidates.length}`);

        // Cache the results
        cache.set(enhancedRequest, allCandidates);
        debug.debug("auto-tag", "Cached results for future lookups");

        const mergedCandidates = this.mergeCandidateFields(allCandidates);
        this.emitTask(taskId, "merge", `Merged ${allCandidates.length} source candidate(s)`, {
          sourceCount: allCandidates.length,
          mergedCount: mergedCandidates.length,
        });

        // LLM selection
        update("Selecting candidate...", 8);
        const selected = await this.selectCandidate(
          enhancedRequest,
          mergedCandidates,
          signal,
        );
        const candidate = selected ?? mergedCandidates[0];
        if (candidate) {
          debug.info("auto-tag", "Step 9/9: Applying album tags...");
          update("Applying tags...", 9);
          await this.applyCandidateTags(taskId, enhancedRequest.path, candidate);
        }
        this.completeTask(taskId, candidate ?? mergedCandidates);
      } catch (err) {
        debug.error("auto-tag", `Unexpected error in lookup chain`, err);
        throw err;
      } finally {
        cache.close();
      }
    } catch (err) {
      debug.error("auto-tag", `processAlbum failed: ${err instanceof Error ? err.message : String(err)}`, err);
      throw err;
    } finally {
      debug.endTimer(`process-${taskId}`, "auto-tag", `processAlbum total for ${albumPath}`);
    }
  }

  /**
   * Optionally enhance ambiguous folder-name hints using LLM.
   */
  private async enhanceHints(
    request: ReturnType<typeof makeLookupRequest>,
    signal: AbortSignal,
  ): Promise<ReturnType<typeof makeLookupRequest>> {
    // Check if hints are ambiguous (same heuristic as Python)
    if (!this.hintsAreAmbiguous(request)) {
      debug.debug("auto-tag", "Hints not ambiguous — skipping LLM enhancement");
      return request;
    }

    if (!this.config.llmApiKey) {
      debug.debug("auto-tag", "No LLM API key — skipping hint enhancement");
      return request;
    }

    debug.info("auto-tag", `Enhancing ambiguous hints via LLM (model: ${this.config.llmModel ?? "default"})`);
    debug.startTimer("enhance-hints");

    try {
      const client = new OpenRouterClient({
        apiKey: this.config.llmApiKey,
        model: this.config.llmModel,
      });

      const folderName = request.path.split("/").pop() ?? "";
      const parentName =
        request.path.split("/").slice(-2, -1)[0] ?? null;

      debug.debug("auto-tag", `LLM extraction from folder: "${folderName}" parent: "${parentName}"`);

      const messages = buildFolderExtractionMessages(folderName, parentName);
      const schema = {
        type: "object",
        properties: {
          artist: { type: "string" },
          album: { type: "string" },
          year: { type: "string" },
          disc: { type: "string" },
        },
      };

      const response = await client.completeJson(
        messages,
        "FolderExtractionResponse",
        schema,
      );

      const extraction = response.data;
      debug.info("auto-tag", `LLM extracted: artist="${extraction.artist ?? ""}" album="${extraction.album ?? ""}" year="${extraction.year ?? ""}"`);

      return {
        ...request,
        artistHint: (extraction.artist as string) || request.artistHint,
        albumHint: (extraction.album as string) || request.albumHint,
        yearHint: (extraction.year as string) || request.yearHint,
      };
    } catch (err) {
      debug.warn("auto-tag", `LLM hint enhancement failed (non-fatal)`, err);
      // LLM enhancement failure is non-fatal
      return request;
    } finally {
      debug.endTimer("enhance-hints", "auto-tag", "LLM hint enhancement");
    }
  }

  /**
   * Check if deterministic parsing produced ambiguous hints.
   * Ported from Python lookup.py _hints_are_ambiguous.
   */
  private hintsAreAmbiguous(
    request: ReturnType<typeof makeLookupRequest>,
  ): boolean {
    const albumHint = request.albumHint ?? "";
    const artistHint = request.artistHint ?? "";

    if (!albumHint || !artistHint) return true;

    const folderName = request.path.split("/").pop() ?? "";

    // Bracket/bookmark annotations
    if (/[\[\]《》「」【】]/.test(folderName)) return true;

    // Chinese dot between CJK characters
    if (/[\u4e00-\u9fff]\.[\u4e00-\u9fff]/.test(folderName)) return true;
    if (folderName.includes("。")) return true;

    // Year prefix on album
    if (/^\d{4}[-.]/.test(albumHint)) return true;

    // Dot convention in CJK context
    if (albumHint.includes(".") && !request.yearHint) return true;

    return false;
  }

  /**
   * Write selected candidate's metadata tags to audio files in the album directory.
   * Applies album-level fields (artist, album, genre, etc.) to all files,
   * then matches per-track fields by track number.
   */
  private async applyCandidateTags(
    taskId: string,
    albumPath: string,
    candidate: AlbumCandidate,
  ): Promise<number> {
    debug.info("auto-tag", `Applying candidate tags from source="${candidate.source}" to: ${albumPath}`);
    debug.startTimer("apply-candidate-tags");

    const folderName = basename(dirname(albumPath));
    const albumArtists = splitArtistNames(candidate.albumArtists.length > 0 ? candidate.albumArtists : [folderName]);
    const albumArtist = artistDisplayName(albumArtists, folderName);
    const cover = this.findLocalCover(albumPath, candidate.album);

    // Discover audio files in the album directory
    let audioFiles: string[] = [];
    try {
      const entries = readdirSync(albumPath).sort();
      for (const entry of entries) {
        if (entry.startsWith(".")) continue;
        const fullPath = join(albumPath, entry);
        try {
          if (statSync(fullPath).isFile()) {
            const ext = extname(entry).toLowerCase();
            if (AUDIO_EXTENSIONS.has(ext)) {
              audioFiles.push(fullPath);
            }
          }
        } catch { /* skip unreadable */ }
      }
    } catch (err) {
      debug.error("auto-tag", `Cannot read album directory: ${albumPath}`, err);
      throw new Error(`Cannot read album directory: ${albumPath}`);
    }

    debug.debug("auto-tag", `Found ${audioFiles.length} audio files in ${albumPath}`);
    if (audioFiles.length === 0) return 0;

    // Build album-level fields applied to every file
    const albumFields: WriteFields = {};
    if (candidate.artist !== undefined) albumFields.artist = candidate.artist;
    albumFields.artists = candidate.artists.length > 0 ? candidate.artists : splitArtistNames([candidate.artist]);
    if (candidate.album !== undefined) albumFields.album = candidate.album;
    albumFields.albumArtist = albumArtist;
    albumFields.albumArtists = albumArtists;
    if (candidate.year !== undefined) albumFields.year = candidate.year;
    if (candidate.genre !== undefined) albumFields.genre = candidate.genre;
    if (candidate.musicbrainzAlbumId !== undefined) albumFields.musicbrainzAlbumId = candidate.musicbrainzAlbumId;
    if (candidate.musicbrainzArtistId !== undefined) albumFields.musicbrainzArtistId = candidate.musicbrainzArtistId;
    if (cover) {
      albumFields.coverData = cover.data;
      albumFields.coverMime = cover.mime;
      this.emitTask(taskId, "source", `Local cover: ${cover.path}`, { source: "cover", path: cover.path });
    }

    // Build track-level fields indexed by track number
    const trackMap = new Map<number, WriteFields>();
    for (const tc of candidate.tracks) {
      if (tc.trackNumber != null) {
        const fields: WriteFields = {};
        if (tc.title !== undefined) fields.title = tc.title;
        if (tc.artist !== undefined) fields.artist = tc.artist;
        if (tc.artists.length > 0) fields.artists = tc.artists;
        if (tc.trackNumber != null) fields.trackNumber = tc.trackNumber;
        if (tc.trackTotal != null) fields.trackTotal = tc.trackTotal;
        if (tc.discNumber != null) fields.discNumber = tc.discNumber;
        if (tc.discTotal != null) fields.discTotal = tc.discTotal;
        if (tc.musicbrainzTrackId != null) fields.musicbrainzTrackId = tc.musicbrainzTrackId;
        trackMap.set(tc.trackNumber, fields);
      }
    }

    // Write tags — album-level to every file, track-level when matching by position
    let written = 0;
    let errors = 0;
    for (let i = 0; i < audioFiles.length; i++) {
      const filePath = audioFiles[i];
      const trackNum = i + 1; // 1-based position fallback

      const trackFields = trackMap.get(trackNum) ?? {};
      const mergedFields: WriteFields = { ...albumFields, ...trackFields };
      const lyrics = this.findLocalLyrics(filePath);
      if (lyrics) {
        mergedFields.lyrics = lyrics;
        this.emitTask(taskId, "source", `Local lyrics: ${filePath}`, { source: "lyrics", path: filePath });
      }

      if (Object.keys(mergedFields).length === 0) {
        debug.debug("auto-tag", `No fields to write for: ${filePath} — skipping`);
        continue;
      }

      try {
        await writeTags(filePath, mergedFields);
        written++;
        this.emitTask(taskId, "write", `Wrote tags: ${basename(filePath)}`, {
          path: filePath,
          trackNumber: mergedFields.trackNumber ?? trackNum,
        });
        debug.debug("auto-tag", `Wrote tags: ${filePath}`);
      } catch (err) {
        errors++;
        debug.warn("auto-tag", `Failed to write tags to: ${filePath}`, err);
      }
    }

    debug.info("auto-tag", `Applied tags: ${written}/${audioFiles.length} files (${errors} errors)`);
    debug.endTimer("apply-candidate-tags", "auto-tag", `Tag application for ${albumPath}`);
    return written;
  }

  private async searchVariants(
    client: { searchAlbum(artist: string, album: string): Promise<AlbumCandidate[]> },
    variants: Array<[string, string]>,
  ): Promise<AlbumCandidate[]> {
    for (const [artist, album] of variants) {
      const candidates = await client.searchAlbum(artist, album);
      if (candidates.length > 0) return candidates;
    }
    return [];
  }

  private mergeCandidateFields(candidates: AlbumCandidate[]): AlbumCandidate[] {
    if (candidates.length === 0) return [];
    const preferred = candidates.find((c) => c.source === "dataset") ?? candidates[0];
    const merged = makeAlbumCandidate({
      source: preferred.source,
      verification: preferred.verification,
    });

    for (const candidate of candidates) {
      merged.artist ??= candidate.artist;
      if (merged.artists.length === 0) merged.artists = candidate.artists;
      merged.album ??= candidate.album;
      merged.albumArtist ??= candidate.albumArtist;
      if (merged.albumArtists.length === 0) merged.albumArtists = candidate.albumArtists;
      merged.year ??= candidate.year;
      merged.genre ??= candidate.genre;
      merged.musicbrainzAlbumId ??= candidate.musicbrainzAlbumId;
      merged.musicbrainzArtistId ??= candidate.musicbrainzArtistId;
      if (merged.tracks.length === 0 && candidate.tracks.length > 0) {
        merged.tracks = candidate.tracks;
      } else if (merged.tracks.length > 0 && candidate.tracks.length > 0) {
        this.fillTrackGaps(merged.tracks, candidate.tracks);
      }
    }

    const rest = candidates.filter((candidate) => candidate !== preferred);
    return [merged, ...rest];
  }

  private fillTrackGaps(target: TrackCandidate[], source: TrackCandidate[]): void {
    const byNumber = new Map<number, TrackCandidate>();
    for (const track of target) {
      if (track.trackNumber != null) byNumber.set(track.trackNumber, track);
    }
    for (const sourceTrack of source) {
      if (sourceTrack.trackNumber == null) continue;
      const targetTrack = byNumber.get(sourceTrack.trackNumber);
      if (!targetTrack) continue;
      targetTrack.title ??= sourceTrack.title;
      targetTrack.artist ??= sourceTrack.artist;
      if (targetTrack.artists.length === 0) targetTrack.artists = sourceTrack.artists;
      targetTrack.discNumber ??= sourceTrack.discNumber;
      targetTrack.discTotal ??= sourceTrack.discTotal;
      targetTrack.musicbrainzTrackId ??= sourceTrack.musicbrainzTrackId;
      targetTrack.length ??= sourceTrack.length;
    }
  }

  private findLocalCover(albumPath: string, albumName: string | null): {
    path: string;
    data: Buffer;
    mime: string;
  } | null {
    const names = [
      albumName,
      "cover",
      "folder",
      "front",
      "album",
    ].filter((name): name is string => !!name && name.trim().length > 0);
    const exts = [".jpg", ".jpeg", ".png"];
    for (const name of names) {
      for (const ext of exts) {
        const path = join(albumPath, `${name}${ext}`);
        if (!existsSync(path)) continue;
        return {
          path,
          data: readFileSync(path),
          mime: ext === ".png" ? "image/png" : "image/jpeg",
        };
      }
    }
    return null;
  }

  private findLocalLyrics(filePath: string): string | null {
    for (const ext of [".lrc", ".txt"]) {
      const lyricsPath = filePath.replace(/\.[^.]+$/, ext);
      if (!existsSync(lyricsPath)) continue;
      const data = readFileSync(lyricsPath);
      if (data.length >= 2 && data[0] === 0xff && data[1] === 0xfe) {
        return data.subarray(2).toString("utf16le");
      }
      if (data.length >= 2 && data[0] === 0xfe && data[1] === 0xff) {
        return Buffer.from(data.subarray(2)).swap16().toString("utf16le");
      }
      return data.toString("utf8");
    }
    return null;
  }

  /**
   * Use LLM to select the best candidate from the results.
   */
  private async selectCandidate(
    request: ReturnType<typeof makeLookupRequest>,
    candidates: AlbumCandidate[],
    signal: AbortSignal,
  ): Promise<AlbumCandidate | null> {
    if (!this.config.llmApiKey) {
      debug.debug("auto-tag", "No LLM API key — skipping candidate selection");
      return null;
    }
    if (candidates.length <= 1) {
      debug.debug("auto-tag", `Only ${candidates.length} candidate(s) — no selection needed`);
      return null;
    }
    if (signal.aborted) return null;

    debug.info("auto-tag", `Selecting best candidate from ${candidates.length} via LLM...`);
    debug.startTimer("select-candidate");

    try {
      const client = new OpenRouterClient({
        apiKey: this.config.llmApiKey,
        model: this.config.llmModel,
      });

      const messages = buildSelectionMessages(request, candidates);
      const schema = {
        type: "object",
        properties: {
          selectedIndex: { type: "number" },
          confidence: { type: "number" },
          reason: { type: "string" },
        },
        required: ["confidence", "reason"],
      };

      const response = await client.completeJson(
        messages,
        "CandidateSelectionResponse",
        schema,
      );

      const idx = response.data.selectedIndex as number | null;
      if (idx != null && idx >= 0 && idx < candidates.length) {
        debug.info("auto-tag", `LLM selected candidate ${idx}: confidence=${response.data.confidence}, reason="${response.data.reason}"`);
        return candidates[idx];
      } else {
        debug.warn("auto-tag", `LLM returned invalid index ${idx} — returning all candidates`);
      }
    } catch (err) {
      debug.warn("auto-tag", `LLM candidate selection failed (non-fatal)`, err);
      // Selection failure is non-fatal — return all candidates
    } finally {
      debug.endTimer("select-candidate", "auto-tag", "LLM candidate selection");
    }
    return null;
  }

  // ── Task helpers ────────────────────────────────────────────────

  private updateTask(
    taskId: string,
    update: Partial<TaskProgress>,
  ): void {
    const task = this.tasks.get(taskId);
    if (!task) return;
    task.progress = { ...task.progress, ...update };
    this.emitTask(taskId, "progress", task.progress.message);
  }

  private completeTask(
    taskId: string,
    result: unknown,
  ): void {
    debug.info("auto-tag", `Task ${taskId} completed`);
    this.updateTask(taskId, {
      status: "completed",
      progress: 9,
      message: "Complete",
      result,
    });
    this.emitTask(taskId, "completed", "Complete", result);
    // Clean up after 5 minutes
    setTimeout(() => {
      this.tasks.delete(taskId);
      debug.debug("auto-tag", `Task ${taskId} cleaned up`);
    }, 5 * 60 * 1000);
  }

  private failCancelled(taskId: string): void {
    debug.info("auto-tag", `Task ${taskId} cancelled`);
    this.updateTask(taskId, {
      status: "cancelled",
      message: "Cancelled",
    });
    this.emitTask(taskId, "cancelled", "Cancelled");
  }

  private emitTask(
    taskId: string,
    type: AutoTagEvent["type"],
    message: string,
    data?: unknown,
  ): void {
    const progress = this.tasks.get(taskId)?.progress;
    taskEvents.emit("event", {
      taskId,
      type,
      message,
      progress: progress?.progress ?? 0,
      total: progress?.total ?? 9,
      data,
    } satisfies AutoTagEvent);
  }
}

// ── Singleton ───────────────────────────────────────────────────────

let taskManager: TaskManager | null = null;

function getTaskManager(): TaskManager {
  if (!taskManager) {
    taskManager = new TaskManager();
  }
  return taskManager;
}

/** Enable/disable debug mode dynamically. */
export function setDebugMode(enabled: boolean): void {
  debug.setEnabled(enabled);
  const mgr = getTaskManager();
  const cfg = mgr.getConfig();
  cfg.debug = enabled;
  debug.info("config", `Debug mode set to: ${enabled}`);
}

/** Start auto-tagging an album. Returns a taskId string. */
export function startAutoTag(albumPath: string): string {
  const taskId = getTaskManager().startAutoTag(albumPath);
  debug.info("auto-tag", `Started auto-tag task ${taskId} for: ${albumPath}`);
  return taskId;
}

/** Get progress for a task. */
export function getProgress(
  taskId: string,
): TaskProgress | null {
  const progress = getTaskManager().getTaskProgress(taskId);
  if (progress) {
    debug.debug("auto-tag", `Progress [${taskId}]: ${progress.progress}/${progress.total} — ${progress.message}`);
  }
  return progress;
}

/** Cancel a running task. */
export function cancelTask(taskId: string): void {
  debug.info("auto-tag", `Cancelling task ${taskId}`);
  getTaskManager().cancelTask(taskId);
}

/** Get dataset status. */
export function getDatasetStatus(): {
  available: boolean;
  musicbrainz: boolean;
  totalRecords: number;
  lastUpdated: string | null;
} {
  const config = getTaskManager().getConfig();
  const reader = new DatasetReader(config.datasetPath);
  try {
    return reader.getStatus();
  } finally {
    reader.close();
  }
}

/** Get current config. */
export function getConfig(): Record<string, unknown> {
  const cfg = getTaskManager().getConfig();
  return {
    llmApiKey: cfg.llmApiKey ? "****" + cfg.llmApiKey.slice(-4) : null,
    llmModel: cfg.llmModel,
    discogsToken: cfg.discogsToken ? "****" + cfg.discogsToken.slice(-4) : null,
    remoteLookupEnabled: cfg.remoteLookupEnabled,
    discogsEnabled: cfg.discogsEnabled,
    debug: cfg.debug ?? false,
  };
}

/** Refresh config from file/env. */
export function refreshConfig(): void {
  getTaskManager().refreshConfig();
}

// ── Config key mapping ────────────────────────────────────────────

const CONFIG_KEY_MAP: Record<string, string> = {
  llmApiKey: "llm_api_key",
  llmModel: "llm_model",
  discogsToken: "discogs_token",
  remoteLookupEnabled: "remote_lookup_enabled",
  discogsEnabled: "discogs_enabled",
  debug: "debug",
};

/**
 * Save a single config value back to the YAML config file.
 * Finds the first existing config file, or creates ~/.auto-tagger/config.yaml.
 * Handles simple flat YAML (one level, no nested structures).
 */
export function saveConfig(key: string, value: unknown): void {
  const yamlKey = CONFIG_KEY_MAP[key];
  if (!yamlKey) return; // Unknown key, skip

  // Find or determine config file path
  let configPath: string | null = null;
  for (const p of getConfigPaths()) {
    if (existsSync(p)) {
      configPath = p;
      break;
    }
  }

  // If no config file exists, use ~/.auto-tagger/config.yaml
  if (!configPath) {
    configPath = getConfigPaths()[0]; // ~/.auto-tagger/config.yaml
  }

  // Ensure parent directory exists
  const parentDir = join(configPath, "..");
  if (!existsSync(parentDir)) {
    mkdirSync(parentDir, { recursive: true });
  }

  // Format the value for YAML
  const formattedValue = formatYamlValue(value);

  try {
    let lines: string[] = [];
    let existingContent = "";
    try {
      existingContent = readFileSync(configPath, "utf-8") || "";
    } catch {
      // File doesn't exist yet
    }
    lines = existingContent ? existingContent.split("\n") : [];

    // Check if the key already exists and update it
    let found = false;
    const updatedLines = lines.map((line) => {
      const trimmed = line.trim();
      if (!trimmed || trimmed.startsWith("#")) return line;
      const colonIdx = trimmed.indexOf(":");
      if (colonIdx === -1) return line;
      const existingKey = trimmed.slice(0, colonIdx).trim();
      if (existingKey === yamlKey) {
        found = true;
        return line.replace(/:.+/, ": " + formattedValue);
      }
      return line;
    });

    // If key wasn't found, append it
    if (!found) {
      updatedLines.push(`${yamlKey}: ${formattedValue}`);
    }

    writeFileSync(configPath, updatedLines.join("\n") + "\n", "utf-8");
  } catch (error) {
    console.error(`Failed to save config key ${key}:`, error);
  }
}

function formatYamlValue(value: unknown): string {
  if (typeof value === "boolean") {
    return value ? "true" : "false";
  }
  if (value === null || value === undefined) {
    return "null";
  }
  const str = String(value);
  // Quote if contains spaces, colons, or special chars
  if (/[\s:#]/.test(str)) {
    return `"${str.replace(/"/g, '\\"')}"`;
  }
  return str;
}
