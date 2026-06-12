/**
 * Auto-tag orchestrator - implements the full lookup chain.
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
  makeAlbumCandidate,
  makeLookupRequest,
  splitArtistNames,
  verifyAlbumName,
} from "./candidates";
import { MatchCache } from "./cache";
import { DatasetReader } from "./dataset";
import { MusicBrainzClient } from "./musicbrainz";
import { DiscogsClient } from "./discogs";
import { OpenRouterClient } from "./openrouter";
import { parseAlbumWithTags, candidateFromFolder } from "./fallback";
import { buildGenreFillMessages, buildTagCorrectionMessages } from "./prompts";
import { getAllNameVariants } from "./aliases";
import { writeTags, type WriteFields } from "./writer";
import { matchRemoteCandidateTracks } from "../services/RemoteTrackMatcher";
import { readLocalLyrics, LyricsClient } from "./lyrics";
import { readTrackMetadata } from "./tracks";
import { existsSync, readFileSync, writeFileSync, mkdirSync, readdirSync, statSync } from "node:fs";
import { basename, dirname, join, extname } from "node:path";

const AUDIO_EXTENSIONS = new Set([".mp3", ".flac", ".m4a", ".mp4", ".wav", ".ogg", ".opus", ".aiff"]);
import { homedir } from "node:os";
import debug from "./debug";
import { AUTO_TAG_ALBUM_CONCURRENCY, LOCAL_READ_CONCURRENCY } from "../services/concurrency";
import { getDefaultWriteQueue } from "../services/TagWriteQueue";

/**
 * Get sorted audio filenames from an album path.
 * Returns only the basenames (not full paths) in filesystem sort order.
 */
function getSortedAudioFilenames(albumPath: string): string[] {
  if (!albumPath || albumPath === ".") return [];
  try {
    return readdirSync(albumPath)
      .filter((e) => !e.startsWith(".") && AUDIO_EXTENSIONS.has(extname(e).toLowerCase()))
      .sort();
  } catch {
    return [];
  }
}

export function filterCandidatesForAutoApply(
  request: ReturnType<typeof makeLookupRequest>,
  candidates: AlbumCandidate[],
): AlbumCandidate[] {
  return candidates.filter((candidate) => {
    const verification = verifyAlbumName(request.albumHint, candidate);
    candidate.verification = verification;
    return verification !== "mismatch";
  });
}

const REMOTE_TRACK_SOURCES = new Set<AlbumCandidate["source"]>(["dataset", "discogs", "musicbrainz"]);

/**
 * Replace the coarse positional guard with deterministic per-file matching.
 * Uses matchRemoteCandidateTracks to align remote tracks to local files by
 * title + duration evidence. Unmatched tracks keep their local metadata.
 */
export async function protectCandidateTrackFieldsForAutoApply(
  request: ReturnType<typeof makeLookupRequest>,
  candidates: AlbumCandidate[],
): Promise<AlbumCandidate[]> {
  // Discover sorted audio filenames for filename-derived title matching
  const albumPath = extname(basename(request.path)) ? dirname(request.path) : request.path;
  const filenames = getSortedAudioFilenames(albumPath);

  return Promise.all(candidates.map(async (candidate) => {
    if (!REMOTE_TRACK_SOURCES.has(candidate.source)) return candidate;

    const matcher = await matchRemoteCandidateTracks(
      request.tracks,
      filenames,
      candidate.tracks,
      candidate.source,
      {
        artistHints: [
          request.artistHint,
          candidate.artist,
          candidate.albumArtist,
          ...candidate.artists,
          ...candidate.albumArtists,
        ].filter((artist): artist is string => !!artist?.trim()),
      },
    );

    // Log when tracklists don't fully align
    if (!matcher.isFullOrderedMatch && request.tracks.length > 0 && candidate.tracks.length > 0) {
      const skipSummary = matcher.stats.skipped
        .map((s) => `${s.localIndex}:${s.reason}`)
        .join(", ");
      debug.warn(
        "auto-tag",
        `Tracklist mismatch for source="${candidate.source}" album="${candidate.album ?? "?"}": ` +
        `matched=${matcher.stats.matched} local=${matcher.stats.local} ` +
        `remote=${matcher.stats.remote} skipped=[${skipSummary}]`,
      );
    }

    return { ...candidate, tracks: matcher.tracks };
  }));
}

function pathSegments(inputPath: string): string[] {
  return inputPath.split(/[\\/]+/).filter((segment) => segment.length > 0);
}

/**
 * Check if deterministic parsing produced ambiguous hints that warrant
 * LLM enhancement. Ported from Python lookup.py _hints_are_ambiguous.
 *
 * Returns true when the folder name has bracket annotations, Chinese dots,
 * year-prefixed album hints, or other patterns that the basic folder-name
 * parser might misinterpret.
 *
 * Known format suffixes like "[flac]", "[MP3]" are stripped before the
 * bracket check - they are not ambiguous naming conventions.
 */
export function hintsAreAmbiguous(
  albumHint: string | null | undefined,
  artistHint: string | null | undefined,
  path: string,
  yearHint: string | null | undefined,
): boolean {
  const aHint = albumHint ?? "";
  const arHint = artistHint ?? "";

  if (!aHint || !arHint) return true;

  const folderName = pathSegments(path).pop() ?? "";

  // Strip known format suffixes (e.g. "[flac]", "[FLAC]", "[MP3]") before
  // checking brackets - these are not ambiguous naming conventions.
  const cleanName = folderName.replace(
    /\[?(flac|mp3|wav|aac|ogg|m4a|wma|ape|flac\s*分轨|wav\s*分轨)\]?\s*$/i,
    "",
  );

  // Bracket/bookmark annotations (after stripping format suffixes)
  if (/[\[\]《》「」【】]/.test(cleanName)) return true;

  // Chinese dot between CJK characters
  if (/[\u4e00-\u9fff]\.[\u4e00-\u9fff]/.test(folderName)) return true;
  if (folderName.includes("。")) return true;

  // Year prefix on album
  if (/^\d{4}[-.]/.test(aHint)) return true;

  // Dot convention in CJK context
  if (aHint.includes(".") && !yearHint) return true;

  return false;
}

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
  lyricsDownloadEnabled?: boolean;
  lyricsApiUrl?: string;
  googleImageApiKey?: string;
  googleImageSearchEngineId?: string;
  googleImageEnabled?: boolean;
  theAudioDbApiKey?: string;
  theAudioDbEnabled?: boolean;
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
  const parseBoolOrNull = (v: string): boolean | null =>
    v === "true" ? true : v === "false" ? false : null;

  const configSetters: Record<
    string,
    (value: string) => void
  > = {
    llm_api_key: (v) => { config.llmApiKey = v; },
    llm_model: (v) => { config.llmModel = v; },
    discogs_token: (v) => { config.discogsToken = v; },
    dataset_path: (v) => { config.datasetPath = v; },
    remote_lookup_enabled: (v) => {
      const b = parseBoolOrNull(v);
      if (b !== null) config.remoteLookupEnabled = b;
    },
    discogs_enabled: (v) => {
      const b = parseBoolOrNull(v);
      if (b !== null) config.discogsEnabled = b;
    },
    debug: (v) => {
      const b = parseBoolOrNull(v);
      if (b !== null) config.debug = b;
    },
    lyrics_download_enabled: (v) => {
      const b = parseBoolOrNull(v);
      if (b !== null) config.lyricsDownloadEnabled = b;
    },
    lyrics_api_url: (v) => { config.lyricsApiUrl = v; },
    google_image_api_key: (v) => { config.googleImageApiKey = v; },
    google_image_search_engine_id: (v) => { config.googleImageSearchEngineId = v; },
    google_image_enabled: (v) => {
      const b = parseBoolOrNull(v);
      if (b !== null) config.googleImageEnabled = b;
    },
    theaudiodb_api_key: (v) => { config.theAudioDbApiKey = v; },
    theaudiodb_enabled: (v) => {
      const b = parseBoolOrNull(v);
      if (b !== null) config.theAudioDbEnabled = b;
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
  if (process.env.LLM_API_KEY) config.llmApiKey = process.env.LLM_API_KEY;
  if (process.env.LLM_MODEL) config.llmModel = process.env.LLM_MODEL;
  if (process.env.AUTO_TAG_DISCOGS_TOKEN) config.discogsToken = process.env.AUTO_TAG_DISCOGS_TOKEN;
  if (process.env.AUTO_TAG_REMOTE_LOOKUP === "false") config.remoteLookupEnabled = false;
  if (process.env.AUTO_TAG_DISCOGS_ENABLED === "false") config.discogsEnabled = false;
  if (process.env.AUTO_TAG_DEBUG === "true") config.debug = true;
  if (process.env.AUTO_TAG_LYRICS_DOWNLOAD_ENABLED === "false") config.lyricsDownloadEnabled = false;
  if (process.env.AUTO_TAG_LYRICS_API_URL) config.lyricsApiUrl = process.env.AUTO_TAG_LYRICS_API_URL;
  if (process.env.GOOGLE_IMAGE_API_KEY) config.googleImageApiKey = process.env.GOOGLE_IMAGE_API_KEY;
  if (process.env.GOOGLE_IMAGE_SEARCH_ENGINE_ID) config.googleImageSearchEngineId = process.env.GOOGLE_IMAGE_SEARCH_ENGINE_ID;
  if (process.env.GOOGLE_IMAGE_ENABLED === "false") config.googleImageEnabled = false;
  if (process.env.THEAUDIODB_API_KEY) config.theAudioDbApiKey = process.env.THEAUDIODB_API_KEY;
  if (process.env.THEAUDIODB_ENABLED === "false") config.theAudioDbEnabled = false;

  // Sync debug logger with config
  debug.setEnabled(!!config.debug);

  // Log which env vars were loaded (logger is now active)
  if (config.llmApiKey) {
    const masked = config.llmApiKey.slice(0, 8) + "...";
    debug.info("config", `LLM_API_KEY loaded from env (${masked})`);
  }
  if (config.llmModel) debug.info("config", `LLM_MODEL loaded from env: ${config.llmModel}`);
  if (config.discogsToken) debug.info("config", "AUTO_TAG_DISCOGS_TOKEN loaded from env");
  if (config.googleImageApiKey) debug.info("config", "GOOGLE_IMAGE_API_KEY loaded from env");
  if (config.theAudioDbApiKey) debug.info("config", "THEAUDIODB_API_KEY loaded from env");
  if (config.debug) debug.info("config", "AUTO_TAG_DEBUG loaded from env - debug mode enabled");

  return config;
}

// ── Aliased Lookup Variants ──────────────────────────────────────────

/**
 * Build lookup variant pairs that include known artist name aliases.
 * For Chinese artists who use English names (e.g. 周杰伦 → "Jay Chou", "TS"),
 * this ensures MusicBrainz and Discogs are queried with all known name forms.
 *
 * Aliases are loaded from the persisted alias file (artist-aliases.json),
 * which is self-learned from LLM fallback results.
 *
 * The cross-product of artist variants × album variants is generated:
 *   - Latin-script aliases first (uppercase-initial before lowercase-initial,
 *     then by descending length)
 *   - Script variants (Simplified/Traditional Chinese via opencc-js)
 *   - The original name
 *   - Non-Latin aliases
 */
/**
 * Safely fetch name variants (aliases + SC/TC), falling back to original.
 */
async function safeGetNameVariants(text: string): Promise<string[]> {
  try {
    const variants = text ? await getAllNameVariants(text) : [text];
    return variants.length > 0 ? variants : [text];
  } catch {
    return [text];
  }
}

export async function buildAliasedLookupVariants(
  artistHint: string | null | undefined,
  albumHint: string | null | undefined,
): Promise<Array<[string, string]>> {
  const pairs: Array<[string, string]> = [];
  const addPair = (a: string, b: string) => {
    if (!pairs.some(([x, y]) => x === a && y === b)) {
      pairs.push([a, b]);
    }
  };

  const artistText = artistHint ?? "";
  const albumText = albumHint ?? "";

  const artistVariants = await safeGetNameVariants(artistText);
  const albumVariants = await safeGetNameVariants(albumText);

  // Generate cross-product: every artist variant × every album variant.
  // This ensures that e.g. "Jay Chou" + "叶惠美" is tried,
  // not just "周杰伦" + "叶惠美".
  for (const artist of artistVariants) {
    for (const album of albumVariants) {
      addPair(artist, album);
    }
  }

  return pairs;
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

  // ── Concurrency-limited task queue ──────────────────────────────
  /** Maximum concurrent album processing tasks. */
  private maxConcurrency = AUTO_TAG_ALBUM_CONCURRENCY;
  /** Number of currently running album tasks. */
  private runningCount = 0;
  /** Queue of pending album tasks: { taskId, albumPath, abort, queued: true }. */
  private pendingQueue: Array<{
    taskId: string;
    albumPath: string;
    abort: AbortController;
  }> = [];

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
   * The task is queued if the concurrency limit is reached.
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
        message: this.runningCount >= this.maxConcurrency
          ? "Queued (waiting for slot)..."
          : "Starting...",
        result: null,
      },
      abort,
    });

    if (this.runningCount >= this.maxConcurrency) {
      // Queue the task — it will start when a slot opens
      debug.info("auto-tag", `Task ${taskId} queued for ${albumPath} (running=${this.runningCount}, max=${this.maxConcurrency})`);
      this.pendingQueue.push({ taskId, albumPath, abort });
    } else {
      this.startTask(taskId, albumPath, abort);
    }

    return taskId;
  }

  /**
   * Start a task immediately (increments running count).
   */
  private startTask(taskId: string, albumPath: string, abort: AbortController): void {
    this.runningCount++;
    debug.debug("auto-tag", `Task ${taskId} started (running=${this.runningCount})`);

    this.processAlbum(taskId, albumPath, abort.signal)
      .catch((err) => {
        const msg = err instanceof Error ? err.message : "Unknown error";
        debug.error("auto-tag", `Task ${taskId} failed: ${msg}`, err);
        if (!abort.signal.aborted) {
          this.updateTask(taskId, {
            status: "failed",
            message: msg,
          });
          this.emitTask(taskId, "failed", msg, { error: msg });
        }
      })
      .finally(() => {
        this.runningCount--;
        // Dequeue the next pending task
        this.dequeueNext();
      });
  }

  /**
   * Start the next queued task if concurrency allows.
   */
  private dequeueNext(): void {
    while (this.pendingQueue.length > 0 && this.runningCount < this.maxConcurrency) {
      const next = this.pendingQueue.shift()!;
      if (next.abort.signal.aborted) {
        // Task was cancelled while queued — skip it
        this.updateTask(next.taskId, {
          status: "cancelled",
          message: "Cancelled (queued)",
        });
        this.emitTask(next.taskId, "cancelled", "Cancelled before starting");
        this.cleanupTask(next.taskId);
        continue;
      }
      this.updateTask(next.taskId, {
        message: "Starting...",
        progress: 0,
      });
      this.startTask(next.taskId, next.albumPath, next.abort);
    }
  }

  getTaskProgress(taskId: string): TaskProgress | null {
    return this.tasks.get(taskId)?.progress ?? null;
  }

  cancelTask(taskId: string): void {
    // Check if the task is still queued (not yet started)
    const queueIndex = this.pendingQueue.findIndex((q) => q.taskId === taskId);
    if (queueIndex >= 0) {
      const [queued] = this.pendingQueue.splice(queueIndex, 1);
      queued.abort.abort();
      this.updateTask(taskId, {
        status: "cancelled",
        message: "Cancelled (queued)",
      });
      this.emitTask(taskId, "cancelled", "Cancelled before starting");
      this.cleanupTask(taskId);
      return;
    }

    // Task is running (or finished) — abort the signal
    const task = this.tasks.get(taskId);
    if (task) {
      task.abort.abort();
      this.updateTask(taskId, {
        status: "cancelled",
        message: "Cancelled",
      });
      this.emitTask(taskId, "cancelled", "Cancelled");
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

      // Fail fast if album directory does not exist
      // (prevents wasting 30-60s running the full pipeline on stale paths)
      if (!existsSync(albumPath)) {
        throw new Error(`Album directory does not exist: ${albumPath}`);
      }

      if (signal.aborted) return this.failCancelled(taskId);
      const request = await parseAlbumWithTags(albumPath);
      debug.info("auto-tag", `Parsed hints: artist="${request.artistHint}" album="${request.albumHint}" year="${request.yearHint}"`);

      // Step 2: LLM tag resolution - uses folder name + existing file metadata
      // to produce corrected search hints AND a fallback candidate with genre.
      debug.info("auto-tag", "Step 2/9: Resolving tags via LLM...");
      update("Resolving tags via LLM...", 2);
      if (signal.aborted) return this.failCancelled(taskId);
      const { correctedRequest, fallbackCandidate: llmFallback } = await this.resolveTagsViaLLM(request, signal);
      const hintsChanged = request.artistHint !== correctedRequest.artistHint || request.albumHint !== correctedRequest.albumHint;
      if (hintsChanged) {
        debug.info("auto-tag", `LLM corrected: artist="${correctedRequest.artistHint}" album="${correctedRequest.albumHint}"`);
      } else {
        debug.debug("auto-tag", "Hints unchanged after LLM check");
      }

      // Step 3: Local staging-derived dataset index must be checked first.
      debug.info("auto-tag", "Step 3/9: Querying local dataset...");
      update("Querying local dataset...", 3);
      if (signal.aborted) return this.failCancelled(taskId);
      let allCandidates: AlbumCandidate[] = [];
      const lookupVariants = await buildAliasedLookupVariants(
        correctedRequest.artistHint,
        correctedRequest.albumHint,
      );
      const dataset = new DatasetReader(this.config.datasetPath);
      if (dataset.isAvailable() && dataset.hasLookupTable()) {
        debug.debug("auto-tag", `Dataset available at: ${dataset.getPath()}`);
        try {
          const datasetCandidates = dataset.queryAlbum(
            correctedRequest.artistHint ?? "",
            correctedRequest.albumHint ?? "",
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
        const cached = cache.get(correctedRequest);
        if (cached) {
          debug.info("auto-tag", `Cache HIT: ${cached.length} candidates`);
          this.emitTask(taskId, "source", `Cache: ${cached.length} candidate(s)`, {
            source: "cache",
            count: cached.length,
          });
          allCandidates.push(...cached);
        } else {
          debug.debug("auto-tag", "Cache MISS - proceeding with remote lookups");
        }
        if (signal.aborted) {
          return this.failCancelled(taskId);
        }

        // Step 4b: Direct provider ID lookups (before name-based search)
        // If the file tags contained provider IDs, bypass the generic search
        // and fetch the release directly from the provider's API.
        const directLookups = await this.performDirectIdLookups(correctedRequest);
        if (directLookups.length > 0) {
          debug.info("auto-tag", `Direct ID lookups returned ${directLookups.length} candidate(s)`);
          this.emitTask(taskId, "source", `Direct ID lookup: ${directLookups.length} candidate(s)`, {
            source: "direct-id",
            count: directLookups.length,
          });
          allCandidates.push(...directLookups);
        }
        if (signal.aborted) {
          return this.failCancelled(taskId);
        }

        // Step 5: MusicBrainz
        if (this.config.remoteLookupEnabled !== false) {
          debug.info("auto-tag", "Step 5/9: Searching MusicBrainz...");
          update("Searching MusicBrainz...", 5);
          try {
            const mb = new MusicBrainzClient();
            debug.debug("auto-tag", `MusicBrainz search: artist="${correctedRequest.artistHint}" album="${correctedRequest.albumHint}"`);
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
          debug.info("auto-tag", "Remote lookups disabled - skipping MusicBrainz");
        }
        if (signal.aborted) {
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
            debug.debug("auto-tag", `Discogs search: artist="${correctedRequest.artistHint}" album="${correctedRequest.albumHint}"`);
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
          debug.info("auto-tag", "Discogs disabled - skipping");
        }
        if (signal.aborted) {
          return this.failCancelled(taskId);
        }

        // Step 7: Fallback candidate
        // Always include the LLM fallback (with genre + corrected metadata) when
        // available, even if API lookups returned candidates. The LLM selection
        // step (8) can then pick the best from the full pool.
        debug.info("auto-tag", "Step 7/9: Building fallback candidate...");
        update("Building fallback...", 7);
        if (llmFallback) {
          debug.debug("auto-tag", `Adding LLM fallback candidate (genre="${llmFallback.genre ?? ""}")`);
          allCandidates.push(llmFallback);
        }
        // Always add folder-based fallback as lowest-priority safety net
        const folderCandidate = candidateFromFolder(correctedRequest);
        allCandidates.push(folderCandidate);

        allCandidates = await protectCandidateTrackFieldsForAutoApply(
          correctedRequest,
          filterCandidatesForAutoApply(correctedRequest, allCandidates),
        );

        debug.info("auto-tag", `Total candidates across all sources: ${allCandidates.length}`);

        // Cache the results
        cache.set(correctedRequest, allCandidates);
        debug.debug("auto-tag", "Cached results for future lookups");

        const mergedCandidates = this.mergeCandidateFields(allCandidates);
        this.emitTask(taskId, "merge", `Merged ${allCandidates.length} source candidate(s)`, {
          sourceCount: allCandidates.length,
          mergedCount: mergedCandidates.length,
        });

        // Use the merged candidate directly (it already contains the best data
        // from all sources, filled in by mergeCandidateFields). No LLM selection
        // step - that was a fragile bottleneck (HTTP 400 failures).
        const candidate = mergedCandidates.length > 0 ? mergedCandidates[0] : null;
        if (candidate) {
          // Conditional genre fill: only call LLM if genre is still missing
          // after Discogs and LLM tag resolution.
          update("Resolving genre...", 8);
          const filledCandidate = await this.fillGenreIfMissing(candidate, correctedRequest, signal);
          debug.info("auto-tag", `Step 8/9: Genre resolved: "${filledCandidate.genre ?? "(none)"}"`);

          debug.info("auto-tag", "Step 9/9: Applying album tags...");
          update("Applying tags...", 9);
          await this.applyCandidateTags(taskId, correctedRequest.path, filledCandidate);
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
   * Resolve correct tags via LLM by analyzing folder name, parent folder,
   * basic parser hints, AND existing file metadata.
   *
   * Returns:
   *   - correctedRequest: LookupRequest with cleaned artist/album/year hints
   *     for use in API search queries.
   *   - fallbackCandidate: AlbumCandidate with genre and other fields populated,
   *     used when all API lookups return 0 candidates.
   *
   * When LLM is unavailable (no API key) or fails, returns the original request
   * and null fallback (preserving old folder-fallback behavior).
   */
  private async resolveTagsViaLLM(
    request: ReturnType<typeof makeLookupRequest>,
    signal: AbortSignal,
  ): Promise<{
    correctedRequest: ReturnType<typeof makeLookupRequest>;
    fallbackCandidate: AlbumCandidate | null;
  }> {
    if (!this.config.llmApiKey) {
      debug.debug("auto-tag", "No LLM API key - skipping LLM tag resolution");
      return { correctedRequest: request, fallbackCandidate: null };
    }

    debug.info("auto-tag", `Resolving tags via LLM (model: ${this.config.llmModel ?? "default"})`);
    debug.startTimer("resolve-tags");

    try {
      const client = new OpenRouterClient({
        apiKey: this.config.llmApiKey,
        model: this.config.llmModel,
      });

      const pathParts = pathSegments(request.path);
      const folderName = pathParts.at(-1) ?? "";
      const parentName = pathParts.at(-2) ?? null;

      debug.debug(
        "auto-tag",
        `LLM tag resolution input: folder="${folderName}" parent="${parentName}" ` +
        `hints=(${request.artistHint}, ${request.albumHint}, ${request.yearHint}) ` +
        `tracks=${request.tracks.length}`,
      );

      const messages = buildTagCorrectionMessages(
        folderName,
        parentName,
        request.artistHint,
        request.albumHint,
        request.yearHint,
        request.tracks.map((t) => ({
          title: t.title,
          artist: t.artist,
          album: null,
          trackNumber: t.trackNumber,
          genre: t.genre,
        })),
      );

      const schema = {
        type: "object",
        properties: {
          artist: { type: "string" },
          albumArtist: { type: "string" },
          album: { type: "string" },
          year: { type: "string" },
          genre: { type: "string" },
          tracks: {
            type: "array",
            items: {
              type: "object",
              properties: {
                index: { type: "number" },
                title: { type: "string" },
                artist: { type: "string" },
              },
            },
          },
          confidence: { type: "number" },
        },
      };

      const response = await client.completeJson(
        messages,
        "TagCorrectionResponse",
        schema,
      );

      const data = response.data as Record<string, unknown>;
      debug.info(
        "auto-tag",
        `LLM resolved: artist="${data.artist ?? ""}" album="${data.album ?? ""}" ` +
        `year="${data.year ?? ""}" genre="${data.genre ?? ""}" ` +
        `confidence=${String(data.confidence ?? "")}`,
      );

      // Build corrected request with cleaned hints for API search
      const correctedRequest = {
        ...request,
        artistHint: (data.artist as string) || request.artistHint,
        albumHint: (data.album as string) || request.albumHint,
        yearHint: (data.year as string) || request.yearHint,
      };

      // Build fallback candidate with genre (used when APIs return nothing).
      // Normalize LLM track indices: some models return 1-based indices
      // instead of 0-based, which causes title-to-file mapping drift.
      const rawTracks = Array.isArray(data.tracks)
        ? (data.tracks as Array<Record<string, unknown>>)
        : [];
      const llmTracks = normalizeLlmTrackIndices(rawTracks, request.tracks.length);
      const fallbackCandidate: AlbumCandidate = {
        artist: (data.artist as string) || request.artistHint,
        artists: splitArtistNames([(data.artist as string) || request.artistHint]),
        album: (data.album as string) || request.albumHint,
        albumArtist: (data.albumArtist as string) || (data.artist as string) || request.artistHint,
        albumArtists: splitArtistNames([(data.albumArtist as string) || (data.artist as string) || request.artistHint]),
        year: (data.year as string) || request.yearHint || null,
        genre: (data.genre as string) || null,
        musicbrainzAlbumId: null,
        musicbrainzArtistId: null,
        discogsArtistId: null,
        discogsReleaseId: null,
        tracks: request.tracks.length > 0
          ? request.tracks.map((t, i) => {
              const llmTrack = llmTracks[i];
              return {
                ...t,
                title: (llmTrack?.title as string) || t.title,
                artist: (llmTrack?.artist as string) || t.artist || (data.artist as string) || request.artistHint,
              };
            })
          : request.tracks,
        distance: null,
        source: "llm",
        verification: null,
      };

      debug.debug(
        "auto-tag",
        `LLM fallback candidate: artist="${fallbackCandidate.artist}" ` +
        `album="${fallbackCandidate.album}" genre="${fallbackCandidate.genre}" ` +
        `tracks=${fallbackCandidate.tracks.length}`,
      );

      return { correctedRequest, fallbackCandidate };
    } catch (err) {
      debug.warn("auto-tag", `LLM tag resolution failed (non-fatal)`, err);
      // LLM failure is non-fatal - fall back to original hints and folder fallback
      return { correctedRequest: request, fallbackCandidate: null };
    } finally {
      debug.endTimer("resolve-tags", "auto-tag", "LLM tag resolution");
    }
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
    // NOTE: artist/artists are per-track fields - they must NOT be set at the
    // album level. Setting them here would overwrite per-track artist data
    // for multi-disc compilations where file position ≠ track number.
    const albumFields: WriteFields = {};
    if (candidate.album !== undefined) albumFields.album = candidate.album;
    albumFields.albumArtist = albumArtist;
    albumFields.albumArtists = albumArtists;
    if (candidate.year !== undefined) albumFields.year = candidate.year;
    if (candidate.genre) albumFields.genre = candidate.genre;
    if (candidate.musicbrainzAlbumId !== undefined) albumFields.musicbrainzAlbumId = candidate.musicbrainzAlbumId;
    if (candidate.musicbrainzArtistId !== undefined) albumFields.musicbrainzArtistId = candidate.musicbrainzArtistId;
    if (cover) {
      albumFields.coverData = cover.data;
      albumFields.coverMime = cover.mime;
      this.emitTask(taskId, "source", `Local cover: ${cover.path}`, { source: "cover", path: cover.path });
    }

    // Build track-level fields, preserving input order.
    // Positional matching avoids Map<number, ...> which silently drops
    // duplicate track numbers (common on multi-disc compilations).
    // Both arrays are filename-sorted identically, so index i is correct.
    const trackFieldsList: WriteFields[] = candidate.tracks.map((tc) => {
      const fields: WriteFields = {};
      if (tc.title !== undefined) fields.title = tc.title;
      if (tc.artist !== undefined) fields.artist = tc.artist;
      if (tc.artists.length > 0) fields.artists = splitArtistNames(tc.artists);
      if (tc.trackNumber != null) fields.trackNumber = tc.trackNumber;
      if (tc.trackTotal != null) fields.trackTotal = tc.trackTotal;
      if (tc.discNumber != null) fields.discNumber = tc.discNumber;
      if (tc.discTotal != null) fields.discTotal = tc.discTotal;
      if (tc.musicbrainzTrackId != null) fields.musicbrainzTrackId = tc.musicbrainzTrackId;
      return fields;
    });

    // Write tags - album-level to every file, track-level matched by
    // position in the sorted directory listing.
    // All writes go through the concurrent write queue for bounded parallelism.

    // 1. Collect all write jobs (with lyrics resolution)
    const writeJobs: Array<{ filePath: string; fields: WriteFields }> = [];
    for (let i = 0; i < audioFiles.length; i++) {
      const filePath = audioFiles[i];
      const trackFields = i < trackFieldsList.length ? trackFieldsList[i] : {};
      const mergedFields: WriteFields = { ...albumFields, ...trackFields };

      // 1. Read local lyrics (with encoding fix)
      let lyrics = readLocalLyrics(filePath);
      if (lyrics) {
        mergedFields.lyrics = lyrics;
        this.emitTask(taskId, "source", `Local lyrics: ${filePath}`, { source: "lyrics", path: filePath });
      }

      // 2. If no local file and download is enabled, fetch from API
      if (!lyrics && this.config.lyricsDownloadEnabled) {
        const trackName = mergedFields.title;
        const artistName = mergedFields.artist ?? albumFields.albumArtist ?? folderName;
        const downloaded = await this.fetchTrackLyrics(taskId, trackName, artistName, mergedFields.album);
        if (downloaded) {
          mergedFields.lyrics = downloaded;
        }
      }

      if (Object.keys(mergedFields).length === 0) {
        debug.debug("auto-tag", `No fields to write for: ${filePath} - skipping`);
        continue;
      }
      writeJobs.push({ filePath, fields: mergedFields });
    }

    // 2. Submit all write jobs through the concurrent queue
    const writeResults = await getDefaultWriteQueue().submit(writeJobs);
    const written = writeResults.filter((r) => r.success).length;
    const errors = writeResults.filter((r) => !r.success).length;

    for (const r of writeResults) {
      if (r.success) {
        this.emitTask(taskId, "write", `Wrote tags: ${basename(r.filePath)}`, {
          path: r.filePath,
        });
        debug.debug("auto-tag", `Wrote tags: ${r.filePath}`);
      } else {
        debug.warn("auto-tag", `Failed to write tags to: ${r.filePath} — ${r.error}`);
      }
    }

    debug.info("auto-tag", `Applied tags: ${written}/${audioFiles.length} files (${errors} errors)`);
    debug.endTimer("apply-candidate-tags", "auto-tag", `Tag application for ${albumPath}`);
    return written;
  }

  /**
   * Fetch lyrics for a track from the API, or return null if unavailable.
   */
  private async fetchTrackLyrics(
    taskId: string,
    trackName: string | null | undefined,
    artistName: string | null | undefined,
    album: string | null | undefined,
  ): Promise<string | null> {
    if (!trackName || !artistName) return null;
    const client = new LyricsClient({ baseUrl: this.config.lyricsApiUrl });
    const downloaded = await client.fetchLyrics(
      trackName,
      artistName,
      album ?? undefined,
      undefined,
    );
    if (downloaded) {
      this.emitTask(taskId, "source", `Downloaded lyrics for "${trackName}"`, {
        source: "lyrics-download",
        track: trackName,
      });
    }
    return downloaded;
  }

  /**
   * Perform direct provider ID lookups when file tags contain
   * MusicBrainz or Discogs IDs. Uses dedicated endpoints before
   * falling back to name-based search.
   */
  private async performDirectIdLookups(
    request: ReturnType<typeof makeLookupRequest>,
  ): Promise<AlbumCandidate[]> {
    const results: AlbumCandidate[] = [];
    const albumHint = request.albumHint ?? "";

    // 1. MusicBrainz album ID → direct release fetch
    if (request.musicbrainzAlbumId) {
      debug.info("auto-tag", `Direct MB lookup: albumId=${request.musicbrainzAlbumId}`);
      try {
        const mb = new MusicBrainzClient();
        const candidate = await mb.lookupReleaseById(request.musicbrainzAlbumId);
        if (candidate) {
          debug.info("auto-tag", `Direct MB lookup: found album="${candidate.album}"`);
          results.push(candidate);
        } else {
          debug.warn("auto-tag", `Direct MB lookup: release not found for albumId=${request.musicbrainzAlbumId}`);
        }
      } catch (err) {
        debug.warn("auto-tag", `Direct MB lookup failed (non-fatal)`, err);
      }
    }

    // 2. MusicBrainz artist ID only (no album ID) → browse artist releases
    if (!request.musicbrainzAlbumId && request.musicbrainzArtistId) {
      debug.info("auto-tag", `Direct MB artist lookup: artistId=${request.musicbrainzArtistId}`);
      try {
        const mb = new MusicBrainzClient();
        // Use the existing searchAlbum but this goes through /release?query=artistid:...
        // For now, fall back to search by name since MusicBrainz search
        // with artist ID requires browsing all releases which MB rate-limits heavily.
        debug.debug("auto-tag", "MB artist-only ID — falling back to name search");
      } catch (err) {
        debug.warn("auto-tag", `Direct MB artist lookup failed (non-fatal)`, err);
      }
    }

    // 3. Discogs release ID → direct release fetch
    if (request.discogsReleaseId) {
      debug.info("auto-tag", `Direct Discogs lookup: releaseId=${request.discogsReleaseId}`);
      try {
        const discogs = new DiscogsClient({
          token: this.config.discogsToken,
        });
        const candidate = await discogs.lookupReleaseById(request.discogsReleaseId);
        if (candidate) {
          debug.info("auto-tag", `Direct Discogs lookup: found album="${candidate.album}"`);
          results.push(candidate);
        } else {
          debug.warn("auto-tag", `Direct Discogs lookup: release not found for releaseId=${request.discogsReleaseId}`);
        }
      } catch (err) {
        debug.warn("auto-tag", `Direct Discogs lookup failed (non-fatal)`, err);
      }
    }

    // 4. Discogs artist ID only (no release ID) → browse artist releases
    if (!request.discogsReleaseId && request.discogsArtistId && albumHint) {
      debug.info("auto-tag", `Direct Discogs artist lookup: artistId=${request.discogsArtistId} album="${albumHint}"`);
      try {
        const discogs = new DiscogsClient({
          token: this.config.discogsToken,
        });
        const candidate = await discogs.lookupArtistReleaseByAlbum(request.discogsArtistId, albumHint);
        if (candidate) {
          debug.info("auto-tag", `Direct Discogs artist lookup: found album="${candidate.album}"`);
          results.push(candidate);
        } else {
          debug.debug("auto-tag", `Direct Discogs artist lookup: no matching release found`);
        }
      } catch (err) {
        debug.warn("auto-tag", `Direct Discogs artist lookup failed (non-fatal)`, err);
      }
    }

    return results;
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

      // Provider ID conflict detection: prefer existing (direct ID-backed) values.
      // Log a warning if a later name-search candidate has a conflicting ID.
      this.mergeProviderId(merged, candidate, "musicbrainzAlbumId");
      this.mergeProviderId(merged, candidate, "musicbrainzArtistId");
      this.mergeProviderId(merged, candidate, "discogsReleaseId");
      this.mergeProviderId(merged, candidate, "discogsArtistId");

      if (merged.tracks.length === 0 && candidate.tracks.length > 0) {
        merged.tracks = candidate.tracks;
      } else if (merged.tracks.length > 0 && candidate.tracks.length > 0) {
        this.fillTrackGaps(merged.tracks, candidate.tracks);
      }
    }

    // Keep albumArtist and albumArtists in sync — they may come from
    // different independent candidates (first-nonnull vs first-nonempty).
    if (merged.albumArtists.length > 0 && merged.albumArtist == null) {
      merged.albumArtist = merged.albumArtists[0];
    } else if (merged.albumArtists.length === 0 && merged.albumArtist != null) {
      merged.albumArtists = [merged.albumArtist];
    }

    const rest = candidates.filter((candidate) => candidate !== preferred);
    return [merged, ...rest];
  }

  /**
   * Fill safe gaps in target tracks from source tracks by position (index).
   * Both arrays are in the same local file order after protection.
   * Only fills safe nullable fields that the matcher allows.
   */
  /**
   * Merge a provider ID field from candidate into merged.
   * Prefers the existing value (direct ID-backed result) over a new one.
   * Logs a warning on conflict.
   */
  private mergeProviderId(
    merged: AlbumCandidate,
    candidate: AlbumCandidate,
    field: "musicbrainzAlbumId" | "musicbrainzArtistId" | "discogsReleaseId" | "discogsArtistId",
  ): void {
    const existing = merged[field];
    const incoming = candidate[field];

    if (!incoming) return; // nothing to merge

    if (!existing) {
      merged[field] = incoming;
      return;
    }

    // Conflict: existing (from direct lookup) vs incoming (from name search)
    if (existing !== incoming) {
      debug.warn("auto-tag", `Provider ID conflict: ${field} existing="${existing}" (direct) vs incoming="${incoming}" (name-search) — keeping existing`);
    }
  }

  private fillTrackGaps(target: TrackCandidate[], source: TrackCandidate[]): void { 
    for (let i = 0; i < Math.min(target.length, source.length); i++) {
      const targetTrack = target[i];
      const sourceTrack = source[i];
      targetTrack.musicbrainzTrackId ??= sourceTrack.musicbrainzTrackId;
      targetTrack.length ??= sourceTrack.length;
      targetTrack.genre ??= sourceTrack.genre;
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
    ].filter((name): name is string => !!name?.trim());
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

  /**
   * Fill genre via LLM if no source provided one.
   * Only called when genre is still null after Discogs and LLM tag resolution.
   * Non-fatal - if LLM fails, the candidate is used without genre.
   */
  private async fillGenreIfMissing(
    candidate: AlbumCandidate,
    request: ReturnType<typeof makeLookupRequest>,
    signal: AbortSignal,
  ): Promise<AlbumCandidate> {
    if (candidate.genre || !this.config.llmApiKey || signal.aborted) {
      return candidate;
    }

    debug.info("auto-tag", "Genre missing after all sources - filling via LLM...");
    debug.startTimer("fill-genre");

    try {
      const client = new OpenRouterClient({
        apiKey: this.config.llmApiKey,
        model: this.config.llmModel,
      });

      const messages = buildGenreFillMessages(
        candidate.artist ?? request.artistHint,
        candidate.album ?? request.albumHint,
        candidate.tracks.map((t) => t.title).filter(Boolean) as string[],
      );

      const schema = {
        type: "object",
        properties: {
          genre: { type: "string" },
          confidence: { type: "number" },
        },
        required: ["genre", "confidence"],
      };

      const response = await client.completeJson(
        messages,
        "GenreFillResponse",
        schema,
      );

      const genre = response.data.genre as string | null;
      const confidence = response.data.confidence as number | null;

      if (genre && confidence != null && confidence >= 0.6) {
        debug.info("auto-tag", `LLM genre fill: "${genre}" (confidence=${confidence})`);
        return { ...candidate, genre };
      }

      debug.debug("auto-tag", "LLM genre fill returned null or low confidence - skipping");
    } catch (err) {
      debug.warn("auto-tag", "LLM genre fill failed (non-fatal)", err);
    } finally {
      debug.endTimer("fill-genre", "auto-tag", "LLM genre fill");
    }

    return candidate;
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

  /**
   * Immediately remove a task (used for cancelled queued tasks).
   */
  private cleanupTask(taskId: string): void {
    this.tasks.delete(taskId);
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
    debug.debug("auto-tag", `Progress [${taskId}]: ${progress.progress}/${progress.total} - ${progress.message}`);
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

/**
 * Get the raw (unredacted) API config for main-process internal use.
 * Returns the actual API key - DO NOT send this to the renderer.
 */
export function getRawApiConfig(): { apiKey: string; model: string } {
  const cfg = getTaskManager().getConfig();
  return {
    apiKey: cfg.llmApiKey ?? "",
    model: cfg.llmModel ?? "",
  };
}

/** Get current config (redacted for renderer safety). */
export function getConfig(): Record<string, unknown> {
  const cfg = getTaskManager().getConfig();
  return {
    llmApiKey: cfg.llmApiKey ? "****" + cfg.llmApiKey.slice(-4) : null,
    llmModel: cfg.llmModel,
    discogsToken: cfg.discogsToken ? "****" + cfg.discogsToken.slice(-4) : null,
    remoteLookupEnabled: cfg.remoteLookupEnabled,
    discogsEnabled: cfg.discogsEnabled,
    debug: cfg.debug ?? false,
    lyricsDownloadEnabled: cfg.lyricsDownloadEnabled ?? false,
    lyricsApiUrl: cfg.lyricsApiUrl ?? null,
    googleImageApiKey: cfg.googleImageApiKey ? "****" + cfg.googleImageApiKey.slice(-4) : null,
    googleImageSearchEngineId: cfg.googleImageSearchEngineId ?? null,
    googleImageEnabled: cfg.googleImageEnabled ?? true,
    theAudioDbApiKey: cfg.theAudioDbApiKey ? "****" + cfg.theAudioDbApiKey.slice(-4) : null,
    theAudioDbEnabled: cfg.theAudioDbEnabled ?? true,
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
  lyricsDownloadEnabled: "lyrics_download_enabled",
  lyricsApiUrl: "lyrics_api_url",
  googleImageApiKey: "google_image_api_key",
  googleImageSearchEngineId: "google_image_search_engine_id",
  googleImageEnabled: "google_image_enabled",
  theAudioDbApiKey: "theaudiodb_api_key",
  theAudioDbEnabled: "theaudiodb_enabled",
};

/**
 * Save a single config value back to the YAML config file.
 * Writes a config value to ~/.auto-tagger/config.yaml.
 * Creates the file and parent directory if they don't exist.
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

  if (!configPath) {
    configPath = getConfigPaths()[0];
  }

  const parentDir = dirname(configPath);
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
  if (typeof value === "boolean") return value ? "true" : "false";
  if (value == null) return "null";
  const str = String(value);
  if (/[\s:#]/.test(str)) return `"${str.replace(/"/g, '\\"')}"`;
  return str;
}

// ── LLM track index normalization ────────────────────────────────

/**
 * Normalize LLM track indices to 0-based, positional array.
 *
 * Problem: Some LLMs return 1-based indices (index: 1, 2, 3...) instead
 * of 0-based (index: 0, 1, 2...). When the code matches by `lt.index === i`
 * and the LLM uses 1-based, track 0 gets no match (drift).
 *
 * Strategy:
 *  1. If the minimum index is 1 and count matches, subtract 1 from all.
 *  2. If indices don't align (count mismatch or gaps), sort by index
 *     and use positional order, ignoring the absolute index values.
 *  3. Return a positional array (index = position).
 */
function normalizeLlmTrackIndices(
  tracks: Array<Record<string, unknown>>,
  expectedCount: number,
): Array<Record<string, unknown>> {
  if (tracks.length === 0) return [];

  const indices = tracks
    .map((t) => Number(t.index))
    .filter((n) => !Number.isNaN(n));

  if (indices.length === 0) {
    // No usable indices - assign by position
    return tracks.slice(0, expectedCount);
  }

  const minIndex = Math.min(...indices);
  const maxIndex = Math.max(...indices);
  const uniqueIndices = new Set(indices).size;

  // Case 1: 1-based indexing (min index is 1, max matches expected count)
  if (minIndex === 1 && maxIndex === expectedCount && uniqueIndices === expectedCount) {
    // Sort by index and return positional array (subtracting 1)
    const sorted = [...tracks].sort((a, b) => Number(a.index) - Number(b.index));
    return sorted.slice(0, expectedCount);
  }

  // Case 2: 0-based indexing with contiguous indices
  if (
    minIndex === 0 &&
    uniqueIndices >= Math.min(tracks.length, expectedCount)
  ) {
    const sorted = [...tracks].sort((a, b) => Number(a.index) - Number(b.index));
    return sorted.slice(0, expectedCount);
  }

  // Case 3: non-contiguous or mismatched - use positional fallback
  debug.warn(
    "auto-tag",
    `LLM track indices don't align with input (min=${minIndex}, max=${maxIndex}, ` +
    `unique=${uniqueIndices}, expected=${expectedCount}, received=${tracks.length}) ` +
    `- using positional fallback`,
  );
  return tracks.slice(0, expectedCount);
}

// ── Track number extraction from filename ──────────────────────

/**
 * Extract track number from a filename.
 *
 * Handles common patterns:
 *   "01 - Title.ext"  → 1
 *   "01 Title.ext"    → 1
 *   "01.Title.ext"    → 1
 *   "101-Title.ext"   → 101
 *   "track 1.ext"     → null (not a numeric prefix)
 *   "- no number.ext" → null
 */
function extractTrackNumberFromFilename(filename: string): number | null {
  const base = filename.replace(/\.[^.]+$/, "");
  const match = base.match(/^(\d+)/);
  if (!match) return null;
  const num = parseInt(match[1], 10);
  if (Number.isNaN(num) || num < 1) return null;
  return num;
}

// ── Standalone lyrics download (independent of auto-tag) ───────────

/**
 * Download / fix lyrics for all tracks in an album.
 *
 * For each audio file:
 *  1. Look for a local `.lrc`/`.txt` file → detect & fix encoding → write to tag
 *  2. If no local file and download is enabled → fetch from API → fix encoding → write to tag
 *
 * Returns the number of tracks that received lyrics.
 */
export async function downloadAlbumLyrics(
  albumPath: string,
): Promise<number> {
  const config = loadConfig();

  // Collect audio files
  const audioFiles: string[] = [];
  let entries: string[];
  try {
    entries = readdirSync(albumPath).sort();
  } catch {
    return 0;
  }

  for (const name of entries) {
    const fullPath = join(albumPath, name);
    const ext = extname(name).toLowerCase();
    if (AUDIO_EXTENSIONS.has(ext) && statSync(fullPath).isFile()) {
      audioFiles.push(fullPath);
    }
  }

  if (audioFiles.length === 0) return 0;

  // Collect write jobs: read local lyrics or fetch from API
  const client = new LyricsClient({ baseUrl: config.lyricsApiUrl });
  const writeJobs: Array<{ filePath: string; fields: WriteFields }> = [];

  for (const filePath of audioFiles) {
    let lyrics = readLocalLyrics(filePath);

    if (!lyrics) {
      try {
        const meta = await readTrackMetadata(filePath);
        if (meta.title && meta.artist) {
          lyrics = await client.fetchLyrics(
            meta.title,
            meta.artist,
            meta.album ?? undefined,
            meta.duration > 0 ? Math.round(meta.duration) : undefined,
          );
        }
      } catch {
        // skip tracks that fail to read
      }
    }

    if (lyrics) {
      writeJobs.push({ filePath, fields: { lyrics } });
    }
  }

  // Submit all lyrics writes through the concurrent queue
  if (writeJobs.length === 0) return 0;
  const writeResults = await getDefaultWriteQueue().submit(writeJobs);

  return writeResults.filter((r) => r.success).length;
}
