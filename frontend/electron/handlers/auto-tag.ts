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
import { existsSync, readFileSync, writeFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";
import { homedir } from "node:os";

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
        total: 7, // 7 steps in the lookup chain
        message: "Starting...",
        result: null,
      },
      abort,
    });

    // Start the async processing (don't await — runs in background)
    this.processAlbum(taskId, albumPath, abort.signal).catch((err) => {
      this.updateTask(taskId, {
        status: "failed",
        message: err instanceof Error ? err.message : "Unknown error",
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

    // Step 1: Parse folder hints
    update("Parsing folder hints...", 1);
    if (signal.aborted) return this.failCancelled(taskId);
    const request = await parseAlbumWithTags(albumPath);

    // Step 2: LLM hint enhancement (if ambiguous)
    update("Checking folder name...", 2);
    if (signal.aborted) return this.failCancelled(taskId);
    const enhancedRequest = await this.enhanceHints(request, signal);

    // Step 3: Cache check
    update("Checking cache...", 3);
    if (signal.aborted) return this.failCancelled(taskId);
    const cache = new MatchCache(this.config.cachePath ?? join(homedir(), ".auto-tagger", "cache.db"));
    try {
      const cached = cache.get(enhancedRequest);
      if (cached) {
        // Cache hit — skip all network lookups
        update(`Cache hit: ${cached.length} candidates`, 7);
        const selected = await this.selectCandidate(
          enhancedRequest,
          cached,
          signal,
        );
        cache.close();
        this.completeTask(taskId, selected ?? cached);
        return;
      }

      // Step 4: Local dataset
      update("Querying local dataset...", 4);
      let allCandidates: AlbumCandidate[] = [];
      const dataset = new DatasetReader(this.config.datasetPath);
      if (dataset.isAvailable()) {
        const datasetCandidates = dataset.queryAlbum(
          enhancedRequest.artistHint ?? "",
          enhancedRequest.albumHint ?? "",
        );
        allCandidates.push(...datasetCandidates);
        dataset.close();
      }
      if (signal.aborted) {
        cache.close();
        return this.failCancelled(taskId);
      }

      // Step 5: MusicBrainz
      if (this.config.remoteLookupEnabled !== false) {
        update("Searching MusicBrainz...", 5);
        try {
          const mb = new MusicBrainzClient();
          const mbCandidates = await mb.searchAlbum(
            enhancedRequest.artistHint,
            enhancedRequest.albumHint,
          );
          allCandidates.push(...mbCandidates);
        } catch {
          // MusicBrainz failure is non-fatal
        }
      }
      if (signal.aborted) {
        cache.close();
        return this.failCancelled(taskId);
      }

      // Step 6: Discogs
      if (this.config.discogsEnabled !== false) {
        update("Searching Discogs...", 6);
        try {
          const discogs = new DiscogsClient({
            token: this.config.discogsToken,
          });
          const discogsCandidates = await discogs.searchAlbum(
            enhancedRequest.artistHint ?? "",
            enhancedRequest.albumHint ?? "",
          );
          allCandidates.push(...discogsCandidates);
        } catch {
          // Discogs failure is non-fatal
        }
      }
      if (signal.aborted) {
        cache.close();
        return this.failCancelled(taskId);
      }

      // Step 7: Folder fallback (always included as safety net)
      update("Building fallback...", 7);
      const folderCandidate = candidateFromFolder(enhancedRequest);
      allCandidates.push(folderCandidate);

      // Apply verification status
      for (const c of allCandidates) {
        c.verification = "match";
      }

      // Cache the results
      cache.set(enhancedRequest, allCandidates);

      // LLM selection
      const selected = await this.selectCandidate(
        enhancedRequest,
        allCandidates,
        signal,
      );
      this.completeTask(taskId, selected ?? allCandidates);
    } finally {
      cache.close();
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
    if (!this.hintsAreAmbiguous(request)) return request;

    if (!this.config.llmApiKey) return request;

    try {
      const client = new OpenRouterClient({
        apiKey: this.config.llmApiKey,
        model: this.config.llmModel,
      });

      const folderName = request.path.split("/").pop() ?? "";
      const parentName =
        request.path.split("/").slice(-2, -1)[0] ?? null;

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
      return {
        ...request,
        artistHint: (extraction.artist as string) || request.artistHint,
        albumHint: (extraction.album as string) || request.albumHint,
        yearHint: (extraction.year as string) || request.yearHint,
      };
    } catch {
      // LLM enhancement failure is non-fatal
      return request;
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
   * Use LLM to select the best candidate from the results.
   */
  private async selectCandidate(
    request: ReturnType<typeof makeLookupRequest>,
    candidates: AlbumCandidate[],
    signal: AbortSignal,
  ): Promise<AlbumCandidate | null> {
    if (!this.config.llmApiKey || candidates.length <= 1) return null;
    if (signal.aborted) return null;

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
        return candidates[idx];
      }
    } catch {
      // Selection failure is non-fatal — return all candidates
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
    this.updateTask(taskId, {
      status: "completed",
      progress: 7,
      message: "Complete",
      result,
    });
    // Clean up after 5 minutes
    setTimeout(() => this.tasks.delete(taskId), 5 * 60 * 1000);
  }

  private failCancelled(taskId: string): void {
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

/** Start auto-tagging an album. Returns a taskId string. */
export function startAutoTag(albumPath: string): string {
  return getTaskManager().startAutoTag(albumPath);
}

/** Get progress for a task. */
export function getProgress(
  taskId: string,
): TaskProgress | null {
  return getTaskManager().getTaskProgress(taskId);
}

/** Cancel a running task. */
export function cancelTask(taskId: string): void {
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
};

const BOOLEAN_KEYS = new Set(["remote_lookup_enabled", "discogs_enabled"]);

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
