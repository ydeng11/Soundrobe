import { contextBridge, ipcRenderer } from "electron";
import type {
  AlbumInfo,
  CoverInfo,
  TrackData,
  AlbumDetail,
  TaskProgress,
  AuditTrackResult,
  AuditEvent,
  AuditRunSummary,
  AuditApplyFixesSummary,
  AutoTagEvent,
  DatasetStatus,
  DirEntry,
  DirectoryData,
  TrackUndoSnapshot,
  ExtraTagUndoSnapshot,
  AssistantAction,
  AssistantActionBatch,
  AssistantEvent,
  LogEntry,
  ExtraTag,
  ExtraTagUpdate,
  SortByAlbumResult,
  SessionSummary,
  ConversationEntry,
  DesktopAPI,
} from "../src/shared/desktop-api";

export type {
  AlbumInfo,
  CoverInfo,
  TrackData,
  AlbumDetail,
  TaskProgress,
  AuditTrackResult,
  AuditEvent,
  AuditRunSummary,
  AuditApplyFixesSummary,
  AutoTagEvent,
  DatasetStatus,
  DirEntry,
  DirectoryData,
  TrackUndoSnapshot,
  ExtraTagUndoSnapshot,
  AssistantAction,
  AssistantActionBatch,
  AssistantEvent,
  LogEntry,
  ExtraTag,
  ExtraTagUpdate,
  SortByAlbumResult,
  SessionSummary,
  ConversationEntry,
  DesktopAPI,
} from "../src/shared/desktop-api";

/**
 * @deprecated Prefer the renderer-neutral `DesktopAPI` type from
 * `src/shared/desktop-api.ts`. Kept so existing imports of `ElectronAPI`
 * continue to resolve during the Tauri migration.
 */
export type ElectronAPI = DesktopAPI;

contextBridge.exposeInMainWorld("api", {
  // Library
  scanLibrary: (dirPath: string) =>
    ipcRenderer.invoke("library:scan", dirPath),
  // Dialogs
  openFolderDialog: (): Promise<string | null> =>
    ipcRenderer.invoke("dialog:open-folder"),
  refreshAlbum: (albumPath: string) =>
    ipcRenderer.invoke("album:refresh", albumPath),

  // Tracks
  readAlbum: (albumPath: string) =>
    ipcRenderer.invoke("album:read", albumPath),
  writeTrack: (
    trackPath: string,
    fields: Record<string, unknown>
  ): Promise<TrackData> =>
    ipcRenderer.invoke("track:write", trackPath, fields),
  writeTracks: (
    updates: Array<{ path: string; fields: Record<string, unknown> }>
  ): Promise<TrackData[]> =>
    ipcRenderer.invoke("tracks:batch-write", updates),
  readExtraTags: (trackPath: string): Promise<ExtraTag[]> =>
    ipcRenderer.invoke("track:extra-tags:read", trackPath),
  writeExtraTags: (
    trackPath: string,
    tags: ExtraTagUpdate[]
  ): Promise<TrackData> =>
    ipcRenderer.invoke("track:extra-tags:write", trackPath, tags),
  writeExtraTagsBatch: (
    updates: Array<{ path: string; tags: ExtraTagUpdate[] }>
  ): Promise<TrackData[]> =>
    ipcRenderer.invoke("tracks:batch-write-extra-tags", updates),
  renameTrack: (oldPath: string, newPath: string): Promise<TrackData> =>
    ipcRenderer.invoke("track:rename", oldPath, newPath),
  checkFileExists: (filePath: string): Promise<boolean> =>
    ipcRenderer.invoke("file:exists", filePath),

  showTrackContextMenu: (
    trackPath: string,
    labels: Record<string, string>
  ): Promise<"extra-tags" | "delete-files" | null> =>
    ipcRenderer.invoke("track:context-menu", trackPath, labels),

  deleteFiles: (
    filePaths: string[]
  ): Promise<
    { path: string; success: boolean; error?: string }[]
  > => ipcRenderer.invoke("track:delete-files", filePaths),

  // Cover
  getCoverDataUrl: (albumPath: string) =>
    ipcRenderer.invoke("cover:data-url", albumPath),
  setCover: (albumPath: string): Promise<string | null> =>
    ipcRenderer.invoke("cover:set", albumPath),
  removeCover: (albumPath: string): Promise<boolean> =>
    ipcRenderer.invoke("cover:remove", albumPath),
  downloadCoverArt: (albumPath: string): Promise<string | null> =>
    ipcRenderer.invoke("cover:download", albumPath),
  downloadArtistArt: (
    albumPath: string,
  ): Promise<{ path: string; source: string } | null> =>
    ipcRenderer.invoke("cover:download-artist-art", albumPath),

  // Directory browser
  listDirectory: (dirPath: string) =>
    ipcRenderer.invoke("directory:list", dirPath),
  readDirectory: (dirPath: string) =>
    ipcRenderer.invoke("directory:read", dirPath),

  // Lyrics
  fetchLyrics: (
    trackName: string,
    artistName: string,
    albumName?: string,
    duration?: number,
  ): Promise<string | null> =>
    ipcRenderer.invoke("lyrics:fetch", trackName, artistName, albumName, duration),

  // Config
  getConfig: () => ipcRenderer.invoke("config:get"),
  setConfig: (key: string, value: unknown) =>
    ipcRenderer.invoke("config:set", key, value),

  // Auto-tag
  autoTagAlbum: (albumPath: string) =>
    ipcRenderer.invoke("album:auto-tag", albumPath),
  downloadAlbumLyrics: (albumPath: string) =>
    ipcRenderer.invoke("album:download-lyrics", albumPath),
  onAutoTagEvent: (callback: (event: AutoTagEvent) => void): (() => void) => {
    const listener = (_event: Electron.IpcRendererEvent, payload: AutoTagEvent) =>
      callback(payload);
    ipcRenderer.on("auto-tag:event", listener);
    return () => ipcRenderer.removeListener("auto-tag:event", listener);
  },
  getTaskProgress: (taskId: string) =>
    ipcRenderer.invoke("task:progress", taskId),
  cancelTask: (taskId: string) =>
    ipcRenderer.invoke("task:cancel", taskId),
  getDatasetStatus: () => ipcRenderer.invoke("dataset:status"),

  // Audit
  runAudit: (libraryPath: string) =>
    ipcRenderer.invoke("audit:run", libraryPath),
  runAuditOnTracks: (trackPaths: string[]) =>
    ipcRenderer.invoke("audit:run-specified", { trackPaths }),
  runAuditOnAlbums: (albumPaths: string[]) =>
    ipcRenderer.invoke("audit:run-specified", { albumPaths }),
  runAlbumAudit: (albumPath: string) =>
    ipcRenderer.invoke("audit:run-album", albumPath),
  applyAuditFixes: (albumResults: NonNullable<AuditRunSummary["albumResults"]>) =>
    ipcRenderer.invoke("audit:apply-fixes", albumResults),
  onAuditEvent: (callback: (event: AuditEvent) => void): (() => void) => {
    const listener = (_event: Electron.IpcRendererEvent, payload: AuditEvent) =>
      callback(payload);
    ipcRenderer.on("audit:event", listener);
    return () => ipcRenderer.removeListener("audit:event", listener);
  },
  cancelAudit: () => ipcRenderer.invoke("audit:cancel"),

  // Assistant
  assistantSend: (input: any) =>
    ipcRenderer.invoke("assistant:send", input),
  assistantCancel: () =>
    ipcRenderer.invoke("assistant:cancel"),
  assistantClear: () =>
    ipcRenderer.invoke("assistant:clear"),
  assistantApplyActions: (actionBatchId: string) =>
    ipcRenderer.invoke("assistant:apply-actions", actionBatchId),
  assistantRejectActions: (actionBatchId: string) =>
    ipcRenderer.invoke("assistant:reject-actions", actionBatchId),
  assistantGetBatches: () =>
    ipcRenderer.invoke("assistant:get-batches"),
  assistantInitRuntime: () =>
    ipcRenderer.invoke("assistant:init-runtime"),
  assistantInitServices: (config: any) =>
    ipcRenderer.invoke("assistant:init-services", config),
  onAssistantEvent: (callback: (event: AssistantEvent) => void): (() => void) => {
    const listener = (_event: Electron.IpcRendererEvent, payload: AssistantEvent) =>
      callback(payload);
    ipcRenderer.on("assistant:event", listener);
    return () => ipcRenderer.removeListener("assistant:event", listener);
  },

  // Debug — subscribe to live log forwarding from main process
  subscribeDebugLogs: () =>
    ipcRenderer.invoke("debug:subscribe"),

  // Debug — toggle debug mode
  setDebugMode: (enabled: boolean) =>
    ipcRenderer.invoke("debug:set-mode", enabled),

  // Window events
  onFocus: () => ipcRenderer.invoke("window:focused"),

  // Organizer
  sortByAlbum: (
    sourceDir: string,
    options?: { copy?: boolean }
  ): Promise<SortByAlbumResult> =>
    ipcRenderer.invoke("files:sort-by-album", sourceDir, options),

  // Conversation logs
  listSessions: (limit?: number): Promise<SessionSummary[]> =>
    ipcRenderer.invoke("assistant:list-sessions", limit),
  getConversation: (sessionUuidOrNumber: string): Promise<ConversationEntry[]> =>
    ipcRenderer.invoke("assistant:get-conversation", sessionUuidOrNumber),
  getSession: (sessionUuidOrNumber: string): Promise<SessionSummary | null> =>
    ipcRenderer.invoke("assistant:get-session", sessionUuidOrNumber),
  getCurrentSession: (): Promise<{ sessionId: string; sessionNumber: string } | null> =>
    ipcRenderer.invoke("assistant:current-session"),
});

const CONSOLE_METHOD: Record<string, "error" | "warn" | "debug" | "log"> = {
  error: "error",
  warn: "warn",
  debug: "debug",
};

// Listen for debug log events pushed from main process
ipcRenderer.on("debug:log", (_event, entry: LogEntry) => {
  const prefix = `[${entry.tag}] ${entry.level.toUpperCase()}`;
  const method = CONSOLE_METHOD[entry.level] ?? "log";
  console[method](`[auto-tagger] ${prefix} ${entry.message}`, entry.data ?? "");
});
