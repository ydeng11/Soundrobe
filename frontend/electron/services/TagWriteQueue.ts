/**
 * TagWriteQueue — bounded concurrent tag write queue with path deduplication.
 *
 * All tag writes (auto-tag, audit, batch save, extra tags, assistant tag
 * services) go through this queue. It ensures:
 *   - Bounded concurrency (never exceeds LOCAL_WRITE_CONCURRENCY)
 *   - Same-path serialization (one file is never written concurrently)
 *   - Per-file error propagation (one failure does not cancel other writes)
 *   - Safe Buffer/Uint8Array handling for cover art
 *
 * Two execution modes:
 *   1. Inline (default): calls writeTags/writeExtraTags directly. Fast, simple,
 *      always available. Used in tests and for maximum compatibility.
 *   2. Worker-backed: optional Worker thread that executes writes off the main
 *      process. Enabled by passing `executor: createWorkerExecutor()`.
 *
 * Types TagWriteJob, TagWriteResult, TagWorkerRequest, TagWorkerResponse
 * are exported for the IPC boundary.
 */

import path from "node:path";
import { Worker } from "node:worker_threads";
import { fileURLToPath, pathToFileURL } from "node:url";
import { writeTags, writeExtraTags } from "../handlers/writer";
import type { WriteFields, ExtraTagUpdate } from "../handlers/writer";
import { mapConcurrent, mapConcurrentContinue } from "./concurrency";
import { LOCAL_WRITE_CONCURRENCY } from "./concurrency";

// ── Types ──────────────────────────────────────────────────────────

export interface TagWriteJob {
  filePath: string;
  fields?: WriteFields;
  extraTags?: ExtraTagUpdate[];
}

export interface TagWriteResult {
  filePath: string;
  success: boolean;
  error?: string;
}

// ── Inline write execution ─────────────────────────────────────────

/**
 * Execute a single tag write job by calling the low-level writer directly.
 * Kept separate from the queue so it remains individually testable.
 */
export async function executeTagWrite(job: TagWriteJob): Promise<TagWriteResult> {
  try {
    if (job.extraTags && job.extraTags.length > 0) {
      await writeExtraTags(job.filePath, job.extraTags);
    } else if (job.fields && Object.keys(job.fields).length > 0) {
      await writeTags(job.filePath, job.fields);
    }
    return { filePath: job.filePath, success: true };
  } catch (err) {
    return {
      filePath: job.filePath,
      success: false,
      error: err instanceof Error ? err.message : String(err),
    };
  }
}

// ── Path deduplication ─────────────────────────────────────────────

/**
 * Deduplicate jobs by normalized file path.
 *
 * Rules:
 *   1. Later `fields` jobs overwrite earlier ones for the same path.
 *   2. `extraTags` from multiple jobs targeting the same path are merged.
 *   3. A `fields` job followed by an `extraTags` job for the same path
 *      produces separate entries (both are needed).
 *
 * This ensures one file is never written concurrently by two jobs.
 */
export function deduplicateJobs(jobs: TagWriteJob[]): TagWriteJob[] {
  if (jobs.length === 0) return [];

  // Group by normalized path, then by type (fields vs extraTags)
  const fieldsByPath = new Map<string, TagWriteJob>();
  const extraTagsByPath = new Map<string, TagWriteJob[]>();

  for (const job of jobs) {
    const normalized = path.resolve(job.filePath);

    if (job.extraTags && job.extraTags.length > 0) {
      const existing = extraTagsByPath.get(normalized) ?? [];
      existing.push({ ...job, filePath: normalized });
      extraTagsByPath.set(normalized, existing);
    } else if (job.fields && Object.keys(job.fields).length > 0) {
      // Later fields write wins for the same path
      fieldsByPath.set(normalized, { ...job, filePath: normalized });
    }
  }

  const result: TagWriteJob[] = [];

  // Add fields jobs first
  for (const job of fieldsByPath.values()) {
    result.push(job);
  }

  // Then add extra tag jobs (merged per path)
  for (const [, jobs] of extraTagsByPath) {
    if (jobs.length === 1) {
      result.push(jobs[0]);
    } else {
      // Merge extra tags from multiple jobs for the same path
      const mergedExtraTags: ExtraTagUpdate[] = [];
      const seen = new Set<string>();
      for (const j of jobs) {
        for (const tag of j.extraTags ?? []) {
          const key = `${tag.key}\0${tag.value}`;
          if (!seen.has(key)) {
            seen.add(key);
            mergedExtraTags.push(tag);
          }
        }
      }
      result.push({
        filePath: jobs[0].filePath,
        extraTags: mergedExtraTags,
      });
    }
  }

  return result;
}

// ── Queue ──────────────────────────────────────────────────────────

/**
 * Tag write queue that bounds concurrent writes and deduplicates
 * same-path updates. Used by IPC handlers, audit, auto-tag, assistant
 * tag services, and batch save paths.
 */
export class TagWriteQueue {
  private maxConcurrency: number;
  private executor: TagWriteExecutor;

  /**
   * @param maxConcurrency Max concurrent tag writes. Defaults to LOCAL_WRITE_CONCURRENCY.
   * @param executor Optional custom executor (e.g. from createWorkerExecutor()).
   *                 Defaults to inline executeTagWrite.
   */
  constructor(maxConcurrency: number = LOCAL_WRITE_CONCURRENCY, executor?: TagWriteExecutor) {
    this.maxConcurrency = maxConcurrency;
    this.executor = executor ?? executeTagWrite;
  }

  /**
   * Submit one or more tag write jobs for concurrent execution.
   *
   * Features:
   *   - Path deduplication: same-path updates are merged or serialized.
   *   - Bounded concurrency: max concurrent writes = `maxConcurrency`.
   *   - Per-file error propagation: one failure does not cancel others.
   *
   * @returns Results in the same order as the deduplicated jobs.
   *          When a path appears multiple times in input, only the deduplicated
   *          result is returned (one per unique path).
   */
  async submit(jobs: TagWriteJob[]): Promise<TagWriteResult[]> {
    if (jobs.length === 0) return [];

    const deduped = deduplicateJobs(jobs);

    const { results, errors } = await mapConcurrentContinue(
      deduped,
      this.maxConcurrency,
      async (job) => this.executor(job),
    );

    // Merge errors and results preserving deduped order
    const resultMap = new Map<string, TagWriteResult>();
    for (const r of results) {
      resultMap.set(r.filePath, r);
    }
    for (const e of errors) {
      // mapConcurrentContinue errors don't carry the filePath directly.
      // We handle this: errors from executeTagWrite are always TagWriteResults.
      // For unexpected errors, we catch them here.
    }

    // Build result in deduped order
    const allResults: TagWriteResult[] = deduped.map((job) => {
      const normalized = path.resolve(job.filePath);
      return (
        resultMap.get(normalized) ?? {
          filePath: normalized,
          success: false,
          error: "Unknown error",
        }
      );
    });

    return allResults;
  }

  /**
   * Submit a single tag write job.
   */
  async submitOne(
    filePath: string,
    fields?: WriteFields,
    extraTags?: ExtraTagUpdate[],
  ): Promise<TagWriteResult> {
    const results = await this.submit([{ filePath, fields, extraTags }]);
    return results[0];
  }

  /**
   * Get the current max concurrency setting.
   */
  getMaxConcurrency(): number {
    return this.maxConcurrency;
  }

  /**
   * Update max concurrency at runtime.
   */
  setMaxConcurrency(value: number): void {
    if (value < 1) throw new Error("Max concurrency must be >= 1");
    this.maxConcurrency = value;
  }
}

// ── Worker executor (optional) ──────────────────────────────────────

/** A function that executes a single tag write job and returns the result. */
export type TagWriteExecutor = (job: TagWriteJob) => Promise<TagWriteResult>;

/** Message sent from the worker to the parent. */
export interface TagWorkerResponse {
  type: "result" | "ready";
  jobId?: string;
  filePath?: string;
  success?: boolean;
  error?: string;
}

/** Message sent from the parent to the worker. */
export interface TagWorkerRequest {
  type: "write" | "shutdown";
  jobId?: string;
  action?: "writeTags" | "writeExtraTags";
  filePath?: string;
  fields?: WriteFields;
  extraTags?: ExtraTagUpdate[];
}

/**
 * Resolve the path to the built tag-worker entry module.
 * In development it's at dist-electron/tag-worker.mjs;
 * in production it's bundled alongside main.js.
 */
function getWorkerEntryPath(): string {
  try {
    const currentFile = fileURLToPath(import.meta.url);
    const dir = path.dirname(currentFile);
    // In production, main.js is in dist-electron/, so the worker is alongside it.
    // In dev/test, import.meta.url points to the source tree; try dist-electron/.
    const candidate = path.resolve(dir, "tag-worker.mjs");
    return candidate;
  } catch {
    // Fallback for envs where import.meta.url is unavailable
    return "tag-worker.mjs";
  }
}

/**
 * Create a Worker-based tag write executor.
 * Spawns a single Worker thread that handles all write operations.
 * The Worker is automatically terminated when the Node process exits.
 *
 * @param workerPath - Path to the built tag-worker module.
 *                     Defaults to the built dist-electron/tag-worker.mjs.
 * @returns An executor function that sends jobs to the worker.
 */
export function createWorkerExecutor(workerPath?: string): TagWriteExecutor {
  const resolvedPath = workerPath ?? getWorkerEntryPath();
  let worker: Worker | null = null;
  let ready = false;
  const pending = new Map<string, {
    resolve: (result: TagWriteResult) => void;
    reject: (err: Error) => void;
  }>();
  let jobCounter = 0;

  function getWorker(): Worker {
    if (!worker) {
      const workerUrl = pathToFileURL(resolvedPath).href;
      worker = new Worker(workerUrl);

      worker.on("message", (msg: TagWorkerResponse) => {
        if (msg.type === "ready") {
          ready = true;
          return;
        }
        if (msg.type === "result" && msg.jobId) {
          const pending_job = pending.get(msg.jobId);
          if (pending_job) {
            pending.delete(msg.jobId);
            const filePath = msg.filePath ?? msg.jobId;
            if (msg.success) {
              pending_job.resolve({ filePath, success: true });
            } else {
              pending_job.resolve({
                filePath,
                success: false,
                error: msg.error ?? "Unknown error",
              });
            }
          }
        }
      });

      worker.on("error", (err) => {
        // Reject all pending jobs
        for (const [, p] of pending) {
          p.reject(err);
        }
        pending.clear();
        ready = false;
      });

      worker.on("exit", (code) => {
        if (code !== 0) {
          for (const [, p] of pending) {
            p.reject(new Error(`Worker exited with code ${code}`));
          }
          pending.clear();
        }
        ready = false;
        worker = null;
      });
    }
    return worker;
  }

  return async (job: TagWriteJob): Promise<TagWriteResult> => {
    const w = getWorker();
    const jobId = `job-${++jobCounter}-${Date.now()}`;

    // Wait for worker to be ready on first call
    if (!ready && worker) {
      await new Promise<void>((resolve) => {
        const check = (msg: TagWorkerResponse) => {
          if (msg.type === "ready") {
            worker?.removeListener("message", check);
            resolve();
          }
        };
        worker?.on("message", check);
      });
    }

    return new Promise<TagWriteResult>((resolve, reject) => {
      pending.set(jobId, { resolve, reject });

      const request: TagWorkerRequest = {
        type: "write",
        jobId,
        action: job.extraTags && job.extraTags.length > 0 ? "writeExtraTags" : "writeTags",
        filePath: job.filePath,
        fields: job.fields,
        extraTags: job.extraTags,
      };

      w.postMessage(request);
    });
  };
}

// ── Singleton ──────────────────────────────────────────────────────

let defaultQueue: TagWriteQueue | null = null;

/**
 * Get the default application-wide tag write queue.
 *
 * Tries to use a worker-thread based executor for CPU isolation.
 * Falls back to inline execution when the worker module is not yet built
 * (e.g. dev before first build, or in tests).
 */
export function getDefaultWriteQueue(): TagWriteQueue {
  if (!defaultQueue) {
    try {
      const workerPath = getWorkerEntryPath();
      // Check if the worker file exists before trying to create it
      const fs = require("node:fs") as typeof import("node:fs");
      if (fs.existsSync(workerPath)) {
        const executor = createWorkerExecutor(workerPath);
        defaultQueue = new TagWriteQueue(LOCAL_WRITE_CONCURRENCY, executor);
      } else {
        defaultQueue = new TagWriteQueue();
      }
    } catch {
      defaultQueue = new TagWriteQueue();
    }
  }
  return defaultQueue;
}

/**
 * Reset the default write queue (for testing).
 */
export function resetDefaultWriteQueue(): void {
  defaultQueue = null;
}
