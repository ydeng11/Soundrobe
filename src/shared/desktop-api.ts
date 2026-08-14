/**
 * Renderer-neutral desktop API contract.
 *
 * The Tauri adapter and renderer tests depend on this single stable
 * `window.api` surface. Method names, signatures, task IDs, payloads, event
 * shapes, error semantics, and config-redaction behavior are contractual.
 */

export interface AlbumInfo {
  path: string;
  name: string;
  artistHint: string;
  albumHint: string;
  trackCount: number;
}

export interface AppInfo {
  identifier: string;
  version: string;
  runtime: "tauri";
  dev: boolean;
}

export interface CoverInfo {
  path: string | null;
  source: "external" | "embedded" | "missing";
  dataUrl: string | null;
}

export interface LyricsDocument {
  syncedLyrics: string | null;
  plainLyrics: string;
  language: string;
}

export type LyricsSource = "embedded" | "lrc" | "txt" | "remote";
export type LyricsTrackStatus =
  | "embeddedPreserved"
  | "written"
  | "noLyrics"
  | "unsupported"
  | "failed";

export interface LyricsBatchReport {
  total: number;
  written: number;
  embeddedPreserved: number;
  noLyrics: number;
  unsupported: number;
  failed: number;
  results: Array<{
    path: string;
    status: LyricsTrackStatus;
    source: LyricsSource | null;
    error: string | null;
  }>;
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
  lyrics: LyricsDocument | null;
  compilation: boolean | null;
  musicbrainzTrackId: string | null;
  musicbrainzAlbumId: string | null;
  musicbrainzArtistId: string | null;
  discogsArtistId: string | null;
  discogsReleaseId: string | null;
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
    source?: "deterministic" | "llm";
    confidence?: number;
    autoFixEligible?: boolean;
    autoFixed?: boolean;
    corrected?: AuditTrackResult["corrected"];
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

export interface WriteProbePhase {
  name: string;
  success: boolean;
  error: string | null;
  osErrorCode: number | null;
}

export interface WriteProbeResult {
  path: string;
  phases: WriteProbePhase[];
  allSuccessful: boolean;
}

export interface RealWriteProbeResult {
  path: string;
  outcome: "Skipped" | "Replaced" | "Error";
  error: string | null;
  osErrorCode: number | null;
  beforeField: string | null;
  afterField: string | null;
  copyRemoved: boolean;
}

export interface AuditTrackResult {
  index: number;
  field: string;
  status: "correct" | "warning" | "error";
  message: string;
  suggestion?: string | null;
  source?: "deterministic" | "llm";
  confidence?: number;
  autoFixEligible?: boolean;
  autoFixed?: boolean;
  corrected?: {
    title?: string | null;
    artist?: string | null;
    artists?: string[] | null;
    album?: string | null;
    albumArtist?: string | null;
    albumArtists?: string[] | null;
    year?: string | null;
    genre?: string | null;
    trackNumber?: number | null;
    trackTotal?: number | null;
    discNumber?: number | null;
    discTotal?: number | null;
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

export interface AuditRunSummary {
  albums: number;
  issues: number;
  albumResults?: Array<{
    albumPath: string;
    results: AuditTrackResult[];
  }>;
}

export interface AuditApplyFixesSummary {
  fixed: number;
  albumResults: Array<{
    albumPath: string;
    results: AuditTrackResult[];
  }>;
}

export interface TrackWriteFailure {
  path: string;
  error: string;
}

export interface BatchWriteResult {
  tracks: TrackData[];
  failures: TrackWriteFailure[];
}

// ── Manual search types ────────────────────────────────────────────

export interface ReleaseSearchResult {
  provider: "musicbrainz" | "discogs";
  id: string;
  kind?: string;
  title: string;
  artist?: string;
  year?: string;
  country?: string;
  formats: string[];
  catalogNumber?: string;
  barcode?: string;
}

export interface ReleaseSearchPage {
  results: ReleaseSearchResult[];
  page: number;
  pageSize: number;
  total?: number;
  hasNext: boolean;
}

export interface TrackMappingRow {
  localIndex: number;
  localTitle?: string;
  localArtist?: string;
  remoteIndex?: number;
  remoteTitle?: string;
  remoteArtist?: string;
  remoteTrackNumber?: number;
  remoteTrackTotal?: number;
  evidence?: string;
}

export interface PreviewMatchResult {
  release: ProviderAlbum;
  candidates: TrackMappingRow[];
  unusedRemoteIndices: number[];
  albumCandidate: AlbumCandidate;
}

export interface TrackEdit {
  title?: string;
  match_titles?: string[];
  artist?: string;
  artists: string[];
  track_number?: number;
  track_total?: number;
  disc_number?: number;
  disc_total?: number;
  musicbrainz_trackid?: string;
  length?: number;
  genre?: string;
  filename?: string;
}

export interface ProviderTrack {
  title?: string;
  matchTitles: string[];
  artist?: string;
  artists: string[];
  trackNumber?: number;
  trackTotal?: number;
  discNumber?: number;
  recordingId?: string;
  length?: number;
}

export interface ProviderAlbum {
  id: string;
  title: string;
  artist?: string;
  artists: string[];
  artistId?: string;
  year?: string;
  genre?: string;
  tracks: ProviderTrack[];
}

export interface AlbumCandidate {
  artist?: string;
  artists: string[];
  album?: string;
  albumArtist?: string;
  albumArtists: string[];
  year?: string;
  genre?: string;
  musicbrainzAlbumId?: string;
  musicbrainzArtistId?: string;
  discogsReleaseId?: string;
  discogsArtistId?: string;
  tracks: TrackEdit[];
  source?: string;
  distance?: number;
  verification?: string;
}

export interface TrackWriteEvent {
  current: number;
  total: number;
  message: string;
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
  available: boolean;
  musicbrainz: boolean;
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
  dependsOnBatchIds?: string[];
  completionContract?: {
    scopePaths: string[];
    scopeSnapshot: Array<{
      path: string;
      standardValues: Record<string, unknown>;
      extraValues: Record<string, unknown>;
    }>;
    expectedActionPaths: string[];
    expectedActions: Array<{
      trackPath: string;
      tagKind: string;
      field: string;
      operation: string;
      expectedValue: unknown;
    }>;
    postcondition: "exactMetadataActions" | "splitArtistsNormalized";
  };
}

export interface AssistantVerification {
  status: "verified" | "failed";
  phase: "preflight" | "write" | "readback" | "persistence";
  scopeCount: number;
  expectedActionCount: number;
  verifiedActionCount: number;
  failures: Array<{
    trackPath?: string;
    field?: string;
    error: string;
    expected?: unknown;
    actual?: unknown;
  }>;
}

export interface AssistantEvent {
  sessionId: string;
  type:
    | "step"
    | "tool_running"
    | "tool_result"
    | "action_batch_created"
    | "action_batch_progress"
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

export interface DesktopAPI {
  // App
  appInfo: () => Promise<AppInfo>;

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
  ) => Promise<BatchWriteResult>;
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
  probeWriteVolume: (filePath: string) => Promise<WriteProbeResult>;
  probeWriteVolumeReal: (
    filePath: string,
    patch: Record<string, unknown>
  ) => Promise<RealWriteProbeResult>;

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
  downloadAlbumLyrics: (albumPath: string) => Promise<LyricsBatchReport>;
  onAutoTagEvent: (callback: (event: AutoTagEvent) => void) => () => void;
  onTrackWriteEvent: (callback: (event: TrackWriteEvent) => void) => () => void;
  getTaskProgress: (taskId: string) => Promise<TaskProgress | null>;
  cancelTask: (taskId: string) => Promise<void>;
  getDatasetStatus: () => Promise<DatasetStatus>;

  // Audit
  runAudit: (libraryPath: string) => Promise<AuditRunSummary>;
  runAuditOnTracks: (trackPaths: string[]) => Promise<AuditRunSummary>;
  runAuditOnAlbums: (albumPaths: string[]) => Promise<AuditRunSummary>;
  runAlbumAudit: (albumPath: string) => Promise<AuditTrackResult[]>;
  applyAuditFixes: (
    albumResults: NonNullable<AuditRunSummary["albumResults"]>
  ) => Promise<AuditApplyFixesSummary>;
  onAuditEvent: (callback: (event: AuditEvent) => void) => () => void;
  cancelAudit: () => Promise<void>;

  // Cover
  getCoverDataUrl: (albumPath: string, preferredTrackPath?: string) => Promise<string | null>;
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
  ) => Promise<LyricsDocument | null>;

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
    verification?: AssistantVerification;
  }>;
  assistantCompleteTaskActions: (
    actionBatchId: string,
    error?: string | null,
  ) => Promise<{ success: boolean; error?: string | null }>;
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
  testLlmConnection: (
    apiKey: string,
    model: string,
    provider?: string,
    baseUrl?: string,
  ) => Promise<{ model: string }>;

  // Manual search
  searchReleases: (request: {
    provider: "musicbrainz" | "discogs";
    artist?: string;
    album?: string;
    year?: string;
    country?: string;
    format?: string;
    catalogNumber?: string;
    barcode?: string;
    page?: number;
    pageSize?: number;
  }) => Promise<ReleaseSearchPage>;

  resolveRelease: (
    provider: "musicbrainz" | "discogs",
    releaseId: string,
    kind?: string
  ) => Promise<ProviderAlbum>;

  previewReleaseMatch: (request: {
    albumPath: string;
    release: ProviderAlbum;
    provider: string;
  }) => Promise<PreviewMatchResult>;

  searchApplyCandidate: (
    albumPath: string,
    candidate: AlbumCandidate,
    selectedTrackIndices: number[]
  ) => Promise<number>;

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
