// @vitest-environment jsdom
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

const listenMock = vi.fn();
vi.mock("@tauri-apps/api/event", () => ({
  listen: (...args: unknown[]) => listenMock(...args),
}));
vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
}));

import {
  installDesktopApi,
  isTauriRuntime,
} from "../../src/shared/install-desktop-api";

describe("install-desktop-api loader", () => {
  beforeEach(() => {
    listenMock.mockReset();
    listenMock.mockResolvedValue(() => {});
    delete (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
    delete (window as unknown as { api?: unknown }).api;
  });

  afterEach(() => {
    delete (window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__;
    delete (window as unknown as { api?: unknown }).api;
  });

  it("is a no-op under Electron (no window.api mutation)", () => {
    // Intent: the Electron preload already exposes a frozen context-bridged
    // window.api; the loader must not overwrite or remove it.
    (window as unknown as { api: unknown }).api = "electron-frozen";
    expect(isTauriRuntime()).toBe(false);
    installDesktopApi();
    expect((window as unknown as { api: unknown }).api).toBe("electron-frozen");
  });

  it("installs a DesktopAPI facade under Tauri and wires debug:log", () => {
    (window as unknown as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {};
    expect(isTauriRuntime()).toBe(true);
    installDesktopApi();
    const api = (window as unknown as { api: unknown }).api;
    expect(typeof api).toBe("object");
    expect(api).not.toBeNull();
    expect(typeof (api as { scanLibrary: unknown }).scanLibrary).toBe("function");
    // The pushed debug:log console forwarder must attach exactly once.
    expect(listenMock).toHaveBeenCalledTimes(1);
    expect(listenMock.mock.calls[0][0]).toBe("debug:log");
  });

  it("is idempotent under Tauri (does not recreate window.api)", () => {
    (window as unknown as { __TAURI_INTERNALS__: unknown }).__TAURI_INTERNALS__ = {};
    installDesktopApi();
    const first = (window as unknown as { api: unknown }).api;
    installDesktopApi();
    expect((window as unknown as { api: unknown }).api).toBe(first);
    // debug:log still attached only once from the first install.
    expect(listenMock).toHaveBeenCalledTimes(1);
  });
});