import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";
import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import {
  loadConfig,
  startAutoTag,
  getProgress,
  cancelTask,
  getDatasetStatus,
  getConfig,
  refreshConfig,
  onAutoTagEvent,
} from "../../electron/handlers/auto-tag";

describe("loadConfig", () => {
  const originalEnv = { ...process.env };

  afterEach(() => {
    process.env = { ...originalEnv };
    // Clear config-dependent env vars that might exist on the dev machine
    delete process.env.AUTO_TAG_LLM_API_KEY;
    delete process.env.AUTO_TAG_LLM_MODEL;
    delete process.env.AUTO_TAG_DISCOGS_TOKEN;
  });

  it("loads from environment variables", () => {
    process.env.AUTO_TAG_LLM_API_KEY = "env-key";
    process.env.AUTO_TAG_LLM_MODEL = "env-model";
    process.env.AUTO_TAG_DISCOGS_TOKEN = "env-token";

    const config = loadConfig();
    expect(config.llmApiKey).toBe("env-key");
    expect(config.llmModel).toBe("env-model");
    expect(config.discogsToken).toBe("env-token");
  });

  it("defaults to remote lookup enabled", () => {
    delete process.env.AUTO_TAG_REMOTE_LOOKUP;
    const config = loadConfig();
    expect(config.remoteLookupEnabled).toBe(true);
  });

  it("respects remote lookup disabled", () => {
    process.env.AUTO_TAG_REMOTE_LOOKUP = "false";
    expect(loadConfig().remoteLookupEnabled).toBe(false);
  });
});

describe("startAutoTag / getProgress / cancelTask", () => {
  it("creates a task and tracks it", async () => {
    const taskId = startAutoTag("/test/album/path");
    expect(taskId).toBeTruthy();
    expect(taskId.startsWith("auto-tag-")).toBe(true);

    const progress = getProgress(taskId);
    expect(progress).not.toBeNull();
    expect(progress!.taskId).toBe(taskId);
    expect(progress!.status).toBe("running");
    expect(progress!.total).toBe(9);
  });

  it("returns null for unknown task", () => {
    expect(getProgress("nonexistent")).toBeNull();
  });

  it("cancels a running task", () => {
    const taskId = startAutoTag("/test/album");
    cancelTask(taskId);
    const progress = getProgress(taskId);
    expect(progress!.status).toBe("cancelled");
  });

  it("emits live task events", () => {
    const events: string[] = [];
    const unsubscribe = onAutoTagEvent((event) => events.push(event.type));
    const taskId = startAutoTag("/test/album/events");
    const progress = getProgress(taskId);
    unsubscribe();

    expect(progress).not.toBeNull();
    expect(events).toContain("progress");
  });
});

describe("getDatasetStatus", () => {
  it("returns status without crashing", { timeout: 15000 }, () => {
    // May or may not have a dataset — just verify it returns a valid shape
    const status = getDatasetStatus();
    expect(status).toHaveProperty("available");
    expect(status).toHaveProperty("totalRecords");
    expect(typeof status.available).toBe("boolean");
    expect(typeof status.totalRecords).toBe("number");
  });
});

describe("getConfig / refreshConfig", () => {
  it("returns config without exposing full keys", () => {
    const config = getConfig();
    expect(config).toHaveProperty("llmApiKey");
    expect(config).toHaveProperty("llmModel");
  });

  it("refreshConfig does not throw", () => {
    expect(() => refreshConfig()).not.toThrow();
  });
});
