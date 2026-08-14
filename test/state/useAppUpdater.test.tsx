// @vitest-environment jsdom
import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useAppUpdater } from "../../src/state/useAppUpdater";
import type { AppUpdateInfo } from "../../src/shared/desktop-api";

const update: AppUpdateInfo = {
  currentVersion: "0.1.0",
  availableVersion: "0.2.0",
  date: null,
  notes: "Update notes",
};

beforeEach(() => {
  window.api = {
    appInfo: vi.fn().mockResolvedValue({
      identifier: "com.ihelio.soundrobe",
      version: "0.1.0",
      runtime: "tauri",
      dev: false,
    }),
    checkForUpdate: vi.fn().mockResolvedValue(update),
    installUpdate: vi.fn().mockResolvedValue(undefined),
  } as any;
});

describe("useAppUpdater", () => {
  it("checks once on startup and silently presents an available update", async () => {
    const { result, rerender } = renderHook(() => useAppUpdater(false));

    await waitFor(() => expect(result.current.update).toEqual(update));
    rerender();
    expect(window.api.checkForUpdate).toHaveBeenCalledTimes(1);
    expect(result.current.checkMessage).toBeNull();
  });

  it("disables checks in development builds without contacting the updater", async () => {
    window.api.appInfo = vi.fn().mockResolvedValue({
      identifier: "com.ihelio.soundrobe",
      version: "0.1.0",
      runtime: "tauri",
      dev: true,
    });
    const { result } = renderHook(() => useAppUpdater(false));

    await waitFor(() => expect(result.current.supported).toBe(false));
    expect(window.api.checkForUpdate).not.toHaveBeenCalled();

    await act(async () => result.current.checkManually());
    expect(window.api.checkForUpdate).not.toHaveBeenCalled();
    expect(result.current.checkMessage).toBe(
      "Updates are available in packaged production builds.",
    );
  });

  it("keeps startup failures silent but exposes manual failures", async () => {
    window.api.checkForUpdate = vi.fn().mockRejectedValue(new Error("offline"));
    const { result } = renderHook(() => useAppUpdater(false));

    await waitFor(() => expect(window.api.checkForUpdate).toHaveBeenCalledTimes(1));
    expect(result.current.checkMessage).toBeNull();

    await act(async () => result.current.checkManually());
    expect(result.current.checkMessage).toBe("Could not check for updates: offline");
  });

  it("reports a manual no-update result", async () => {
    window.api.checkForUpdate = vi.fn().mockResolvedValue(null);
    const { result } = renderHook(() => useAppUpdater(false));
    await waitFor(() => expect(window.api.checkForUpdate).toHaveBeenCalledTimes(1));

    await act(async () => result.current.checkManually());

    expect(result.current.checkMessage).toBe("Soundrobe is up to date.");
  });

  it("forwards progress and retains the pending update for retry after failure", async () => {
    window.api.installUpdate = vi.fn(async (onProgress) => {
      onProgress({ phase: "downloading", downloaded: 50, total: 100 });
      throw new Error("connection lost");
    });
    const { result } = renderHook(() => useAppUpdater(false));
    await waitFor(() => expect(result.current.update).toEqual(update));

    await act(async () => result.current.install());

    expect(result.current.update).toEqual(update);
    expect(result.current.progress).toEqual({
      phase: "downloading",
      downloaded: 50,
      total: 100,
    });
    expect(result.current.installError).toBe("connection lost");
    expect(result.current.installing).toBe(false);
  });

  it("refuses installation when protected app work is busy", async () => {
    const { result } = renderHook(() => useAppUpdater(true));
    await waitFor(() => expect(result.current.update).toEqual(update));

    await act(async () => result.current.install());

    expect(window.api.installUpdate).not.toHaveBeenCalled();
    expect(result.current.installError).toMatch(/disk operation/i);
  });
});
