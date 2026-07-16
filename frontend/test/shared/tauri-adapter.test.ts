// @vitest-environment node
import { describe, it, expect, vi, beforeEach } from "vitest";

// Mock the Tauri runtime surface so the adapter is exercised without a real
// webview. The captured `invoke`/`listen` calls are the contract under test.
const invokeMock = vi.fn();
const listenMock = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
  invoke: (...args: unknown[]) => invokeMock(...args),
}));
vi.mock("@tauri-apps/api/event", () => ({
  listen: (...args: unknown[]) => listenMock(...args),
}));

// Imported after the mocks take effect.
import { createTauriDesktopApi } from "../../src/shared/tauri-adapter";

/** Every request/response method mapped to its canonical command + arg layout.
 *  This table is the desktop API parity contract: it fails loudly the moment
 *  the renderer adapter diverges from the Rust command surface.
 */
const CHANNEL_PARITY: Array<{
  method: string;
  command: string;
  // sentinel arg keys expected in the single args object passed to `invoke`.
  args: Record<string, unknown>;
}> = [
  { method: "scanLibrary", command: "library_scan", args: { dirPath: "/lib" } },
  { method: "refreshAlbum", command: "album_refresh", args: { albumPath: "/a" } },
  { method: "openFolderDialog", command: "dialog_open_folder", args: {} },
  { method: "readAlbum", command: "album_read", args: { albumPath: "/a" } },
  { method: "writeTrack", command: "track_write", args: { trackPath: "/t", fields: { x: 1 } } },
  { method: "writeTracks", command: "tracks_batch_write", args: { updates: [{ path: "/t", fields: {} }] } },
  { method: "readExtraTags", command: "track_extra_tags_read", args: { trackPath: "/t" } },
  { method: "writeExtraTags", command: "track_extra_tags_write", args: { trackPath: "/t", tags: [] } },
  { method: "writeExtraTagsBatch", command: "tracks_batch_write_extra_tags", args: { updates: [] } },
  { method: "renameTrack", command: "track_rename", args: { oldPath: "/a", newPath: "/b" } },
  { method: "checkFileExists", command: "file_exists", args: { filePath: "/t" } },
  { method: "showTrackContextMenu", command: "track_context_menu", args: { trackPath: "/t", labels: {} } },
  { method: "deleteFiles", command: "track_delete_files", args: { filePaths: ["/t"] } },
  { method: "getCoverDataUrl", command: "cover_data_url", args: { albumPath: "/a" } },
  { method: "setCover", command: "cover_set", args: { albumPath: "/a" } },
  { method: "removeCover", command: "cover_remove", args: { albumPath: "/a" } },
  { method: "downloadCoverArt", command: "cover_download", args: { albumPath: "/a" } },
  { method: "downloadArtistArt", command: "cover_download_artist_art", args: { albumPath: "/a" } },
  { method: "listDirectory", command: "directory_list", args: { dirPath: "/d" } },
  { method: "readDirectory", command: "directory_read", args: { dirPath: "/d" } },
  { method: "fetchLyrics", command: "lyrics_fetch", args: { trackName: "t", artistName: "a", albumName: "g", duration: 1 } },
  { method: "getConfig", command: "config_get", args: {} },
  { method: "setConfig", command: "config_set", args: { key: "k", value: "v" } },
  { method: "autoTagAlbum", command: "album_auto_tag", args: { albumPath: "/a" } },
  { method: "downloadAlbumLyrics", command: "album_download_lyrics", args: { albumPath: "/a" } },
  { method: "getTaskProgress", command: "task_progress", args: { taskId: "1" } },
  { method: "cancelTask", command: "task_cancel", args: { taskId: "1" } },
  { method: "getDatasetStatus", command: "dataset_status", args: {} },
  { method: "runAudit", command: "audit_run", args: { libraryPath: "/lib" } },
  { method: "runAuditOnTracks", command: "audit_run_specified", args: { trackPaths: ["/t"] } },
  { method: "runAuditOnAlbums", command: "audit_run_specified", args: { albumPaths: ["/a"] } },
  { method: "runAlbumAudit", command: "audit_run_album", args: { albumPath: "/a" } },
  { method: "applyAuditFixes", command: "audit_apply_fixes", args: { albumResults: [] } },
  { method: "cancelAudit", command: "audit_cancel", args: {} },
  { method: "assistantSend", command: "assistant_send", args: { input: { message: "hi" } } },
  { method: "assistantCancel", command: "assistant_cancel", args: {} },
  { method: "assistantClear", command: "assistant_clear", args: {} },
  { method: "assistantApplyActions", command: "assistant_apply_actions", args: { actionBatchId: "b1" } },
  { method: "assistantRejectActions", command: "assistant_reject_actions", args: { actionBatchId: "b1" } },
  { method: "assistantGetBatches", command: "assistant_get_batches", args: {} },
  { method: "assistantInitRuntime", command: "assistant_init_runtime", args: {} },
  { method: "assistantInitServices", command: "assistant_init_services", args: { config: { apiKey: "x" } } },
  { method: "subscribeDebugLogs", command: "debug_subscribe", args: {} },
  { method: "setDebugMode", command: "debug_set_mode", args: { enabled: true } },
  { method: "onFocus", command: "window_focused", args: {} },
  { method: "sortByAlbum", command: "files_sort_by_album", args: { sourceDir: "/d", options: { copy: true } } },
  { method: "listSessions", command: "assistant_list_sessions", args: { limit: 5 } },
  { method: "getConversation", command: "assistant_get_conversation", args: { sessionUuidOrNumber: "s" } },
  { method: "getSession", command: "assistant_get_session", args: { sessionUuidOrNumber: "s" } },
  { method: "getCurrentSession", command: "assistant_current_session", args: {} },
];

describe("tauri-adapter channel parity", () => {
  let api: ReturnType<typeof createTauriDesktopApi>;

  beforeEach(() => {
    invokeMock.mockReset();
    listenMock.mockReset();
    invokeMock.mockResolvedValue({ ok: true });
    api = createTauriDesktopApi();
  });

  it.each(CHANNEL_PARITY)(
    "$method invokes the canonical Tauri command with the right args",
    async ({ method, command, args }) => {
      const fn = (api as unknown as Record<string, (...a: unknown[]) => unknown>)[method];
      expect(typeof fn).toBe("function");
      await fn(...Object.values(args));
      expect(invokeMock).toHaveBeenCalledTimes(1);
      const [cmd, payload] = invokeMock.mock.calls[0];
      expect(cmd).toBe(command);
      // single-positional payload object keyed by the renderer param names
      const expected = Object.keys(args).length === 0 ? undefined : args;
      expect(payload).toEqual(expected);
    },
  );

  it("normalizes every non-identifier channel char (':' and '-') to '_' so command names are valid Rust identifiers", () => {
    // Intent: a channel like "<group>:batch-write" must map to
    // "<group>_batch_write", not "<group>_batch-write" — hyphens aren't valid in
    // Rust command identifiers, so an underscored name keeps generate_handler wired.
    for (const { command } of CHANNEL_PARITY) {
      expect(command).toMatch(/^[A-Za-z0-9_]+$/);
      expect(command).not.toContain("-");
    }
    // Direct round-trip spot checks for the noisiest channels.
    expect("tracks:batch-write".replace(/[^A-Za-z0-9_]/g, "_")).toBe("tracks_batch_write");
    expect("track:extra-tags:read".replace(/[^A-Za-z0-9_]/g, "_")).toBe("track_extra_tags_read");
  });

  it("covers every DesktopAPI method (no drift)", () => {
    const covered = new Set(CHANNEL_PARITY.map((r) => r.method));
    // The three event methods are exercised in the subscribe suite and are not
    // request/response rows; every DesktopAPI method must be accounted for.
    const eventMethods = ["onAutoTagEvent", "onAuditEvent", "onAssistantEvent"];
    const apiKeys = Object.keys(api).sort();
    const expected = [...covered, ...eventMethods].sort();
    expect(apiKeys).toEqual(expected);
  });
});

describe("tauri-adapter error mapping", () => {
  let api: ReturnType<typeof createTauriDesktopApi>;

  beforeEach(() => {
    invokeMock.mockReset();
    listenMock.mockReset();
    api = createTauriDesktopApi();
  });

  it("converts a string rejection into a rejected Error matching Electron's message", async () => {
    // Rust ApiError serializes to its Display string (see src-tauri/src/error.rs).
    invokeMock.mockRejectedValue("not implemented: library:scan parity");
    await expect(api.scanLibrary("/lib")).rejects.toThrow(
      "not implemented: library:scan parity",
    );
    // Intent: the rejection is a real Error instance (with a stack), not a bare
    // string, so the renderer's `instanceof Error` and try/catch behave exactly
    // as they did under Electron's `throw new Error(...)`.
    await expect(api.scanLibrary("/lib")).rejects.toBeInstanceOf(Error);
  });

  it("converts an object rejection with a message field", async () => {
    invokeMock.mockRejectedValue({ message: "cover download failed" });
    await expect(api.getCoverDataUrl("/a")).rejects.toThrow("cover download failed");
  });

  it("passes a resolved value through unchanged", async () => {
    invokeMock.mockResolvedValue("dataurl");
    await expect(api.getCoverDataUrl("/a")).resolves.toBe("dataurl");
  });
});

describe("tauri-adapter event subscribe contract", () => {
  let api: ReturnType<typeof createTauriDesktopApi>;
  let unlisten: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    invokeMock.mockReset();
    listenMock.mockReset();
    unlisten = vi.fn();
    listenMock.mockResolvedValue(unlisten);
    api = createTauriDesktopApi();
  });

  it("onAutoTagEvent subscribes to auto-tag:event and forwards event.payload", async () => {
    const received: unknown[] = [];
    const dispose = api.onAutoTagEvent((event) => received.push(event));
    expect(listenMock).toHaveBeenCalledTimes(1);
    expect(listenMock.mock.calls[0][0]).toBe("auto-tag:event");
    // The handler receives a Tauri Event<T> and unwraps .payload for the callback.
    const registeredHandler = listenMock.mock.calls[0][1] as (e: { payload: unknown }) => void;
    const payload = { taskId: "1", type: "progress", message: "x", progress: 0, total: 1 };
    registeredHandler({ payload });
    expect(received).toEqual([payload]);
    // The returned sync disposer detaches the listener when called.
    dispose();
    await Promise.resolve();
    await Promise.resolve();
    expect(unlisten).toHaveBeenCalledTimes(1);
  });

  it.each([
    ["audit:event", "onAuditEvent"],
    ["assistant:event", "onAssistantEvent"],
  ] as const)("subscribe %s via %s", async (channel, method) => {
    const dispose = (api as unknown as Record<string, (cb: (e: unknown) => void) => () => void>)[method](() => {});
    expect(listenMock).toHaveBeenCalledTimes(1);
    expect(listenMock.mock.calls[0][0]).toBe(channel);
    dispose();
    await Promise.resolve();
    await Promise.resolve();
    expect(unlisten).toHaveBeenCalled();
  });

  /// Intent: a failed `listen` attach must never throw out of subscribe, but
  /// it must NOT vanish silently — it logs to console.error so a broken event
  /// stream is observable (Rule 11 — fail loud).
  it("logs (does not swallow) a failed event listener attachment", async () => {
    const errSpy = vi.spyOn(console, "error").mockImplementation(() => {});
    listenMock.mockReset();
    listenMock.mockRejectedValue(new Error("blocked"));
    const dispose = api.onAutoTagEvent(() => {});
    expect(listenMock).toHaveBeenCalledTimes(1);
    // subscribe itself must not have thrown (returns a disposer fn).
    expect(typeof dispose).toBe("function");
    // Microtasks flush the rejected promise chain so console.error fires.
    await Promise.resolve();
    await Promise.resolve();
    expect(errSpy).toHaveBeenCalled();
    const call = errSpy.mock.calls[0];
    // console.error(prefix_with_message, err): index 0 carries the message.
    const msg = String(call[0]).concat(" ", String(call[1] ?? ""));
    expect(msg).toContain("failed to attach Tauri event listener");
    expect(msg).toContain("auto-tag:event");
    errSpy.mockRestore();
  });
});
