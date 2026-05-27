/**
 * Auto-tag orchestrator — implements the full lookup chain.
 *
 * Lookup chain:
 *   Parse hints → LLM enhancement → Cache → Dataset → MusicBrainz
 *   → Discogs → Folder fallback → Cache write → LLM selection
 */

import { type AlbumCandidate, makeLookupRequest } from "./candidates";
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
import { join, extname } from "node:path";
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
          if (key === "llm_api_key") config.llmApiKey = value;
          else if (key === "llm_model") config.llmModel = value;
          else if (key === "discogs_token") config.discogsToken = value;
          else if (key === "dataset_path") config.datasetPath = value;
          else if (
            (key === "remote_lookup_enabled" || key === "remote_lookup") &&
            (value === "false" || value === "true")
          )
            config.remoteLookupEnabled = value === "true";
          else if (key === "discogs_enabled" && (value === "false" || value === "true"))
            config.discogsEnabled = value === "true";
          else if (key === "debug" && (value === "false" || value === "true"))
            config.debug = value === "true";
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
        total: 8, // 8 steps: 7 lookup chain + 1 tag application
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
      debug.info("auto-tag", "Step 1/8: Parsing folder hints...");
      update("Parsing folder hints...", 1);
      if (signal.aborted) return this.failCancelled(taskId);
      const request = await parseAlbumWithTags(albumPath);
      debug.info("auto-tag", `Parsed hints: artist="${request.artistHint}" album="${request.albumHint}" year="${request.yearHint}"`);

      // Step 2: LLM hint enhancement (if ambiguous)
      debug.info("auto-tag", "Step 2/8: Checking folder name...");
      update("Checking folder name...", 2);
      if (signal.aborted) return this.failCancelled(taskId);
      const enhancedRequest = await this.enhanceHints(request, signal);
      const hintsChanged = request.artistHint !== enhancedRequest.artistHint || request.albumHint !== enhancedRequest.albumHint;
      if (hintsChanged) {
        debug.info("auto-tag", `LLM enhanced: artist="${enhancedRequest.artistHint}" album="${enhancedRequest.albumHint}"`);
      } else {
        debug.debug("auto-tag", "Hints unchanged after LLM check");
      }

      // Step 3: Cache check
      debug.info("auto-tag", "Step 3/8: Checking cache...");
      update("Checking cache...", 3);
      if (signal.aborted) return this.failCancelled(taskId);
      const cachePath = this.config.cachePath ?? join(homedir(), ".auto-tagger", "cache.db");
      debug.debug("auto-tag", `Cache path: ${cachePath}`);
      const cache = new MatchCache(cachePath);
      try {
        const cached = cache.get(enhancedRequest);
        if (cached) {
          debug.info("auto-tag", `Cache HIT: ${cached.length} candidates (skipping network lookups)`);
          // Cache hit — skip all network lookups
          update(`Cache hit: ${cached.length} candidates`, 7);
          const selected = await this.selectCandidate(
            enhancedRequest,
            cached,
            signal,
          );
          cache.close();
          const cachePick = selected ?? (Array.isArray(cached) ? cached[0] : cached);
          if (cachePick) {
            debug.info("auto-tag", "Step 8/8: Applying album tags...");
            update("Applying tags...", 8);
            await this.applyCandidateTags(enhancedRequest.path, cachePick);
          }
          this.completeTask(taskId, cachePick ?? cached);
          return;
        }
        debug.debug("auto-tag", "Cache MISS — proceeding with lookups");

        // Step 4: Local dataset
        debug.info("auto-tag", "Step 4/8: Querying local dataset...");
        update("Querying local dataset...", 4);
        let allCandidates: AlbumCandidate[] = [];
        const dataset = new DatasetReader(this.config.datasetPath);
        if (dataset.isAvailable()) {
          debug.debug("auto-tag", `Dataset available at: ${this.config.datasetPath}`);
          const datasetCandidates = dataset.queryAlbum(
            enhancedRequest.artistHint ?? "",
            enhancedRequest.albumHint ?? "",
          );
          debug.info("auto-tag", `Dataset returned ${datasetCandidates.length} candidates`);
          allCandidates.push(...datasetCandidates);
          dataset.close();
        } else {
          debug.warn("auto-tag", "Dataset not available — skipping");
        }
        if (signal.aborted) {
          cache.close();
          return this.failCancelled(taskId);
        }

        // Step 5: MusicBrainz
        if (this.config.remoteLookupEnabled !== false) {
          debug.info("auto-tag", "Step 5/8: Searching MusicBrainz...");
          update("Searching MusicBrainz...", 5);
          try {
            const mb = new MusicBrainzClient();
            debug.debug("auto-tag", `MusicBrainz search: artist="${enhancedRequest.artistHint}" album="${enhancedRequest.albumHint}"`);
            const mbCandidates = await mb.searchAlbum(
              enhancedRequest.artistHint,
              enhancedRequest.albumHint,
            );
            debug.info("auto-tag", `MusicBrainz returned ${mbCandidates.length} candidates`);
            allCandidates.push(...mbCandidates);
          } catch (err) {
            debug.warn("auto-tag", `MusicBrainz lookup failed (non-fatal)`, err);
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
          debug.info("auto-tag", "Step 6/8: Searching Discogs...");
          update("Searching Discogs...", 6);
          try {
            const discogs = new DiscogsClient({
              token: this.config.discogsToken,
            });
            debug.debug("auto-tag", `Discogs search: artist="${enhancedRequest.artistHint}" album="${enhancedRequest.albumHint}"`);
            const discogsCandidates = await discogs.searchAlbum(
              enhancedRequest.artistHint ?? "",
              enhancedRequest.albumHint ?? "",
            );
            debug.info("auto-tag", `Discogs returned ${discogsCandidates.length} candidates`);
            allCandidates.push(...discogsCandidates);
          } catch (err) {
            debug.warn("auto-tag", `Discogs lookup failed (non-fatal)`, err);
          }
        } else {
          debug.info("auto-tag", "Discogs disabled — skipping");
        }
        if (signal.aborted) {
          cache.close();
          return this.failCancelled(taskId);
        }

        // Step 7: Folder fallback (always included as safety net)
        debug.info("auto-tag", "Step 7/8: Building fallback candidate...");
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

        // LLM selection
        const selected = await this.selectCandidate(
          enhancedRequest,
          allCandidates,
          signal,
        );
        const candidate = selected ?? allCandidates[0];
        if (candidate) {
          debug.info("auto-tag", "Step 8/8: Applying album tags...");
          update("Applying tags...", 8);
          await this.applyCandidateTags(enhancedRequest.path, candidate);
        }
        this.completeTask(taskId, candidate ?? allCandidates);
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

    const folderName =
      request.path.split("/").pop() ?? "";

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
    albumPath: string,
    candidate: AlbumCandidate,
  ): Promise<number> {
    debug.info("auto-tag", `Applying candidate tags from source="${candidate.source}" to: ${albumPath}`);
    debug.startTimer("apply-candidate-tags");

    const dir = albumPath;

    // Discover audio files in the album directory
    let audioFiles: string[] = [];
    try {
      const entries = readdirSync(dir).sort();
      for (const entry of entries) {
        if (entry.startsWith(".")) continue;
        const fullPath = join(dir, entry);
        try {
          if (statSync(fullPath).isFile()) {
            const ext = extname(entry).toLowerCase();
            if (["mp3", ".flac", ".m4a", ".mp4", ".wav", ".ogg", ".opus", ".aiff"].includes(ext)) {
              audioFiles.push(fullPath);
            }
          }
        } catch { /* skip unreadable */ }
      }
    } catch (err) {
      debug.error("auto-tag", `Cannot read album directory: ${dir}`, err);
      throw new Error(`Cannot read album directory: ${dir}`);
    }

    debug.debug("auto-tag", `Found ${audioFiles.length} audio files in ${dir}`);
    if (audioFiles.length === 0) return 0;

    // Build album-level fields applied to every file
    const albumFields: WriteFields = {};
    if (candidate.artist !== undefined) albumFields.artist = candidate.artist;
    if (candidate.album !== undefined) albumFields.album = candidate.album;
    if (candidate.albumArtist !== undefined) albumFields.artist = candidate.albumArtist;
    if (candidate.year !== undefined) albumFields.year = candidate.year;
    if (candidate.genre !== undefined) albumFields.genre = candidate.genre;

    // Build track-level fields indexed by track number
    const trackMap = new Map<number, WriteFields>();
    for (const tc of candidate.tracks) {
      if (tc.trackNumber != null) {
        const fields: WriteFields = {};
        if (tc.title !== undefined) fields.title = tc.title;
        if (tc.trackNumber != null) fields.trackNumber = tc.trackNumber;
        if (tc.trackTotal != null) fields.trackTotal = tc.trackTotal;
        if (tc.discNumber != null) fields.discNumber = tc.discNumber;
        if (tc.discTotal != null) fields.discTotal = tc.discTotal;
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

      if (Object.keys(mergedFields).length === 0) {
        debug.debug("auto-tag", `No fields to write for: ${filePath} — skipping`);
        continue;
      }

      try {
        await writeTags(filePath, mergedFields);
        written++;
        debug.debug("auto-tag", `Wrote tags: ${filePath}`);
      } catch (err) {
        errors++;
        debug.warn("auto-tag", `Failed to write tags to: ${filePath}`, err);
      }
    }

    debug.info("auto-tag", `Applied tags: ${written}/${audioFiles.length} files (${errors} errors)`);
    debug.endTimer("apply-candidate-tags", "auto-tag", `Tag application for ${dir}`);
    return written;
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
  }

  private completeTask(
    taskId: string,
    result: unknown,
  ): void {
    debug.info("auto-tag", `Task ${taskId} completed`);
    this.updateTask(taskId, {
      status: "completed",
      progress: 8,
      message: "Complete",
      result,
    });
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
