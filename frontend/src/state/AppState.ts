import type { TrackData, AlbumInfo, AuditTrackResult } from "../../electron/preload";
import { UndoManager, type TrackSnapshot } from "./UndoManager";

export interface AuditResultEntry {
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
}

export interface TrackAuditSummary {
  count: number;
  highestStatus: "correct" | "warning" | "error";
  results: AuditResultEntry[];
  hasManualReview: boolean;
  autoFixedCount: number;
}

export interface AuditApplyAlbumResult {
  albumPath: string;
  results: AuditTrackResult[];
}

function statusRank(status: AuditResultEntry["status"]): number {
  if (status === "error") return 2;
  if (status === "warning") return 1;
  return 0;
}

function parentPath(filePath: string): string {
  const slash = Math.max(filePath.lastIndexOf("/"), filePath.lastIndexOf("\\"));
  return slash >= 0 ? filePath.slice(0, slash) : "";
}

export function buildAuditByTrackPath({
  auditResults,
  tracks,
}: {
  auditResults: Record<string, AuditResultEntry[]>;
  tracks: Array<Pick<TrackData, "path">>;
}): Record<string, TrackAuditSummary> {
  const byPath: Record<string, TrackAuditSummary> = {};
  const tracksByAlbum = new Map<string, Array<Pick<TrackData, "path">>>();

  for (const track of tracks) {
    const albumPath = parentPath(track.path);
    const albumTracks = tracksByAlbum.get(albumPath) ?? [];
    albumTracks.push(track);
    tracksByAlbum.set(albumPath, albumTracks);
  }

  for (const [albumPath, results] of Object.entries(auditResults)) {
    const albumTracks = tracksByAlbum.get(albumPath);
    if (!albumTracks) continue;

    for (const result of results) {
      const track = albumTracks[result.trackIndex];
      if (!track) continue;

      const existing = byPath[track.path] ?? {
        count: 0,
        highestStatus: "correct" as const,
        results: [],
        hasManualReview: false,
        autoFixedCount: 0,
      };
      const isManualReview = (result.status === "error" || result.status === "warning") && !result.autoFixed;
      const highestStatus =
        isManualReview && statusRank(result.status) > statusRank(existing.highestStatus)
          ? result.status
          : existing.highestStatus;

      byPath[track.path] = {
        count: existing.count + 1,
        highestStatus,
        results: [...existing.results, result],
        hasManualReview: existing.hasManualReview || isManualReview,
        autoFixedCount: existing.autoFixedCount + (result.autoFixed ? 1 : 0),
      };
    }
  }

  return byPath;
}

function toAuditTrackResult(result: AuditResultEntry): AuditTrackResult {
  return {
    index: result.trackIndex,
    field: result.field,
    status: result.status,
    message: result.message ?? "",
    suggestion: result.suggestion,
    source: result.source,
    confidence: result.confidence,
    autoFixEligible: result.autoFixEligible,
    autoFixed: result.autoFixed,
    corrected: result.corrected,
  };
}

export function buildAuditApplyAlbumResults({
  auditResults,
  tracks,
  albumPath,
  trackPath,
}: {
  auditResults: Record<string, AuditResultEntry[]>;
  tracks: Array<Pick<TrackData, "path">>;
  albumPath?: string | null;
  trackPath?: string | null;
}): AuditApplyAlbumResult[] {
  if (trackPath) {
    const selectedAlbumPath = parentPath(trackPath);
    const trackIndex = tracks
      .filter((track) => parentPath(track.path) === selectedAlbumPath)
      .findIndex((track) => track.path === trackPath);
    if (trackIndex < 0) return [];

    const results = (auditResults[selectedAlbumPath] ?? [])
      .filter((result) => result.trackIndex === trackIndex)
      .map(toAuditTrackResult);
    return results.length > 0 ? [{ albumPath: selectedAlbumPath, results }] : [];
  }

  if (albumPath) {
    const results = (auditResults[albumPath] ?? []).map(toAuditTrackResult);
    return results.length > 0 ? [{ albumPath, results }] : [];
  }

  return [];
}

export function getVisibleAuditResult(
  auditResults: Record<string, AuditResultEntry[]>,
  activeAlbumPath: string | null,
): { albumPath: string; results: AuditResultEntry[] } | null {
  if (activeAlbumPath && auditResults[activeAlbumPath]) {
    return { albumPath: activeAlbumPath, results: auditResults[activeAlbumPath] };
  }

  const entries = Object.entries(auditResults);
  if (entries.length !== 1) return null;

  const [albumPath, results] = entries[0];
  return { albumPath, results };
}

export interface AppState {
  /** Root library path being browsed */
  libraryPath: string | null;

  /** All discovered albums in the library */
  albums: AlbumInfo[];

  /** All audio tracks across all albums (flattened) */
  tracks: TrackData[];

  /** Filter to a specific album path (null = show all) */
  activeAlbumPath: string | null;

  /** Currently selected file paths (multi-select) */
  selectedTrackPaths: string[];

  /** Primary selected file path for single-track edits */
  selectedTrackPath: string | null;

  /** Metadata for the primary selected file */
  selectedTrack: TrackData | null;

  /** Cover art data URL for the selected file's directory */
  coverDataUrl: string | null;

  /** Filter text for the file grid */
  filterText: string;

  /** Loading states */
  scanning: boolean;
  scanningProgress: { current: number; total: number } | null;
  loaded: boolean;

  /** Error message */
  error: string | null;

  /** Undo manager instance */
  undoManager: UndoManager;

  /** Currently saving flag */
  saving: boolean;

  /** Settings modal visibility */
  showSettings: boolean;

  /** Auto-tag in progress */
  autoTagging: boolean;
  /** Lyrics download in progress */
  lyricsGetting: boolean;
  autoTagProgress: {
    current: number;
    total: number;
    message: string;
  } | null;

  /** Audit in progress */
  auditing: boolean;
  auditProgress: {
    current: number;
    total: number;
    message: string;
  } | null;
  /** Audit results keyed by album path */
  auditResults: Record<string, AuditResultEntry[]>;

  /** Dark mode enabled */
  darkMode: boolean;
}

export const initialAppState: AppState = {
  libraryPath: null,
  albums: [],
  tracks: [],
  activeAlbumPath: null,
  selectedTrackPaths: [],
  selectedTrackPath: null,
  selectedTrack: null,
  coverDataUrl: null,
  filterText: "",
  scanning: false,
  scanningProgress: null,
  loaded: false,
  error: null,
  undoManager: new UndoManager(),
  saving: false,
  showSettings: false,
  autoTagging: false,
  lyricsGetting: false,
  autoTagProgress: null,
  auditing: false,
  auditProgress: null,
  auditResults: {},
  darkMode: false,
};

export type AppAction =
  | { type: "SET_LIBRARY"; path: string }
  | { type: "SET_ALBUMS"; albums: AlbumInfo[] }
  | { type: "SET_TRACKS"; tracks: TrackData[] }
  | { type: "SET_ACTIVE_ALBUM"; path: string | null }
  | { type: "SELECT_TRACK"; path: string; track: TrackData }
  | { type: "SET_SELECTED_TRACKS"; paths: string[] }
  | { type: "CLEAR_SELECTION" }
  | { type: "SET_COVER_URL"; url: string | null }
  | { type: "SET_FILTER"; filter: string }
  | { type: "SET_SCANNING"; scanning: boolean }
  | { type: "SET_SCANNING_PROGRESS"; progress: { current: number; total: number } | null }
  | { type: "SET_LOADED"; loaded: boolean }
  | { type: "SET_ERROR"; error: string | null }
  | { type: "UPDATE_TRACK"; path: string; track: TrackData }
  | { type: "UPDATE_TRACKS"; tracks: TrackData[] }
  | { type: "PUSH_UNDO"; description: string; snapshots: TrackSnapshot[] }
  | { type: "POP_UNDO" }
  | { type: "CLEAR_UNDO" }
  | { type: "SET_SAVING"; saving: boolean }
  | { type: "TOGGLE_SETTINGS"; show: boolean }
  | { type: "SET_AUTO_TAGGING"; autoTagging: boolean }
  | {
      type: "SET_AUTO_TAG_PROGRESS";
      progress: {
        current: number;
        total: number;
        message: string;
      } | null;
    }
  | { type: "SET_LYRICS_GETTING"; lyricsGetting: boolean }
  | { type: "SET_AUDITING"; auditing: boolean }
  | {
      type: "SET_AUDIT_PROGRESS";
      progress: {
        current: number;
        total: number;
        message: string;
      } | null;
    }
  | {
      type: "ADD_AUDIT_RESULTS";
      albumPath: string;
      results: AuditResultEntry[];
    }
  | { type: "CLEAR_AUDIT_RESULTS" }
  | { type: "TOGGLE_DARK_MODE" }
  | { type: "CLEAR_ALL" };

export function appReducer(state: AppState, action: AppAction): AppState {
  switch (action.type) {
    case "SET_LIBRARY":
      return {
        ...state,
        libraryPath: action.path,
        activeAlbumPath: null,
        selectedTrackPaths: [],
        selectedTrackPath: null,
        selectedTrack: null,
        coverDataUrl: null,
        error: null,
      };

    case "SET_ALBUMS":
      return {
        ...state,
        albums: action.albums,
      };

    case "SET_TRACKS": {
      const selectedTrack = state.selectedTrackPath
        ? action.tracks.find((track) => track.path === state.selectedTrackPath) ?? state.selectedTrack
        : null;
      return {
        ...state,
        tracks: action.tracks,
        selectedTrack,
        loaded: true,
      };
    }

    case "SET_ACTIVE_ALBUM": {
      const isInScope = action.path === null || (
        state.selectedTrackPath != null &&
        state.selectedTrackPath.startsWith(action.path + "/")
      );
      return {
        ...state,
        activeAlbumPath: action.path,
        // Only clear selection if the selected track is outside the new scope
        selectedTrackPaths: isInScope ? state.selectedTrackPaths : [],
        selectedTrackPath: isInScope ? state.selectedTrackPath : null,
        selectedTrack: isInScope ? state.selectedTrack : null,
        coverDataUrl: isInScope ? state.coverDataUrl : null,
      };
    }

    case "SELECT_TRACK":
      return {
        ...state,
        selectedTrackPaths: [action.path],
        selectedTrackPath: action.path,
        selectedTrack: action.track,
      };

    case "SET_SELECTED_TRACKS": {
      const selectedTrackPath = state.selectedTrackPath != null &&
        action.paths.includes(state.selectedTrackPath)
        ? state.selectedTrackPath
        : action.paths[0] ?? null;
      const primaryChanged = selectedTrackPath !== state.selectedTrackPath;

      return {
        ...state,
        selectedTrackPaths: action.paths,
        selectedTrackPath,
        selectedTrack: primaryChanged
          ? state.tracks.find((track) => track.path === selectedTrackPath) ?? null
          : state.selectedTrack,
        coverDataUrl: primaryChanged ? null : state.coverDataUrl,
      };
    }

    case "CLEAR_SELECTION":
      return {
        ...state,
        selectedTrackPaths: [],
        selectedTrackPath: null,
        selectedTrack: null,
        coverDataUrl: null,
      };

    case "SET_COVER_URL":
      return { ...state, coverDataUrl: action.url };

    case "SET_FILTER":
      return { ...state, filterText: action.filter };

    case "SET_SCANNING":
      return { ...state, scanning: action.scanning, scanningProgress: action.scanning ? state.scanningProgress : null };

    case "SET_SCANNING_PROGRESS":
      return { ...state, scanningProgress: action.progress };

    case "SET_LOADED":
      return { ...state, loaded: action.loaded };

    case "CLEAR_ALL":
      return { ...initialAppState };

    case "SET_ERROR":
      return { ...state, error: action.error };

    case "UPDATE_TRACK":
      return {
        ...state,
        tracks: state.tracks.map((t) =>
          t.path === action.path ? { ...action.track } : t
        ),
        selectedTrack:
          state.selectedTrackPath === action.path
            ? { ...action.track }
            : state.selectedTrack,
      };

    case "UPDATE_TRACKS": {
      const updated = new Map(action.tracks.map((t) => [t.path, t]));
      return {
        ...state,
        tracks: state.tracks.map((t) => updated.get(t.path) ?? t),
        selectedTrack:
          state.selectedTrackPath && updated.has(state.selectedTrackPath)
            ? updated.get(state.selectedTrackPath)!
            : state.selectedTrack,
      };
    }

    case "PUSH_UNDO":
      return {
        ...state,
        undoManager: state.undoManager.cloneAndPush(
          action.description,
          action.snapshots,
        ),
      };

    case "POP_UNDO":
      // handleRevert does the actual pop; this just triggers a re-render
      return { ...state };

    case "CLEAR_UNDO":
      state.undoManager.clear();
      return { ...state };

    case "SET_SAVING":
      return { ...state, saving: action.saving };

    case "TOGGLE_SETTINGS":
      return { ...state, showSettings: action.show };

    case "SET_AUTO_TAGGING":
      return {
        ...state,
        autoTagging: action.autoTagging,
        autoTagProgress: action.autoTagging ? state.autoTagProgress : null,
      };

    case "SET_AUTO_TAG_PROGRESS":
      return { ...state, autoTagProgress: action.progress };

    case "SET_LYRICS_GETTING":
      return { ...state, lyricsGetting: action.lyricsGetting };

    case "SET_AUDITING":
      return {
        ...state,
        auditing: action.auditing,
        auditProgress: action.auditing ? state.auditProgress : null,
      };

    case "SET_AUDIT_PROGRESS":
      return { ...state, auditProgress: action.progress };

    case "ADD_AUDIT_RESULTS":
      return {
        ...state,
        auditResults: {
          ...state.auditResults,
          [action.albumPath]: action.results,
        },
      };

    case "CLEAR_AUDIT_RESULTS":
      return { ...state, auditResults: {} };

    case "TOGGLE_DARK_MODE":
      return { ...state, darkMode: !state.darkMode };

    default:
      return state;
  }
}
