import React, { useReducer, useCallback, useEffect } from "react";
import { appReducer, initialAppState } from "./state/AppState";
import type { TrackSnapshot } from "./state/UndoManager";
import { TitleBar } from "./components/TitleBar";
import { FileGrid } from "./components/FileGrid";
import { MetadataEditor } from "./components/MetadataEditor";
import type { TrackData } from "../electron/preload";

export default function App() {
  const [state, dispatch] = useReducer(appReducer, initialAppState);

  // --- Library loading ---

  const handleOpenLibrary = useCallback(async () => {
    try {
      if (!window.api) {
        throw new Error("Electron preload bridge is unavailable");
      }

      const selectedPath = await window.api.openFolderDialog();
      if (!selectedPath) return;

      dispatch({ type: "SET_LIBRARY", path: selectedPath });
      dispatch({ type: "SET_SCANNING", scanning: true });
      dispatch({ type: "SET_ERROR", error: null });

      try {
        // Scan library and read all tracks from all albums (flattened)
        const albums = await window.api.scanLibrary(selectedPath);
        dispatch({ type: "SET_ALBUMS", albums });

        const allTracks: TrackData[] = [];
        for (const album of albums) {
          try {
            const detail = await window.api.readAlbum(album.path);
            allTracks.push(...detail.tracks);
          } catch {
            // Skip albums that fail to read
          }
        }
        dispatch({ type: "SET_TRACKS", tracks: allTracks });
      } catch (err: unknown) {
        const message =
          err instanceof Error ? err.message : "Failed to scan library";
        dispatch({ type: "SET_ERROR", error: message });
      } finally {
        dispatch({ type: "SET_SCANNING", scanning: false });
      }
    } catch (err: unknown) {
      const message =
        err instanceof Error ? err.message : "Failed to open folder dialog";
      dispatch({
        type: "SET_ERROR",
        error: `Failed to open library: ${message}`,
      });
    }
  }, []);

  // --- Track selection ---

  // --- Track selection ---

  const handleSelectTrack = useCallback(
    async (path: string, track: TrackData) => {
      dispatch({ type: "SELECT_TRACK", path, track });

      // Load cover art for the track's parent directory
      const dirPath = path.split("/").slice(0, -1).join("/");
      try {
        const url = await window.api.getCoverDataUrl(dirPath);
        dispatch({ type: "SET_COVER_URL", url });
      } catch {
        dispatch({ type: "SET_COVER_URL", url: null });
      }
    },
    []
  );

  // --- Field editing with auto-save + undo ---

  const handleFieldChange = useCallback(
    async (field: string, value: string) => {
      if (!state.selectedTrack) return;

      // Save snapshot before editing
      const prevTrack = state.selectedTrack;
      const snapshot: TrackSnapshot = {
        path: prevTrack.path,
        fields: {
          title: prevTrack.title,
          artist: prevTrack.artist,
          album: prevTrack.album,
          year: prevTrack.year,
          track: prevTrack.trackNumber != null ? String(prevTrack.trackNumber) : null,
          genre: prevTrack.genre,
          composer: prevTrack.composer,
        },
      };
      dispatch({
        type: "PUSH_UNDO",
        description: `Edit ${field}`,
        snapshots: [snapshot],
      });

      // Build the fields to write
      const writeFields: Record<string, unknown> = { [field]: value || null };

      // Optimistically update local state
      const updatedTrack = { ...prevTrack };
      switch (field) {
        case "title":
          updatedTrack.title = value || null;
          break;
        case "artist":
          updatedTrack.artist = value || null;
          break;
        case "album":
          updatedTrack.album = value || null;
          break;
        case "year":
          updatedTrack.year = value || null;
          break;
        case "track": {
          const parts = value.split("/");
          updatedTrack.trackNumber = parts[0] ? parseInt(parts[0], 10) || null : null;
          updatedTrack.trackTotal = parts[1] ? parseInt(parts[1], 10) || null : null;
          if (parts[0]) writeFields["trackNumber"] = updatedTrack.trackNumber;
          if (parts[1]) writeFields["trackTotal"] = updatedTrack.trackTotal;
          break;
        }
        case "genre":
          updatedTrack.genre = value || null;
          break;
        case "composer":
          updatedTrack.composer = value || null;
          break;
        case "comment":
          break; // Comments aren't in TrackData from music-metadata
      }

      dispatch({ type: "UPDATE_TRACK", path: prevTrack.path, track: updatedTrack });
      dispatch({ type: "SET_SAVING", saving: true });

      try {
        // Write to disk and get updated metadata back
        const result = await window.api.writeTrack(prevTrack.path, writeFields);
        dispatch({ type: "UPDATE_TRACK", path: prevTrack.path, track: result });

        // Refresh cover if album changed
        if (field === "album" || field === "title") {
          const dirPath = prevTrack.path.split("/").slice(0, -1).join("/");
          try {
            const url = await window.api.getCoverDataUrl(dirPath);
            dispatch({ type: "SET_COVER_URL", url });
          } catch {
            // ignore
          }
        }
      } catch (err: unknown) {
        // Rollback on error
        dispatch({ type: "UPDATE_TRACK", path: prevTrack.path, track: prevTrack });
        const message =
          err instanceof Error ? err.message : "Failed to save tag";
        dispatch({ type: "SET_ERROR", error: message });
      } finally {
        dispatch({ type: "SET_SAVING", saving: false });
      }
    },
    [state.selectedTrack]
  );

  // --- Cover actions ---

  const handleChangeCover = useCallback(async () => {
    if (!state.selectedTrack) return;
    const dirPath = state.selectedTrack.path.split("/").slice(0, -1).join("/");
    try {
      const url = await window.api.setCover(dirPath);
      if (url) {
        dispatch({ type: "SET_COVER_URL", url });
      }
    } catch {
      dispatch({ type: "SET_ERROR", error: "Failed to set cover art" });
    }
  }, [state.selectedTrack]);

  const handleRemoveCover = useCallback(async () => {
    if (!state.selectedTrack) return;
    const dirPath = state.selectedTrack.path.split("/").slice(0, -1).join("/");
    try {
      await window.api.removeCover(dirPath);
      dispatch({ type: "SET_COVER_URL", url: null });
    } catch {
      dispatch({ type: "SET_ERROR", error: "Failed to remove cover art" });
    }
  }, [state.selectedTrack]);

  // --- Save / Revert ---

  const handleSave = useCallback(async () => {
    if (state.dirtyTracks.size === 0) return;
    dispatch({ type: "SET_SAVING", saving: true });
    // Already auto-saved — just clear the dirty flag
    dispatch({ type: "CLEAR_UNDO" });
    state.dirtyTracks.clear();
    dispatch({ type: "SET_SAVING", saving: false });
  }, [state.dirtyTracks]);

  const handleRevert = useCallback(() => {
    const op = state.undoManager.pop();
    if (!op) return;
    // Restore each snapshot
    for (const snap of op.snapshots) {
      const writeFields: Record<string, unknown> = { ...snap.fields };
      window.api.writeTrack(snap.path, writeFields).then((track) => {
        dispatch({ type: "UPDATE_TRACK", path: snap.path, track });
      });
    }
  }, [state.undoManager]);

  // --- Stub handlers ---

  const handleConvert = useCallback(() => {
    dispatch({ type: "SET_ERROR", error: "Convert not yet implemented" });
  }, []);

  const handleAutonumber = useCallback(() => {
    dispatch({ type: "SET_ERROR", error: "Autonumber not yet implemented" });
  }, []);

  const handleRename = useCallback(() => {
    dispatch({ type: "SET_ERROR", error: "Rename not yet implemented" });
  }, []);

  // --- Filter ---

  const handleFilterChange = useCallback((text: string) => {
    dispatch({ type: "SET_FILTER", filter: text });
  }, []);

  // --- Keyboard shortcuts ---

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "o") {
        e.preventDefault();
        handleOpenLibrary();
      }
      if ((e.metaKey || e.ctrlKey) && e.key === "z" && e.shiftKey) {
        e.preventDefault();
        handleRevert();
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [handleOpenLibrary, handleRevert]);

  return (
    <div className="flex flex-col h-screen bg-surface text-text-primary">
      {/* Top bar — library, filter, actions, stats */}
      <TitleBar
        libraryPath={state.libraryPath}
        trackCount={state.tracks.length}
        filterText={state.filterText}
        onFilterChange={handleFilterChange}
        selectedFilePath={state.selectedTrackPath}
        dirtyCount={state.dirtyTracks.size}
        canUndo={state.undoManager.canUndo}
        saving={state.saving}
        error={state.error}
        onOpenLibrary={handleOpenLibrary}
        onSave={handleSave}
        onRevert={handleRevert}
        onConvert={handleConvert}
        onAutonumber={handleAutonumber}
        onRename={handleRename}
      />

      {/* Two-pane layout */}
      <div className="flex flex-1 overflow-hidden">
        {/* Left: File grid */}
        <div className="flex-1 flex flex-col min-w-0 border-r border-gray-700/30">
          <FileGrid
            tracks={state.tracks}
            selectedTrackPath={state.selectedTrackPath}
            filterText={state.filterText}
            onSelectTrack={handleSelectTrack}
          />
        </div>

        {/* Right: Metadata editor */}
        <div className="w-72 min-w-60 border-l border-gray-700/30 flex flex-col overflow-y-auto">
          {state.selectedTrack ? (
            <MetadataEditor
              track={state.selectedTrack}
              dirPath={
                state.selectedTrack.path.split("/").slice(0, -1).join("/")
              }
              coverDataUrl={state.coverDataUrl}
              saving={state.saving}
              onFieldChange={handleFieldChange}
              onChangeCover={handleChangeCover}
              onRemoveCover={handleRemoveCover}
            />
          ) : (
            <div className="flex items-center justify-center h-full text-text-muted text-xs px-4 text-center leading-relaxed">
              {state.tracks.length > 0
                ? "Select a file to edit its tags"
                : "Open a music library\nto get started"}
            </div>
          )}
        </div>
      </div>


    </div>
  );
}
