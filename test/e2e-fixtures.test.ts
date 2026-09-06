// @vitest-environment node
import fs from "node:fs";
import { describe, expect, it, vi } from "vitest";
import { cleanupE2eWorkspace, prepareE2eWorkspace } from "../e2e-tauri/fixtures";

describe("E2E workspace cleanup", () => {
  it("forces offline provider settings into the native E2E process", () => {
    const originalManifest = process.env.SOUNDROBE_E2E_MANIFEST;
    const originalRemoteLookup = process.env.AUTO_TAG_REMOTE_LOOKUP;
    const originalDiscogsEnabled = process.env.AUTO_TAG_DISCOGS_ENABLED;
    delete process.env.SOUNDROBE_E2E_MANIFEST;

    const workspace = prepareE2eWorkspace();
    try {
      expect(process.env.AUTO_TAG_REMOTE_LOOKUP).toBe("false");
      expect(process.env.AUTO_TAG_DISCOGS_ENABLED).toBe("false");
    } finally {
      cleanupE2eWorkspace(workspace.root);
      if (originalManifest === undefined) delete process.env.SOUNDROBE_E2E_MANIFEST;
      else process.env.SOUNDROBE_E2E_MANIFEST = originalManifest;
      if (originalRemoteLookup === undefined) delete process.env.AUTO_TAG_REMOTE_LOOKUP;
      else process.env.AUTO_TAG_REMOTE_LOOKUP = originalRemoteLookup;
      if (originalDiscogsEnabled === undefined) delete process.env.AUTO_TAG_DISCOGS_ENABLED;
      else process.env.AUTO_TAG_DISCOGS_ENABLED = originalDiscogsEnabled;
    }
  });

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
