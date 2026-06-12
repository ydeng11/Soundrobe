/**
 * TagWriteQueue — bounded concurrent tag write queue with path deduplication.
 *
 * All tag writes (auto-tag, audit, batch save, extra tags, assistant tag
 * services) go through this queue. It ensures:
 *   - Bounded concurrency globally across ALL submit() calls (not just within one).
 *   - Same-path serialization (one file is never written concurrently).
 *   - Per-file error propagation (one failure does not cancel other writes).
 *   - Safe Buffer/Uint8Array handling for cover art.
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
import { stat } from "node:fs/promises";
import { Worker } from "node:worker_threads";
import { fileURLToPath, pathToFileURL } from "node:url";
import { writeTagsWithOutcome, writeExtraTagsWithOutcome } from "../handlers/writer";
import type { WriteFields, ExtraTagUpdate, WriteOutcome } from "../handlers/writer";
import logger from "../handlers/debug";

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
  /** Internal write outcome — how the write was performed. */
  outcome?: WriteOutcome;
  /** Duration in milliseconds for the write operation. */
  durationMs?: number;
}

// ── Job classification helper ────────────────────────────────────

/**
 * Determine whether a job is an extra-tag operation.
 * Any job with `extraTags !== undefined`, including an empty array,
 * is classified as an extra-tag job. A job with only `fields` is not.
 */
export function isExtraTagJob(job: TagWriteJob): boolean {
  return job.extraTags !== undefined;
}

// ── Inline write execution ─────────────────────────────────────────

/**
 * Execute a single tag write job by calling the low-level writer directly.\n * Kept separate from the queue so it remains individually testable.
 */
export async function executeTagWrite(job: TagWriteJob): Promise<TagWriteResult> {
  const start = Date.now();
  try {
    let outcome: WriteOutcome = "full_rewrite";
    if (isExtraTagJob(job)) {
      outcome = await writeExtraTagsWithOutcome(job.filePath, job.extraTags!);
    } else if (job.fields && Object.keys(job.fields).length > 0) {
      outcome = await writeTagsWithOutcome(job.filePath, job.fields);
    }
    return { filePath: job.filePath, success: true, outcome, durationMs: Date.now() - start };
  } catch (err) {
    return {
      filePath: job.filePath,
      success: false,
      error: err instanceof Error ? err.message : String(err),
      durationMs: Date.now() - start,
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

    if (isExtraTagJob(job)) {
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

// ── Internal queue primitives ──────────────────────────────────────

/** A job enqueued in the shared pending queue, tagged with its batch. */
interface QueuedJob {
  job: TagWriteJob;
  batchId: number;
  index: number;
}

/** Tracks one batch of jobs submitted via a single `submit()` call. */
interface BatchState {
  results: (TagWriteResult | undefined)[];
  remaining: number;
  resolve: (results: TagWriteResult[]) => void;
  reject: (err: Error) => void;
}

// ── Queue ──────────────────────────────────────────────────────────

/**
 * Tag write queue that bounds concurrent writes across ALL callers and
 * deduplicates same-path updates. Used by IPC handlers, audit, auto-tag,
 * assistant tag services, and batch save paths.
 *
 * Key difference from a per-call concurrent-map: tasks from overlapping
 * `submit()` / `submitOne()` calls share one pending queue and one
 * concurrency limit. With `maxConcurrency = 1`, undo's
 * `Promise.all(... writeTrack ...)` runs sequentially instead of starting
 * multiple SMB reads at once.
 */
export class TagWriteQueue {
  private maxConcurrency: number;
  private executor: TagWriteExecutor;
  private pending: QueuedJob[] = [];
  private batches = new Map<number, BatchState>();
  private nextBatchId = 0;
  /** Number of writes currently executing. */
  private active = 0;

  /**
   * @param maxConcurrency Max concurrent tag writes across the entire instance.
   *                        Defaults to 1 for NAS-safe serialized writes.
   * @param executor Optional custom executor (e.g. from createWorkerExecutor()).
   *                 Defaults to inline executeTagWrite.
   */
  constructor(maxConcurrency: number = 1, executor?: TagWriteExecutor) {
    this.maxConcurrency = maxConcurrency;
    this.executor = executor ?? executeTagWrite;
  }

  /**
   * Submit one or more tag write jobs.
   *
   * Features:
   *   - Path deduplication per call: same-path updates are merged or serialized.
   *   - Global bounded concurrency: max concurrent writes across ALL submit calls
   *     = `maxConcurrency`.
   *   - Per-file error propagation: one failure does not cancel others.
   *
   * @returns Results in the same order as the deduplicated jobs.
   *          When a path appears multiple times in input, only the deduplicated
   *          result is returned (one per unique path).
   */
  async submit(jobs: TagWriteJob[]): Promise<TagWriteResult[]> {
    if (jobs.length === 0) return [];

    const deduped = deduplicateJobs(jobs);
    if (deduped.length === 0) return [];

    return new Promise<TagWriteResult[]>((resolve, reject) => {
      const batchId = this.nextBatchId++;
      const batch: BatchState = {
        results: new Array(deduped.length),
        remaining: deduped.length,
        resolve,
        reject,
      };
      this.batches.set(batchId, batch);

      for (let i = 0; i < deduped.length; i++) {
        this.pending.push({ job: deduped[i], batchId, index: i });
      }

      this.drain();
    });
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

  // ── Drain loop ──────────────────────────────────────────────────

  /**
   * Start as many pending jobs as the concurrency limit allows.
   *
   * Safe to call multiple times — it is idempotent. Always called after
   * enqueueing new jobs and after a job completes.
   */
  private drain(): void {
    while (this.active < this.maxConcurrency && this.pending.length > 0) {
      const entry = this.pending.shift()!;
      this.active++;
      // Fire-and-forget the async execution; it will call drain() on completion.
      this.executeJob(entry);
    }
  }

  /**
   * Execute one job with timing, logging, and batch-result routing.
   * Calls drain() on completion to start the next pending job.
   */
  private async executeJob(entry: QueuedJob): Promise<void> {
    const start = Date.now();
    let fileSize = 0;
    try {
      fileSize = (await stat(entry.job.filePath)).size;
    } catch {
      // File may not exist yet; size stays 0
    }

    try {
      const result = await this.executor(entry.job);
      if (result.durationMs === undefined) {
        result.durationMs = Date.now() - start;
      }

      logger.info("write", `${path.basename(entry.job.filePath)} — ${result.outcome ?? "full_rewrite"} — ${result.durationMs}ms`, {
        path: entry.job.filePath,
        extension: path.extname(entry.job.filePath),
        size: fileSize,
        durationMs: result.durationMs,
        outcome: result.outcome,
        error: result.error ?? null,
      });

      this.resolveBatchJob(entry.batchId, entry.index, result);
    } catch (err) {
      const durationMs = Date.now() - start;
      logger.info("write", `${path.basename(entry.job.filePath)} — error — ${durationMs}ms`, {
        path: entry.job.filePath,
        extension: path.extname(entry.job.filePath),
        size: fileSize,
        durationMs,
        error: err instanceof Error ? err.message : String(err),
      });

      this.resolveBatchJob(entry.batchId, entry.index, {
        filePath: entry.job.filePath,
        success: false,
        error: err instanceof Error ? err.message : String(err),
        durationMs,
      });
    } finally {
      this.active--;
      this.drain();
    }
  }

  /**
   * Route a completed job's result back to its batch and resolve the
   * batch promise when all jobs in the batch are done.
   */
  private resolveBatchJob(batchId: number, index: number, result: TagWriteResult): void {
    const batch = this.batches.get(batchId);
    if (!batch) return;

    batch.results[index] = result;
    batch.remaining--;

    if (batch.remaining === 0) {
      this.batches.delete(batchId);
      batch.resolve(batch.results as TagWriteResult[]);
    }
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
        action: isExtraTagJob(job) ? "writeExtraTags" : "writeTags",
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
        defaultQueue = new TagWriteQueue(1, executor);
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
