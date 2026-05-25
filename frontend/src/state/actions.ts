import type { TrackData, AlbumInfo } from "../../electron/preload";
import type { AppAction } from "./AppState";
import type { TrackSnapshot } from "./UndoManager";

export const actionCreators = {
  setLibrary: (path: string): AppAction => ({
    type: "SET_LIBRARY",
    path,
  }),

  setAlbums: (albums: AlbumInfo[]): AppAction => ({
    type: "SET_ALBUMS",
    albums,
  }),

  setTracks: (tracks: TrackData[]): AppAction => ({
    type: "SET_TRACKS",
    tracks,
  }),

  setActiveAlbum: (path: string | null): AppAction => ({
    type: "SET_ACTIVE_ALBUM",
    path,
  }),

  selectTrack: (path: string, track: TrackData): AppAction => ({
    type: "SELECT_TRACK",
    path,
    track,
  }),

  clearSelection: (): AppAction => ({
    type: "CLEAR_SELECTION",
  }),

  setCoverUrl: (url: string | null): AppAction => ({
    type: "SET_COVER_URL",
    url,
  }),

  setFilter: (filter: string): AppAction => ({
    type: "SET_FILTER",
    filter,
  }),

  setScanning: (scanning: boolean): AppAction => ({
    type: "SET_SCANNING",
    scanning,
  }),

  setLoaded: (loaded: boolean): AppAction => ({
    type: "SET_LOADED",
    loaded,
  }),

  setError: (error: string | null): AppAction => ({
    type: "SET_ERROR",
    error,
  }),

  updateTrack: (path: string, track: TrackData): AppAction => ({
    type: "UPDATE_TRACK",
    path,
    track,
  }),

  setDirty: (paths: string[]): AppAction => ({
    type: "SET_DIRTY",
    paths,
  }),

  clearDirty: (path: string): AppAction => ({
    type: "CLEAR_DIRTY",
    path,
  }),

  pushUndo: (
    description: string,
    snapshots: TrackSnapshot[]
  ): AppAction => ({
    type: "PUSH_UNDO",
    description,
    snapshots,
  }),

  popUndo: (): AppAction => ({
    type: "POP_UNDO",
    snapshots: null,
  }),

  clearUndo: (): AppAction => ({
    type: "CLEAR_UNDO",
  }),

  setSaving: (saving: boolean): AppAction => ({
    type: "SET_SAVING",
    saving,
  }),

  clearAll: (): AppAction => ({
    type: "CLEAR_ALL",
  }),
};
