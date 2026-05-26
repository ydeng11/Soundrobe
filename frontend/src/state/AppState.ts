import type { TrackData, AlbumInfo } from "../../electron/preload";
import { UndoManager, type TrackSnapshot } from "./UndoManager";

export interface AppState {
  /** Root library path being browsed */
  libraryPath: string | null;

  /** All discovered albums in the library */
  albums: AlbumInfo[];

  /** All audio tracks across all albums (flattened) */
  tracks: TrackData[];

  /** Filter to a specific album path (null = show all) */
  activeAlbumPath: string | null;

  /** Currently selected file path */
  selectedTrackPath: string | null;

  /** Metadata for the selected file */
  selectedTrack: TrackData | null;

  /** Cover art data URL for the selected file's directory */
  coverDataUrl: string | null;

  /** Filter text for the file grid */
  filterText: string;

  /** Loading states */
  scanning: boolean;
  loaded: boolean;

  /** Error message */
  error: string | null;

  /** Undo manager instance */
  undoManager: UndoManager;

  /** Track paths with unsaved changes */
  dirtyTracks: Set<string>;

  /** Currently saving flag */
  saving: boolean;

  /** Settings modal visibility */
  showSettings: boolean;
}

export const initialAppState: AppState = {
  libraryPath: null,
  albums: [],
  tracks: [],
  activeAlbumPath: null,
  selectedTrackPath: null,
  selectedTrack: null,
  coverDataUrl: null,
  filterText: "",
  scanning: false,
  loaded: false,
  error: null,
  undoManager: new UndoManager(),
  dirtyTracks: new Set(),
  saving: false,
  showSettings: false,
};

export type AppAction =
  | { type: "SET_LIBRARY"; path: string }
  | { type: "SET_ALBUMS"; albums: AlbumInfo[] }
  | { type: "SET_TRACKS"; tracks: TrackData[] }
  | { type: "SET_ACTIVE_ALBUM"; path: string | null }
  | { type: "SELECT_TRACK"; path: string; track: TrackData }
  | { type: "CLEAR_SELECTION" }
  | { type: "SET_COVER_URL"; url: string | null }
  | { type: "SET_FILTER"; filter: string }
  | { type: "SET_SCANNING"; scanning: boolean }
  | { type: "SET_LOADED"; loaded: boolean }
  | { type: "SET_ERROR"; error: string | null }
  | { type: "UPDATE_TRACK"; path: string; track: TrackData }
  | { type: "SET_DIRTY"; paths: string[] }
  | { type: "CLEAR_DIRTY"; path: string }
  | { type: "PUSH_UNDO"; description: string; snapshots: TrackSnapshot[] }
  | { type: "POP_UNDO"; snapshots: TrackSnapshot[] | null }
  | { type: "CLEAR_UNDO" }
  | { type: "SET_SAVING"; saving: boolean }
  | { type: "TOGGLE_SETTINGS"; show: boolean }
  | { type: "CLEAR_ALL" };

export function appReducer(state: AppState, action: AppAction): AppState {
  switch (action.type) {
    case "SET_LIBRARY":
      return {
        ...state,
        libraryPath: action.path,
        activeAlbumPath: null,
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

    case "SET_TRACKS":
      return {
        ...state,
        tracks: action.tracks,
        loaded: true,
      };

    case "SET_ACTIVE_ALBUM":
      return {
        ...state,
        activeAlbumPath: action.path,
        selectedTrackPath: null,
        selectedTrack: null,
        coverDataUrl: null,
      };

    case "SELECT_TRACK":
      return {
        ...state,
        selectedTrackPath: action.path,
        selectedTrack: action.track,
      };

    case "CLEAR_SELECTION":
      return {
        ...state,
        selectedTrackPath: null,
        selectedTrack: null,
        coverDataUrl: null,
      };

    case "SET_COVER_URL":
      return { ...state, coverDataUrl: action.url };

    case "SET_FILTER":
      return { ...state, filterText: action.filter };

    case "SET_SCANNING":
      return { ...state, scanning: action.scanning };

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
        dirtyTracks: new Set(state.dirtyTracks).add(action.path),
      };

    case "SET_DIRTY":
      return {
        ...state,
        dirtyTracks: new Set([...state.dirtyTracks, ...action.paths]),
      };

    case "CLEAR_DIRTY": {
      const next = new Set(state.dirtyTracks);
      next.delete(action.path);
      return { ...state, dirtyTracks: next };
    }

    case "PUSH_UNDO":
      state.undoManager.push(action.description, action.snapshots);
      return { ...state };

    case "POP_UNDO":
      state.undoManager.pop();
      return { ...state };

    case "CLEAR_UNDO":
      state.undoManager.clear();
      return { ...state };

    case "SET_SAVING":
      return { ...state, saving: action.saving };

    case "TOGGLE_SETTINGS":
      return { ...state, showSettings: action.show };

    default:
      return state;
  }
}
