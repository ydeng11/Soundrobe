/**
 * Fetch-backed adapter for the renderer-neutral `DesktopAPI` contract.
 *
 * The browser keeps the same method names and payloads as the Tauri adapter;
 * only the transport changes from `invoke` to authenticated JSON POSTs. Native
 * dialogs, menus, and self-updates are intentionally explicit web-runtime
 * errors because those affordances belong to the browser or the container.
 */

import type { DesktopAPI } from "./desktop-api";

export interface WebDesktopApiOptions {
  fetch?: typeof globalThis.fetch;
  baseUrl?: string;
}

function toError(reason: unknown): Error {
  if (reason instanceof Error) return reason;
  if (typeof reason === "string" && reason.length > 0) return new Error(reason);
  if (reason && typeof reason === "object" && "message" in reason) {
    return new Error(String((reason as { message: unknown }).message || reason));
  }
  return new Error(String(reason ?? "Web command failed"));
}

function unsupportedWebAction(action: string): never {
  throw new Error(`${action} is unavailable in the web runtime`);
}

function commandUrl(baseUrl: string, channel: string): string {
  return `${baseUrl}/api/v1/commands/${encodeURIComponent(channel)}`;
}

async function requestCommand<T>(
  fetchImpl: typeof globalThis.fetch,
  baseUrl: string,
  channel: string,
  payload: Record<string, unknown> = {},
): Promise<T> {
  let response: Response;
  try {
    response = await fetchImpl(commandUrl(baseUrl, channel), {
      method: "POST",
      credentials: "same-origin",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(payload),
    });
  } catch (reason) {
    throw toError(reason);
  }

  const text = await response.text();
  let body: unknown = undefined;
  if (text.length > 0) {
    try {
      body = JSON.parse(text) as unknown;
    } catch {
      body = text;
    }
  }
  if (!response.ok) {
    const message =
      body && typeof body === "object" && "error" in body
        ? String((body as { error: unknown }).error)
        : `Web command failed (${response.status})`;
    throw new Error(message);
  }
  return body as T;
}

/** Build the `DesktopAPI` facade backed by the headless HTTP service. */
export function createWebDesktopApi(options: WebDesktopApiOptions = {}): DesktopAPI {
  const fetchImpl = options.fetch ?? globalThis.fetch.bind(globalThis);
  const baseUrl = (options.baseUrl ?? "").replace(/\/$/, "");
  const command = <T>(channel: string, payload?: Record<string, unknown>) =>
    requestCommand<T>(fetchImpl, baseUrl, channel, payload);

  return {
    // App
    appInfo: () => command("app:info"),
    checkForUpdate: async () => null,
    installUpdate: async () => unsupportedWebAction("installUpdate"),

    // Library
    listLibraryRoots: () => command("library:list-roots"),
    scanLibrary: (dirPath) => command("library:scan", { dirPath }),
    refreshAlbum: (albumPath) => command("album:refresh", { albumPath }),

    // Browser replacements for native dialogs are supplied by the React UI.
    openFolderDialog: async () => unsupportedWebAction("openFolderDialog"),

    // Tracks
    readAlbum: (albumPath) => command("album:read", { albumPath }),
    writeTrack: (path, fields) => command("track:write", { path, fields }),
    writeTracks: (updates) => command("tracks:batch-write", { updates }),
    readExtraTags: (trackPath) => command("track:extra-tags:read", { trackPath }),
    writeExtraTags: (trackPath, tags) =>
      command("track:extra-tags:write", { trackPath, tags }),
    writeExtraTagsBatch: (updates) =>
      command("tracks:batch-write-extra-tags", { updates }),
    renameTrack: (oldPath, newPath) => command("track:rename", { oldPath, newPath }),
    checkFileExists: (filePath) => command("file:exists", { filePath }),
    probeWriteVolume: (filePath) => command("volume:probe-write", { path: filePath }),
    probeWriteVolumeReal: (filePath, patch) =>
      command("volume:probe-write-real", { path: filePath, patchJson: patch }),
    showTrackContextMenu: async () => unsupportedWebAction("showTrackContextMenu"),
    deleteFiles: (filePaths) => command("track:delete-files", { filePaths }),

    // Cover
    getCoverDataUrl: (albumPath, preferredTrackPath) =>
      command("cover:data-url", { albumPath, preferredTrackPath }),
    setCover: (albumPath) => command("cover:set", { albumPath }),
    removeCover: (albumPath) => command("cover:remove", { albumPath }),
    downloadCoverArt: (albumPath) => command("cover:download", { albumPath }),
    downloadArtistArt: (albumPath) =>
      command("cover:download-artist-art", { albumPath }),

    // Directory browser
    listDirectory: (dirPath) => command("directory:list", { dirPath }),
    readDirectory: (dirPath) => command("directory:read", { dirPath }),

    // Lyrics
    fetchLyrics: (trackName, artistName, albumName, duration) =>
      command("lyrics:fetch", { trackName, artistName, albumName, duration }),

    // Config
    getConfig: () => command("config:get"),
    setConfig: (key, value) => command("config:set", { key, value }),

    // Auto-tag
    autoTagAlbum: (albumPath) => command("album:auto-tag", { albumPath }),
    downloadAlbumLyrics: (albumPath) =>
      command("album:download-lyrics", { albumPath }),
    onAutoTagEvent: () => unsupportedWebAction("onAutoTagEvent"),
    onTrackWriteEvent: () => unsupportedWebAction("onTrackWriteEvent"),
    getTaskProgress: (taskId) => command("task:progress", { taskId }),
    cancelTask: (taskId) => command("task:cancel", { taskId }),
    getDatasetStatus: () => command("dataset:status"),

    // Audit
    runAudit: (libraryPath) => command("audit:run", { libraryPath }),
    runAuditOnTracks: (trackPaths) =>
      command("audit:run-specified", { trackPaths }),
    runAuditOnAlbums: (albumPaths) =>
      command("audit:run-specified", { albumPaths }),
    runAlbumAudit: (albumPath) => command("audit:run-album", { albumPath }),
    applyAuditFixes: (albumResults) =>
      command("audit:apply-fixes", { albumResults }),
    onAuditEvent: () => unsupportedWebAction("onAuditEvent"),
    cancelAudit: () => command("audit:cancel"),

    // Assistant
    assistantSend: (input) => command("assistant:send", { input }),
    assistantCancel: () => command("assistant:cancel"),
    assistantClear: () => command("assistant:clear"),
    assistantApplyActions: (actionBatchId) =>
      command("assistant:apply-actions", { actionBatchId }),
    assistantCompleteTaskActions: (actionBatchId, error) =>
      command("assistant:complete-task-actions", {
        actionBatchId,
        error: error ?? null,
      }),
    assistantRejectActions: (actionBatchId) =>
      command("assistant:reject-actions", { actionBatchId }),
    assistantGetBatches: () => command("assistant:get-batches"),
    assistantInitRuntime: () => command("assistant:init-runtime"),
    assistantInitServices: (config) => command("assistant:init-services", { config }),
    testLlmConnection: (apiKey, model, provider, baseUrl) =>
      command("test-llm-connection", { apiKey, model, provider, baseUrl }),
    onAssistantEvent: () => unsupportedWebAction("onAssistantEvent"),

    // Debug and window focus are desktop-only until the web event bus lands.
    subscribeDebugLogs: () => command("debug:subscribe"),
    setDebugMode: (enabled) => command("debug:set-mode", { enabled }),
    onFocus: async () => {},

    // Manual search
    searchReleases: (request) => command("album:search-releases", { request }),
    resolveRelease: (provider, releaseId, kind) =>
      command("album:resolve-release", {
        request: { provider, releaseId, kind },
      }),
    previewReleaseMatch: (request) =>
      command("album:preview-release-match", { request }),
    searchApplyCandidate: (albumPath, candidate, selectedTrackIndices) =>
      command("album:search-apply-candidate", {
        request: { albumPath, candidate, selectedTrackIndices },
      }),

    // Organizer
    sortByAlbum: (sourceDir, options) =>
      command("files:sort-by-album", { sourceDir, options }),

    // Conversation logs
    listSessions: (limit) => command("assistant:list-sessions", { limit }),
    getConversation: (sessionUuidOrNumber) =>
      command("assistant:get-conversation", { sessionUuidOrNumber }),
    getSession: (sessionUuidOrNumber) =>
      command("assistant:get-session", { sessionUuidOrNumber }),
    getCurrentSession: () => command("assistant:current-session"),
  };
}
