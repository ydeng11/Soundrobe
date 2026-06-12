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
 *   - worker → parent: { type: "result", jobId: string, success: boolean, error?: string }
 *   - worker → parent: { type: "ready" }
 */

import { parentPort, isMainThread } from "node:worker_threads";
import { writeTags, writeExtraTags } from "./handlers/writer";
import type { WriteFields, ExtraTagUpdate } from "./handlers/writer";

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
      if (msg.action === "writeExtraTags") {
        await writeExtraTags(msg.filePath, msg.extraTags ?? []);
      } else if (msg.action === "writeTags" && msg.fields) {
        await writeTags(msg.filePath, msg.fields);
      }
      parentPort.postMessage({
        type: "result",
        jobId: msg.jobId,
        filePath: msg.filePath,
        success: true,
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
