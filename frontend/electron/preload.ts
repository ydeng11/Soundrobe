import { contextBridge, ipcRenderer } from "electron";

export interface AlbumInfo {
  path: string;
  name: string;
  artistHint: string;
  albumHint: string;
  trackCount: number;
}

export interface CoverInfo {
  path: string | null;
  source: "external" | "embedded" | "missing";
  dataUrl: string | null;
}

export interface TrackData {
  path: string;
  title: string | null;
  artist: string | null;
  artists: string[];
  album: string | null;
  albumArtist: string | null;
  albumArtists: string[];
  trackNumber: number | null;
  trackTotal: number | null;
  discNumber: number | null;
  discTotal: number | null;
  year: string | null;
  genre: string | null;
  composer: string | null;
  comment: string | null;
  description: string | null;
  lyrics: string | null;
  compilation: boolean | null;
  musicbrainzTrackId: string | null;
  musicbrainzAlbumId: string | null;
  musicbrainzArtistId: string | null;
  hasCover: boolean;
  sizeBytes: number;
  bitrate: number | null;
  sampleRate: number | null;
  codec: string;
  duration: number;
}

export interface AlbumDetail {
  path: string;
  name: string;
  artistHint: string;
  albumHint: string;
  tracks: TrackData[];
  coverInfo: CoverInfo;
  status: string;
  trackFiles?: string[];
  tracksLoaded?: boolean;
  auditResults?: Array<{
    trackIndex: number;
    field: string;
    status: "correct" | "warning" | "error";
    message: string | null;
    suggestion: string | null;
  }>;
}

export interface TaskProgress {
  taskId: string;
  status: "running" | "completed" | "failed" | "cancelled";
  progress: number;
  total: number;
  message: string;
  result: unknown;
}

export interface AuditTrackResult {
  index: number;
  field: string;
  status: "correct" | "warning" | "error";
  message: string;
  suggestion?: string | null;
  corrected?: {
    title?: string | null;
    artist?: string | null;
    artists?: string[] | null;
    album?: string | null;
    albumArtist?: string | null;
    year?: string | null;
    genre?: string | null;
  } | null;
}

export interface AuditEvent {
  type:
    | "progress"
    | "album-start"
    | "album-result"
    | "album-error"
    | "completed"
    | "cancelled"
    | "failed";
  albumPath?: string;
  current?: number;
  total?: number;
  message?: string;
  results?: AuditTrackResult[];
}

export interface AutoTagEvent {
  taskId: string;
  type:
    | "progress"
    | "lookup"
    | "source"
    | "merge"
    | "write"
    | "warning"
    | "completed"
    | "failed"
    | "cancelled";
  message: string;
  progress: number;
  total: number;
  data?: unknown;
}

export interface DatasetStatus {
  musicbrainz: boolean;
  spotify: boolean;
  totalRecords: number;
  lastUpdated: string | null;
}

export interface DirEntry {
  name: string;
  path: string;
  isDirectory: boolean;
}

export interface DirectoryData {
  path: string;
  name: string;
  subdirs: DirEntry[];
  tracks: TrackData[];
  audioCount: number;
}

export interface TrackUndoSnapshot {
  /** Path of the track that was changed. */
  path: string;
  /** Previous tag values: field name → old value. */
  metadata: Record<string, unknown>;
}

export interface ExtraTagUndoSnapshot {
  /** Path of the track that was changed. */
  path: string;
  /** Previous extra tags for this track. */
  extraTags: Array<{ key: string; value: string }>;
}

export interface AssistantAction {
  tagKind?: "standard" | "extra";
  trackPath?: string;
  field?: string;
  oldValue?: string | null;
  newValue?: string | null;
  operation?: string;
  destinationPath?: string;
  sourcePath?: string;
  skipReason?: string;
  description?: string;
}

export interface AssistantActionBatch {
  id: string;
  createdAt: string;
  sessionId: string;
  kind: "tag-update" | "extra-tag-update" | "metadata-update" | "folder-move" | "auto-tag-run" | "audit-run";
  title: string;
  summary: string;
  riskLevel: "low" | "medium" | "high";
  actions: AssistantAction[];
  reversible: boolean;
  status: "pending" | "applied" | "rejected" | "failed";
}

export interface AssistantEvent {
  sessionId: string;
  type:
    | "step"
    | "tool_running"
    | "tool_result"
    | "action_batch_created"
    | "action_batch_applied"
    | "action_batch_rejected"
    | "action_batch_failed"
    | "message"
    | "error"
    | "completed"
    | "cancelled";
  message: string;
  data?: unknown;
}

export interface LogEntry {
  timestamp: string;
  tag: string;
  level: "info" | "warn" | "error" | "debug";
  message: string;
  data?: unknown;
}

export interface ExtraTag {
  key: string;
  value: string;
  source: string;
}

export interface ExtraTagUpdate {
  key: string;
  value: string;
}

export interface SortByAlbumResult {
  sourceDir: string;
  albums: Array<{
    albumName: string;
    destDir: string;
    files: Array<{
      sourcePath: string;
      destPath: string;
      success: boolean;
      error?: string;
    }>;
  }>;
  totalFiles: number;
  skippedFiles: number;
}

export interface SessionSummary {
  sessionNumber: string;
  sessionUuid: string;
  entryCount: number;
  firstMessage: string | null;
  lastActivity: string;
  apiCallCount: number;
  totalCost: number;
}

export interface ConversationEntry {
  id: number;
  sessionUuid: string;
  sessionNumber: string;
  timestamp: string;
  entryType: string;
  content: string;
  model: string | null;
  promptTokens: number;
  completionTokens: number;
  totalTokens: number;
  cost: number;
  metadata: string | null;
}

export interface ElectronAPI {
  // Library
  scanLibrary: (dirPath: string) => Promise<AlbumInfo[]>;
  refreshAlbum: (albumPath: string) => Promise<AlbumDetail>;

  // Dialogs
  openFolderDialog: () => Promise<string | null>;

  // Tracks
  readAlbum: (albumPath: string) => Promise<AlbumDetail>;
  writeTrack: (
    trackPath: string,
    fields: Record<string, unknown>
  ) => Promise<TrackData>;
  writeTracks: (
    updates: Array<{ path: string; fields: Record<string, unknown> }>
  ) => Promise<TrackData[]>;
  readExtraTags: (trackPath: string) => Promise<ExtraTag[]>;
  writeExtraTags: (
    trackPath: string,
    tags: ExtraTagUpdate[]
  ) => Promise<TrackData>;
  writeExtraTagsBatch: (
    updates: Array<{ path: string; tags: ExtraTagUpdate[] }>
  ) => Promise<TrackData[]>;
  renameTrack: (oldPath: string, newPath: string) => Promise<TrackData>;
  checkFileExists: (filePath: string) => Promise<boolean>;

  showTrackContextMenu: (
    trackPath: string,
    labels: Record<string, string>
  ) => Promise<"extra-tags" | "delete-files" | null>;

  deleteFiles: (filePaths: string[]) => Promise<
    { path: string; success: boolean; error?: string }[]
  >;

  // Directory browser
  listDirectory: (dirPath: string) => Promise<DirEntry[]>;
  readDirectory: (dirPath: string) => Promise<DirectoryData>;

  // Auto-tag
  autoTagAlbum: (albumPath: string) => Promise<string>;
  downloadAlbumLyrics: (albumPath: string) => Promise<number>;
  onAutoTagEvent: (callback: (event: AutoTagEvent) => void) => () => void;
  getTaskProgress: (taskId: string) => Promise<TaskProgress>;
  cancelTask: (taskId: string) => Promise<void>;
  getDatasetStatus: () => Promise<DatasetStatus>;

  // Audit
  runAudit: (libraryPath: string) => Promise<{ albums: number; issues: number }>;
  runAuditOnTracks: (trackPaths: string[]) => Promise<{ albums: number; issues: number }>;
  runAuditOnAlbums: (albumPaths: string[]) => Promise<{ albums: number; issues: number }>;
  runAlbumAudit: (albumPath: string) => Promise<AuditTrackResult[]>;
  onAuditEvent: (callback: (event: AuditEvent) => void) => () => void;
  cancelAudit: () => Promise<void>;

  // Cover
  getCoverDataUrl: (albumPath: string) => Promise<string | null>;
  setCover: (albumPath: string) => Promise<string | null>;
  removeCover: (albumPath: string) => Promise<boolean>;
  downloadCoverArt: (albumPath: string) => Promise<string | null>;
  downloadArtistArt: (albumPath: string) => Promise<{ path: string; source: string } | null>;

  // Lyrics
  fetchLyrics: (
    trackName: string,
    artistName: string,
    albumName?: string,
    duration?: number,
  ) => Promise<string | null>;

  // Config
  getConfig: () => Promise<Record<string, unknown>>;
  setConfig: (key: string, value: unknown) => Promise<void>;

  // Debug
  subscribeDebugLogs: () => Promise<void>;
  setDebugMode: (enabled: boolean) => Promise<void>;

  onAssistantEvent: (callback: (event: AssistantEvent) => void) => () => void;

  // Assistant
  assistantSend: (input: {
    message: string;
    apiKey: string;
    model?: string;
    libraryPath?: string | null;
    activeAlbumPath?: string | null;
    selectedTrackPaths?: string[];
    tracks?: TrackData[];
    albums?: AlbumInfo[];
    autonomous?: boolean;
  }) => Promise<AssistantEvent>;
  assistantCancel: () => Promise<void>;
  assistantClear: () => Promise<void>;
  assistantApplyActions: (actionBatchId: string) => Promise<{
    success: boolean;
    error?: string;
    task?: "auto_tag" | "audit";
    trackPaths?: string[];
    results?: unknown;
    undoSnapshots?: TrackUndoSnapshot[];
    extraUndoSnapshots?: ExtraTagUndoSnapshot[];
  }>;
  assistantRejectActions: (actionBatchId: string) => Promise<void>;
  assistantGetBatches: () => Promise<AssistantActionBatch[]>;
  assistantInitRuntime: () => Promise<void>;
  assistantInitServices: (config: {
    apiKey: string;
    model?: string;
    discogsToken?: string | null;
    lyricsHost?: string | null;
    libraryPath?: string | null;
  }) => Promise<void>;

  // Window events
  onFocus: () => Promise<void>;

  // Organizer
  sortByAlbum: (
    sourceDir: string,
    options?: { copy?: boolean }
  ) => Promise<SortByAlbumResult>;

  // Conversation logs
  listSessions: (limit?: number) => Promise<SessionSummary[]>;
  getConversation: (sessionUuidOrNumber: string) => Promise<ConversationEntry[]>;
  getSession: (sessionUuidOrNumber: string) => Promise<SessionSummary | null>;
  getCurrentSession: () => Promise<{ sessionId: string; sessionNumber: string } | null>;
}

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
