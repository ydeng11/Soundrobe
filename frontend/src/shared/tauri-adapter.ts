/**
 * Tauri adapter for the renderer-neutral `DesktopAPI` contract.
 *
 * Mirrors the Electron channel mapping in `electron/preload.ts` so that every
 * method wires to its canonical Tauri command (`<group>:<action>` ->
 * `<group>_<action>`), and the pushed event streams subscribe via `listen`.
 *
 * Structured Rust command errors (see `src-tauri/src/error.rs`) serialize to a
 * display string; `invokeCommand` converts any rejection back into a rejected
 * JavaScript `Error`, preserving the renderer try/catch behavior established by
 * the Electron handlers.
 *
 * Request/response commands resolve once the matching Rust command is wired
 * (`generate_handler!`). Until a slice is green, an unregistered command rejects
 * with an `Error` rather than succeeding silently (Rule 11 — fail loud).
 */

import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { DesktopAPI } from "./desktop-api";

/** Map an Electron IPC channel to a Tauri command name (`:` -> `_`). */
function commandForChannel(channel: string): string {
  return channel.replace(/:/g, "_");
}

/** Convert any invoke rejection into a JavaScript `Error` with a stable message. */
function toError(reason: unknown): Error {
  if (reason instanceof Error) return reason;
  if (typeof reason === "string" && reason.length > 0) return new Error(reason);
  if (reason && typeof reason === "object" && "message" in reason) {
    const msg = String((reason as { message: unknown }).message || reason);
    return new Error(msg);
  }
  return new Error(String(reason ?? "Tauri command failed"));
}

/** Invoke a Tauri command, rejecting as a JS `Error` on failure. */
async function invokeCommand<T>(
  channel: string,
  args?: Record<string, unknown>,
): Promise<T> {
  try {
    return (await invoke(commandForChannel(channel), args)) as T;
  } catch (reason) {
    throw toError(reason);
  }
}

/**
 * Subscribe to a pushed Tauri event stream, returning the synchronous disposer
 * the `DesktopAPI` on* methods promise. Mirrors the Electron
 * `() => ipcRenderer.removeListener(...)` contract: the returned function is
 * safe to call immediately and asynchronously detaches the listener.
 */
function subscribe<T>(channel: string, callback: (payload: T) => void): () => void {
  // A failed attach is a no-op under Electron too; never throw from subscribe.
  const noopUnlisten: UnlistenFn = () => {};
  const unlisten: Promise<UnlistenFn> = listen<T>(channel, (event) =>
    callback(event.payload),
  ).then(
    (fn) => fn,
    () => noopUnlisten,
  );
  return () => {
    unlisten.then((fn) => fn()).catch(() => {});
  };
}

/** Build the `DesktopAPI` facade backed by Tauri commands and events. */
export function createTauriDesktopApi(): DesktopAPI {
  return {
    // Library
    scanLibrary: (dirPath) => invokeCommand("library:scan", { dirPath }),
    refreshAlbum: (albumPath) => invokeCommand("album:refresh", { albumPath }),

    // Dialogs
    openFolderDialog: () => invokeCommand("dialog:open-folder", undefined),

    // Tracks
    readAlbum: (albumPath) => invokeCommand("album:read", { albumPath }),
    writeTrack: (trackPath, fields) =>
      invokeCommand("track:write", { trackPath, fields }),
    writeTracks: (updates) => invokeCommand("tracks:batch-write", { updates }),
    readExtraTags: (trackPath) =>
      invokeCommand("track:extra-tags:read", { trackPath }),
    writeExtraTags: (trackPath, tags) =>
      invokeCommand("track:extra-tags:write", { trackPath, tags }),
    writeExtraTagsBatch: (updates) =>
      invokeCommand("tracks:batch-write-extra-tags", { updates }),
    renameTrack: (oldPath, newPath) =>
      invokeCommand("track:rename", { oldPath, newPath }),
    checkFileExists: (filePath) => invokeCommand("file:exists", { filePath }),

    showTrackContextMenu: (trackPath, labels) =>
      invokeCommand("track:context-menu", { trackPath, labels }),

    deleteFiles: (filePaths) => invokeCommand("track:delete-files", { filePaths }),

    // Cover
    getCoverDataUrl: (albumPath) => invokeCommand("cover:data-url", { albumPath }),
    setCover: (albumPath) => invokeCommand("cover:set", { albumPath }),
    removeCover: (albumPath) => invokeCommand("cover:remove", { albumPath }),
    downloadCoverArt: (albumPath) => invokeCommand("cover:download", { albumPath }),
    downloadArtistArt: (albumPath) =>
      invokeCommand("cover:download-artist-art", { albumPath }),

    // Directory browser
    listDirectory: (dirPath) => invokeCommand("directory:list", { dirPath }),
    readDirectory: (dirPath) => invokeCommand("directory:read", { dirPath }),

    // Lyrics
    fetchLyrics: (trackName, artistName, albumName, duration) =>
      invokeCommand("lyrics:fetch", {
        trackName,
        artistName,
        albumName,
        duration,
      }),

    // Config
    getConfig: () => invokeCommand("config:get", undefined),
    setConfig: (key, value) => invokeCommand("config:set", { key, value }),

    // Auto-tag
    autoTagAlbum: (albumPath) => invokeCommand("album:auto-tag", { albumPath }),
    downloadAlbumLyrics: (albumPath) =>
      invokeCommand("album:download-lyrics", { albumPath }),
    onAutoTagEvent: (callback) =>
      subscribe("auto-tag:event", callback),
    getTaskProgress: (taskId) => invokeCommand("task:progress", { taskId }),
    cancelTask: (taskId) => invokeCommand("task:cancel", { taskId }),
    getDatasetStatus: () => invokeCommand("dataset:status", undefined),

    // Audit
    runAudit: (libraryPath) => invokeCommand("audit:run", { libraryPath }),
    runAuditOnTracks: (trackPaths) =>
      invokeCommand("audit:run-specified", { trackPaths }),
    runAuditOnAlbums: (albumPaths) =>
      invokeCommand("audit:run-specified", { albumPaths }),
    runAlbumAudit: (albumPath) => invokeCommand("audit:run-album", { albumPath }),
    applyAuditFixes: (albumResults) =>
      invokeCommand("audit:apply-fixes", { albumResults }),
    onAuditEvent: (callback) => subscribe("audit:event", callback),
    cancelAudit: () => invokeCommand("audit:cancel", undefined),

    // Assistant
    assistantSend: (input) => invokeCommand("assistant:send", { input }),
    assistantCancel: () => invokeCommand("assistant:cancel", undefined),
    assistantClear: () => invokeCommand("assistant:clear", undefined),
    assistantApplyActions: (actionBatchId) =>
      invokeCommand("assistant:apply-actions", { actionBatchId }),
    assistantRejectActions: (actionBatchId) =>
      invokeCommand("assistant:reject-actions", { actionBatchId }),
    assistantGetBatches: () => invokeCommand("assistant:get-batches", undefined),
    assistantInitRuntime: () =>
      invokeCommand("assistant:init-runtime", undefined),
    assistantInitServices: (config) =>
      invokeCommand("assistant:init-services", { config }),
    onAssistantEvent: (callback) => subscribe("assistant:event", callback),

    // Debug
    subscribeDebugLogs: () => invokeCommand("debug:subscribe", undefined),
    setDebugMode: (enabled) => invokeCommand("debug:set-mode", { enabled }),

    // Window events
    onFocus: () => invokeCommand("window:focused", undefined),

    // Organizer
    sortByAlbum: (sourceDir, options) =>
      invokeCommand("files:sort-by-album", { sourceDir, options }),

    // Conversation logs
    listSessions: (limit) => invokeCommand("assistant:list-sessions", { limit }),
    getConversation: (sessionUuidOrNumber) =>
      invokeCommand("assistant:get-conversation", { sessionUuidOrNumber }),
    getSession: (sessionUuidOrNumber) =>
      invokeCommand("assistant:get-session", { sessionUuidOrNumber }),
    getCurrentSession: () =>
      invokeCommand("assistant:current-session", undefined),
  };
}