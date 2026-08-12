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

  it("does not fail the run when WebView keeps a Windows profile file busy", () => {
    const originalPlatform = process.platform;
    Object.defineProperty(process, "platform", { value: "win32" });
    const warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);
    const rmSync = vi.spyOn(fs, "rmSync").mockImplementation(() => {
      const error = new Error("profile file is still in use") as NodeJS.ErrnoException;
      error.code = "EBUSY";
      throw error;
    });

    try {
      expect(() => cleanupE2eWorkspace("C:\\temp\\soundrobe-tauri-e2e")).not.toThrow();
      expect(warn).toHaveBeenCalledWith(
        expect.stringContaining("leaving it for the runner to reclaim"),
      );
    } finally {
      rmSync.mockRestore();
      warn.mockRestore();
      Object.defineProperty(process, "platform", { value: originalPlatform });
    }
  });

  it("does not fail the run when Mesa adds a Linux shader cache entry during cleanup", () => {
    const originalPlatform = process.platform;
    Object.defineProperty(process, "platform", { value: "linux" });
    const warn = vi.spyOn(console, "warn").mockImplementation(() => undefined);
    const rmSync = vi.spyOn(fs, "rmSync").mockImplementation(() => {
      const error = new Error(
        "directory not empty, rmdir 'home/.cache/mesa_shader_cache'",
      ) as NodeJS.ErrnoException;
      error.code = "ENOTEMPTY";
      throw error;
    });

    try {
      expect(() => cleanupE2eWorkspace("/tmp/soundrobe-tauri-e2e")).not.toThrow();
      expect(warn).toHaveBeenCalledWith(
        expect.stringContaining("leaving it for the runner to reclaim"),
      );
    } finally {
      rmSync.mockRestore();
      warn.mockRestore();
      Object.defineProperty(process, "platform", { value: originalPlatform });
    }
  });

  it("still reports unexpected cleanup failures", () => {
    const originalPlatform = process.platform;
    Object.defineProperty(process, "platform", { value: "linux" });
    const rmSync = vi.spyOn(fs, "rmSync").mockImplementation(() => {
      throw new Error("filesystem failure");
    });

    try {
      expect(() => cleanupE2eWorkspace("/tmp/soundrobe-tauri-e2e")).toThrow(
        "filesystem failure",
      );
    } finally {
      rmSync.mockRestore();
      Object.defineProperty(process, "platform", { value: originalPlatform });
    }
  });
});
