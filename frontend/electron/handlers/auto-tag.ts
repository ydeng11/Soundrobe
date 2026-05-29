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
import { buildSelectionMessages, buildTagCorrectionMessages } from "./prompts";
import { getAllNameVariants } from "./aliases";
import { writeTags } from "./writer";
import type { WriteFields } from "./writer";
import { readLocalLyrics, LyricsClient } from "./lyrics";
import { readTrackMetadata } from "./tracks";
import { existsSync, readFileSync, writeFileSync, mkdirSync, readdirSync, statSync } from "node:fs";
import { basename, dirname, join, extname } from "node:path";

const AUDIO_EXTENSIONS = new Set([".mp3", ".flac", ".m4a", ".mp4", ".wav", ".ogg", ".opus", ".aiff"]);
import { homedir } from "node:os";
import debug from "./debug";

/**
 * Check if deterministic parsing produced ambiguous hints that warrant
 * LLM enhancement. Ported from Python lookup.py _hints_are_ambiguous.
 *
 * Returns true when the folder name has bracket annotations, Chinese dots,
 * year-prefixed album hints, or other patterns that the basic folder-name
 * parser might misinterpret.
 *
 * Known format suffixes like "[flac]", "[MP3]" are stripped before the
 * bracket check — they are not ambiguous naming conventions.
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

  const folderName = path.split("/").pop() ?? "";

  // Strip known format suffixes (e.g. "[flac]", "[FLAC]", "[MP3]") before
  // checking brackets — these are not ambiguous naming conventions.
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
    discogs_enabled: (v) => {
      if (v === "false" || v === "true") config.discogsEnabled = v === "true";
    },
    debug: (v) => {
      if (v === "false" || v === "true") config.debug = v === "true";
    },
    lyrics_download_enabled: (v) => {
      if (v === "false" || v === "true") config.lyricsDownloadEnabled = v === "true";
    },
    lyrics_api_url: (v) => { config.lyricsApiUrl = v; },
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
  if (process.env.AUTO_TAG_LYRICS_DOWNLOAD_ENABLED === "false") config.lyricsDownloadEnabled = false;
  if (process.env.AUTO_TAG_LYRICS_API_URL) config.lyricsApiUrl = process.env.AUTO_TAG_LYRICS_API_URL;

  // Sync debug logger with config
  debug.setEnabled(!!config.debug);

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

  // Get all artist name variants: learned Latin aliases first,
  // then script variants (SC/TC), then the original, then non-Latin aliases.
  let artistVariants: string[];
  try {
    artistVariants = artistText ? await getAllNameVariants(artistText) : [artistText];
  } catch {
    artistVariants = [artistText];
  }
  if (artistVariants.length === 0) artistVariants = [artistText];

  // Get all album name variants (script variants + original).
  let albumVariants: string[];
  try {
    albumVariants = albumText ? await getAllNameVariants(albumText) : [albumText];
  } catch {
    albumVariants = [albumText];
  }
  if (albumVariants.length === 0) albumVariants = [albumText];

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

      // Step 2: LLM tag resolution — uses folder name + existing file metadata
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
          debug.info("auto-tag", "Discogs disabled — skipping");
        }
        if (signal.aborted) {
          cache.close();
          return this.failCancelled(taskId);
        }

        // Step 7: Fallback candidate
        // If API lookups returned nothing and we have an LLM fallback (with genre),
        // use it instead of the bare folder fallback. Otherwise use folder fallback.
        debug.info("auto-tag", "Step 7/9: Building fallback candidate...");
        update("Building fallback...", 7);
        const hadApiCandidates = allCandidates.length > 0;
        if (hadApiCandidates) {
          // API lookups found something — use folder fallback as safety net as before
          const folderCandidate = candidateFromFolder(correctedRequest);
          allCandidates.push(folderCandidate);
        } else {
          // No API results — use LLM fallback (with genre!) if available
          const fallback = llmFallback ?? candidateFromFolder(correctedRequest);
          if (llmFallback) {
            debug.debug("auto-tag", "No API candidates — using LLM fallback (genre present)");
          }
          allCandidates.push(fallback);
        }

        // Apply verification status
        for (const c of allCandidates) {
          c.verification = "match";
        }

        debug.info("auto-tag", `Total candidates across all sources: ${allCandidates.length}`);

        // Cache the results
        cache.set(correctedRequest, allCandidates);
        debug.debug("auto-tag", "Cached results for future lookups");

        const mergedCandidates = this.mergeCandidateFields(allCandidates);
        this.emitTask(taskId, "merge", `Merged ${allCandidates.length} source candidate(s)`, {
          sourceCount: allCandidates.length,
          mergedCount: mergedCandidates.length,
        });

        // LLM selection
        update("Selecting candidate...", 8);
        const selected = await this.selectCandidate(
          correctedRequest,
          mergedCandidates,
          signal,
        );
        const candidate = selected ?? this.folderFallback(mergedCandidates);
        if (candidate) {
          debug.info("auto-tag", "Step 9/9: Applying album tags...");
          update("Applying tags...", 9);
          await this.applyCandidateTags(taskId, correctedRequest.path, candidate);
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
      debug.debug("auto-tag", "No LLM API key — skipping LLM tag resolution");
      return { correctedRequest: request, fallbackCandidate: null };
    }

    debug.info("auto-tag", `Resolving tags via LLM (model: ${this.config.llmModel ?? "default"})`);
    debug.startTimer("resolve-tags");

    try {
      const client = new OpenRouterClient({
        apiKey: this.config.llmApiKey,
        model: this.config.llmModel,
      });

      const folderName = request.path.split("/").pop() ?? "";
      const parentName =
        request.path.split("/").slice(-2, -1)[0] ?? null;

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

      // Build fallback candidate with genre (used when APIs return nothing)
      const llmTracks = Array.isArray(data.tracks) ? data.tracks as Array<Record<string, unknown>> : [];
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
        tracks: request.tracks.length > 0
          ? request.tracks.map((t, i) => {
              const llmTrack = llmTracks.find((lt) => lt.index === i);
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
      // LLM failure is non-fatal — fall back to original hints and folder fallback
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
    const albumFields: WriteFields = {};
    if (candidate.artist !== undefined) albumFields.artist = candidate.artist;
    albumFields.artists = candidate.artists.length > 0 ? candidate.artists : splitArtistNames([candidate.artist]);
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

    // Build track-level fields indexed by track number
    const trackMap = new Map<number, WriteFields>();
    for (const tc of candidate.tracks) {
      if (tc.trackNumber != null) {
        const fields: WriteFields = {};
        if (tc.title !== undefined) fields.title = tc.title;
        if (tc.artist !== undefined) fields.artist = tc.artist;
        if (tc.artists.length > 0) fields.artists = splitArtistNames(tc.artists);
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

      // 1. Read local lyrics (with encoding fix)
      let lyrics = readLocalLyrics(filePath);
      if (lyrics) {
        mergedFields.lyrics = lyrics;
        this.emitTask(taskId, "source", `Local lyrics: ${filePath}`, { source: "lyrics", path: filePath });
      }

      // 2. If no local file and download is enabled, fetch from API
      if (!lyrics && this.config.lyricsDownloadEnabled) {
        const trackName = mergedFields.title;
        const artistName = mergedFields.artist ?? albumFields.artist;
        if (trackName && artistName) {
          const client = new LyricsClient({
            baseUrl: this.config.lyricsApiUrl,
          });
          const downloaded = await client.fetchLyrics(
            trackName,
            artistName,
            mergedFields.album ?? undefined,
            undefined,
          );
          if (downloaded) {
            mergedFields.lyrics = downloaded;
            this.emitTask(taskId, "source", `Downloaded lyrics for “${trackName}”`, { source: "lyrics-download", track: trackName });
          }
        }
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
    return readLocalLyrics(filePath);
  }

  /**
   * Use LLM to select the best candidate from the results.
   * Candidate must meet minimum confidence (0.8) and match the album hint.
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
        const confidence = response.data.confidence as number | null;
        const reason = (response.data.reason as string) ?? "";
        debug.info("auto-tag", `LLM selected candidate ${idx}: confidence=${confidence}, reason="${reason}"`);

        // Confidence must meet threshold
        const MIN_CONFIDENCE = 0.8;
        if (confidence != null && confidence < MIN_CONFIDENCE) {
          debug.warn("auto-tag", `Rejecting candidate ${idx}: confidence ${confidence} < ${MIN_CONFIDENCE}`);
          return null;
        }

        // Selected candidate's album must overlap with the request's album hint
        const candidate = candidates[idx];
        if (!this.candidateMatchesAlbumHint(candidate, request.albumHint)) {
          debug.warn("auto-tag", `Rejecting candidate ${idx}: album "${candidate.album}" does not match hint "${request.albumHint}"`);
          return null;
        }

        return candidate;
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

  /**
   * Check if a candidate's album name matches the request's album hint.
   * Returns true if either string contains the other (case-insensitive),
   * or if either hint is null/empty (no constraint to enforce).
   */
  private candidateMatchesAlbumHint(
    candidate: AlbumCandidate,
    albumHint: string | null | undefined,
  ): boolean {
    if (!albumHint || !candidate.album) return true;
    const hint = albumHint.toLowerCase().trim();
    const cand = candidate.album.toLowerCase().trim();
    return cand.includes(hint) || hint.includes(cand);
  }

  /**
   * Find the folder-sourced fallback candidate, or return null.
   */
  private folderFallback(candidates: AlbumCandidate[]): AlbumCandidate | null {
    return candidates.find((c) => c.source === "folder") ?? null;
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
    lyricsDownloadEnabled: cfg.lyricsDownloadEnabled ?? false,
    lyricsApiUrl: cfg.lyricsApiUrl ?? null,
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

  let written = 0;
  // Always create client for standalone button (ignore lyricsDownloadEnabled toggle)
  const client = new LyricsClient({ baseUrl: config.lyricsApiUrl });

  for (const filePath of audioFiles) {
    // 1. Try local file first (with encoding fix)
    let lyrics = readLocalLyrics(filePath);

    // 2. If no local file, fetch from API
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

    // 3. Write lyrics to tag (only if we got something new)
    if (lyrics) {
      try {
        await writeTags(filePath, { lyrics });
        written++;
      } catch {
        // skip write failures
      }
    }
  }

  return written;
}
