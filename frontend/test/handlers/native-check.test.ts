// @vitest-environment node
import { describe, it, expect, vi, beforeEach } from "vitest";
import { existsSync } from "node:fs";
import { execSync } from "node:child_process";

const nativeRequireMock = vi.hoisted(() => {
  const requireFn = vi.fn() as unknown as ReturnType<typeof vi.fn> & {
    resolve: ReturnType<typeof vi.fn>;
    cache: Record<string, unknown>;
  };
  requireFn.resolve = vi.fn((path: string) => path);
  requireFn.cache = {};
  return requireFn;
});

// Mock "electron" — these are always needed
vi.mock("electron", () => ({
  app: { isReady: vi.fn(() => true) },
  dialog: { showMessageBox: vi.fn() },
}));

// Mock fs — always needed for existsSync check
vi.mock("node:fs", () => ({ existsSync: vi.fn() }));

// Mock child_process — needed for rebuild path
vi.mock("node:child_process", () => ({ execSync: vi.fn() }));

// Mock createRequire so native binding loads are deterministic in tests
vi.mock("node:module", () => ({ createRequire: () => nativeRequireMock }));

// Mock URL resolution
vi.mock("node:url", () => ({
  fileURLToPath: () => "/Users/test/auto_tagger/frontend/electron/handlers",
}));

beforeEach(() => {
  vi.clearAllMocks();
  vi.mocked(existsSync).mockReturnValue(true);
  nativeRequireMock.mockReturnValue({});
  nativeRequireMock.resolve.mockImplementation((path: string) => path);
  nativeRequireMock.cache = {};
});

describe("ensureNativeModules", () => {
  it("returns true when the .node file does not exist", async () => {
    vi.mocked(existsSync).mockReturnValue(false);
    const { ensureNativeModules } = await import(
      "../../electron/handlers/native-check"
    );
    expect(await ensureNativeModules()).toBe(true);
  });

  it("returns true when the module loads successfully", async () => {
    const { ensureNativeModules } = await import(
      "../../electron/handlers/native-check"
    );
    expect(await ensureNativeModules()).toBe(true);
    expect(nativeRequireMock).toHaveBeenCalledOnce();
  });

  it("returns true when app is not ready yet (defer to first window)", async () => {
    const { app } = await import("electron");
    vi.mocked(app.isReady).mockReturnValue(false);
    const mismatch = Object.assign(new Error("ABI mismatch"), {
      code: "ERR_DLOPEN_FAILED",
    });
    nativeRequireMock.mockImplementation(() => {
      throw mismatch;
    });

    const { ensureNativeModules } = await import(
      "../../electron/handlers/native-check"
    );
    const result = await ensureNativeModules();
    expect(result).toBe(true);
  });

  it("shows dialog and quits when ABI mismatch detected", async () => {
    vi.resetModules();
    const mismatch = Object.assign(new Error("ABI mismatch"), {
      code: "ERR_DLOPEN_FAILED",
    });
    nativeRequireMock.mockImplementation(() => {
      throw mismatch;
    });

    const { ensureNativeModules } = await import(
      "../../electron/handlers/native-check"
    );

    const { app, dialog } = await import("electron");
    vi.mocked(app.isReady).mockReturnValue(true);
    vi.mocked(dialog.showMessageBox).mockResolvedValue({
      response: 1,
      checkboxChecked: false,
    });

    const result = await ensureNativeModules();
    expect(result).toBe(false);
    expect(dialog.showMessageBox).toHaveBeenCalledOnce();
  });

  it("returns false when ABI rebuild fails so startup does not continue", async () => {
    vi.resetModules();
    const mismatch = Object.assign(new Error("NODE_MODULE_VERSION 147 requires 146"), {
      code: "ERR_DLOPEN_FAILED",
    });
    nativeRequireMock.mockImplementation(() => {
      throw mismatch;
    });
    vi.mocked(execSync).mockImplementation(() => {
      throw new Error("rebuild failed");
    });

    const { app, dialog } = await import("electron");
    vi.mocked(app.isReady).mockReturnValue(true);
    vi.mocked(dialog.showMessageBox)
      .mockResolvedValueOnce({ response: 0, checkboxChecked: false })
      .mockResolvedValueOnce({ response: 0, checkboxChecked: false });

    const { ensureNativeModules } = await import(
      "../../electron/handlers/native-check"
    );

    const result = await ensureNativeModules();

    expect(result).toBe(false);
    expect(dialog.showMessageBox).toHaveBeenCalledTimes(2);
  });
});

describe("attemptRebuild", () => {
  it("calls electron-rebuild for better-sqlite3", async () => {
    vi.mocked(execSync).mockReturnValue(Buffer.from(""));

    const { attemptRebuild } = await import(
      "../../electron/handlers/native-check"
    );
    const result = await attemptRebuild();

    expect(execSync).toHaveBeenCalledOnce();
    expect(execSync).toHaveBeenCalledWith(
      "npx electron-rebuild -f -w better-sqlite3",
      expect.objectContaining({ timeout: 120_000 }),
    );
    expect(nativeRequireMock).toHaveBeenCalledOnce();
    expect(result).toBe(true);
  });
});
