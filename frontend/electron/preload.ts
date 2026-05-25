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

  // Directory browser
  listDirectory: (dirPath: string) => Promise<DirEntry[]>;
  readDirectory: (dirPath: string) => Promise<DirectoryData>;

  // Auto-tag
  autoTagAlbum: (albumPath: string) => Promise<string>;
  getTaskProgress: (taskId: string) => Promise<TaskProgress>;
  cancelTask: (taskId: string) => Promise<void>;
  getDatasetStatus: () => Promise<DatasetStatus>;

  // Cover
  getCoverDataUrl: (albumPath: string) => Promise<string | null>;
  setCover: (albumPath: string) => Promise<string | null>;
  removeCover: (albumPath: string) => Promise<boolean>;

  // Config
  getConfig: () => Promise<Record<string, unknown>>;
  setConfig: (key: string, value: unknown) => Promise<void>;

  // Window events
  onFocus: () => Promise<void>;
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

  // Cover
  getCoverDataUrl: (albumPath: string) =>
    ipcRenderer.invoke("cover:data-url", albumPath),
  setCover: (albumPath: string): Promise<string | null> =>
    ipcRenderer.invoke("cover:set", albumPath),
  removeCover: (albumPath: string): Promise<boolean> =>
    ipcRenderer.invoke("cover:remove", albumPath),

  // Directory browser
  listDirectory: (dirPath: string) =>
    ipcRenderer.invoke("directory:list", dirPath),
  readDirectory: (dirPath: string) =>
    ipcRenderer.invoke("directory:read", dirPath),

  // Config
  getConfig: () => ipcRenderer.invoke("config:get"),
  setConfig: (key: string, value: unknown) =>
    ipcRenderer.invoke("config:set", key, value),

  // Auto-tag
  autoTagAlbum: (albumPath: string) =>
    ipcRenderer.invoke("album:auto-tag", albumPath),
  getTaskProgress: (taskId: string) =>
    ipcRenderer.invoke("task:progress", taskId),
  cancelTask: (taskId: string) =>
    ipcRenderer.invoke("task:cancel", taskId),
  getDatasetStatus: () => ipcRenderer.invoke("dataset:status"),

  // Window events
  onFocus: () => ipcRenderer.invoke("window:focused"),
});
