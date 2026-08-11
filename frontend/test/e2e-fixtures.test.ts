// @vitest-environment node
import fs from "node:fs";
import { describe, expect, it, vi } from "vitest";
import { cleanupE2eWorkspace } from "../e2e-tauri/fixtures";

describe("E2E workspace cleanup", () => {
  it("retries transient Windows file locks", () => {
    const rmSync = vi.spyOn(fs, "rmSync").mockImplementation(() => undefined);

    cleanupE2eWorkspace("C:\\temp\\soundrobe-tauri-e2e");

    expect(rmSync).toHaveBeenCalledWith("C:\\temp\\soundrobe-tauri-e2e", {
      recursive: true,
      force: true,
      maxRetries: 10,
      retryDelay: 250,
    });
    rmSync.mockRestore();
  });
});
