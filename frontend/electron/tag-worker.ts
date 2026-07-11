/**
 * Tag worker entry point for Electron's `worker_threads`.
 *
 * This module runs inside a Node.js Worker, not in the Electron renderer or
 * main process. It receives TagWriteRequest messages from the parent process
 * and executes tag writes using the low-level writer functions.
 *
 * The worker processes each job independently and sends back a
 * TagWorkerResponse for each completed or failed write.
 *
 * Message protocol:
 *   - parent → worker: { type: "write", jobId: string, action: "writeTags" | "writeExtraTags", filePath: string, fields?: WriteFields, extraTags?: ExtraTagUpdate[] }
 *   - parent → worker: { type: "shutdown" }
 *   - worker → parent: { type: "result", jobId: string, success: boolean, outcome?: WriteOutcome, reason?: WriteReason, error?: string }
 *   - worker → parent: { type: "ready" }
 */

import { parentPort, isMainThread } from "node:worker_threads";
import { writeTagsWithResult, writeExtraTagsWithResult } from "./handlers/writer";
import type {
  WriteFields,
  ExtraTagUpdate,
  WriteOutcome,
  WriteReason,
  WriteResult,
} from "./handlers/writer";

if (isMainThread) {
  throw new Error("tag-worker should only run as a worker thread");
}

// ── Types ──────────────────────────────────────────────────────────

interface TagWriteMessage {
  type: "write";
  jobId: string;
  action: "writeTags" | "writeExtraTags";
  filePath: string;
  fields?: WriteFields;
  extraTags?: ExtraTagUpdate[];
}

interface ShutdownMessage {
  type: "shutdown";
}

type WorkerMessage = TagWriteMessage | ShutdownMessage;

interface TagResultMessage {
  type: "result";
  jobId: string;
  filePath: string;
  success: boolean;
  error?: string;
  outcome?: WriteOutcome;
  reason?: WriteReason;
}

// ── Message handler ────────────────────────────────────────────────

if (!parentPort) {
  throw new Error("tag-worker requires a parentPort");
}

// Signal readiness
parentPort.postMessage({ type: "ready" });

parentPort.on("message", async (msg: WorkerMessage) => {
  if (!parentPort) return;

  if (msg.type === "shutdown") {
    process.exit(0);
  }

  if (msg.type === "write") {
    try {
      let result: WriteResult = {
        outcome: "skipped",
        reason: "unchanged",
      };
      if (msg.action === "writeExtraTags") {
        result = await writeExtraTagsWithResult(msg.filePath, msg.extraTags ?? []);
      } else if (msg.action === "writeTags" && msg.fields) {
        result = await writeTagsWithResult(msg.filePath, msg.fields);
      }
      parentPort.postMessage({
        type: "result",
        jobId: msg.jobId,
        filePath: msg.filePath,
        success: true,
        outcome: result.outcome,
        reason: result.reason,
      } satisfies TagResultMessage);
    } catch (err) {
      parentPort.postMessage({
        type: "result",
        jobId: msg.jobId,
        filePath: msg.filePath,
        success: false,
        error: err instanceof Error ? err.message : String(err),
      } satisfies TagResultMessage);
    }
  }
});
