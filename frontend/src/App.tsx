import React, { useReducer, useCallback, useEffect, useMemo } from "react";
import { appReducer, initialAppState } from "./state/AppState";
import type { TrackSnapshot } from "./state/UndoManager";
import { TitleBar } from "./components/TitleBar";
import { Sidebar } from "./components/Sidebar";
import { FileGrid } from "./components/FileGrid";
import { MetadataEditor } from "./components/MetadataEditor";
import { BatchEditor } from "./components/BatchEditor";
import { ScanProgressBar } from "./components/ScanProgressBar";
import { AuditBanner } from "./components/AuditBanner";
import { AuditPanel } from "./components/AuditPanel";
import { SettingsModal } from "./components/SettingsModal";
import { ConvertDialog } from "./components/ConvertDialog";
import { ExtraTagsEditor } from "./components/ExtraTagsEditor";
import { BatchExtraTagsEditor } from "./components/BatchExtraTagsEditor";
import type { ConvertDirection } from "./components/ConvertDialog";
import type { TrackData, AlbumInfo } from "../electron/preload";

/** Get the parent directory of a path. */
function dirPath(p: string): string {
  return p.split("/").slice(0, -1).join("/");
}

export default function App() {
  const [state, dispatch] = useReducer(appReducer, initialAppState);
  const [showConvertDialog, setShowConvertDialog] = React.useState(false);
  const [extraTagsTrack, setExtraTagsTrack] = React.useState<TrackData | null>(null);
  const [batchExtraTagsOpen, setBatchExtraTagsOpen] = React.useState(false);

  /** Read track data for every album and dispatch results. */
  const loadAlbumTracks = useCallback(
    async (albums: AlbumInfo[]) => {
      const allTracks: TrackData[] = [];
      for (let i = 0; i < albums.length; i++) {
        dispatch({
          type: "SET_SCANNING_PROGRESS",
          progress: { current: i + 1, total: albums.length },
        });
        try {
          const detail = await window.api.readAlbum(albums[i].path);
          allTracks.push(...detail.tracks);
        } catch {
          // Skip albums that fail to read
        }
      }
      dispatch({ type: "SET_SCANNING_PROGRESS", progress: null });
      dispatch({ type: "SET_TRACKS", tracks: allTracks });
    },
    [],
  );

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
        const albums = await window.api.scanLibrary(selectedPath);
        dispatch({ type: "SET_ALBUMS", albums });
        await loadAlbumTracks(albums);
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
  }, [loadAlbumTracks]);

  // --- Album selection ---

  const handleSelectAlbum = useCallback(
    async (albumPath: string | null) => {
      dispatch({ type: "SET_ACTIVE_ALBUM", path: albumPath });

      if (albumPath === null) {
        if (state.libraryPath) {
          dispatch({ type: "SET_SCANNING", scanning: true });
          try {
            const albums = await window.api.scanLibrary(state.libraryPath);
            dispatch({ type: "SET_ALBUMS", albums });
            await loadAlbumTracks(albums);
          } catch {
            // ignore
          } finally {
            dispatch({ type: "SET_SCANNING", scanning: false });
          }
        }
      } else {
        dispatch({ type: "SET_SCANNING", scanning: true });
        try {
          const detail = await window.api.readAlbum(albumPath);
          dispatch({ type: "SET_TRACKS", tracks: detail.tracks });
        } catch {
          dispatch({
            type: "SET_ERROR",
            error: "Failed to read album",
          });
        } finally {
          dispatch({ type: "SET_SCANNING", scanning: false });
        }
      }
    },
    [state.libraryPath, loadAlbumTracks],
  );

  // --- Multi-track selection ---

  const handleMultiSelect = useCallback(
    (paths: string[]) => {
      dispatch({ type: "SET_SELECTED_TRACKS", paths });

      // Still show the primary (first) track's cover art
      if (paths.length > 0) {
        const primary = state.tracks.find((t) => t.path === paths[0]);
        if (primary) {
          window.api.getCoverDataUrl(dirPath(primary.path)).then(
            (url) => dispatch({ type: "SET_COVER_URL", url }),
            () => dispatch({ type: "SET_COVER_URL", url: null }),
          );
        }
      }
    },
    [state.tracks],
  );

  // --- Track selection ---

  const handleSelectTrack = useCallback(
    async (path: string, track: TrackData) => {
      dispatch({ type: "SELECT_TRACK", path, track });

      try {
        const url = await window.api.getCoverDataUrl(dirPath(path));
        dispatch({ type: "SET_COVER_URL", url });
      } catch {
        dispatch({ type: "SET_COVER_URL", url: null });
      }
    },
    [],
  );

  const handleEditExtraTagsFromSelection = useCallback(
    (track: TrackData, selectedPaths: string[]) => {
      if (selectedPaths.length > 1) {
        setExtraTagsTrack(null);
        setBatchExtraTagsOpen(true);
        return;
      }

      setBatchExtraTagsOpen(false);
      setExtraTagsTrack(track);
    },
    [],
  );

  // --- Field editing (batch save via Save button, no auto-save) ---

  const handleSaveMetadata = useCallback(
    async (fields: Record<string, string>) => {
      if (!state.selectedTrack) return;
      const track = state.selectedTrack;

      // Create undo snapshot with previous state
      const snapshot: TrackSnapshot = {
        path: track.path,
        fields: {
          title: track.title,
          artist: track.artist,
          artists: track.artists,
          album: track.album,
          albumArtist: track.albumArtist,
          albumArtists: track.albumArtists,
          year: track.year,
          trackNumber: track.trackNumber,
          trackTotal: track.trackTotal,
          discNumber: track.discNumber,
          discTotal: track.discTotal,
          genre: track.genre,
          composer: track.composer,
          comment: track.comment ?? null,
          musicbrainzTrackId: track.musicbrainzTrackId,
          musicbrainzAlbumId: track.musicbrainzAlbumId,
          musicbrainzArtistId: track.musicbrainzArtistId,
        },
      };
      dispatch({
        type: "PUSH_UNDO",
        description: "Metadata save",
        snapshots: [snapshot],
      });

      // Build write fields and optimistic local state
      const writeFields: Record<string, unknown> = {};
      const updatedTrack = { ...track };

      for (const [field, value] of Object.entries(fields)) {
        switch (field) {
          case "track": {
            const parts = value.split("/");
            updatedTrack.trackNumber = parseNum(parts[0]);
            updatedTrack.trackTotal = parseNum(parts[1]);
            if (parts[0]) writeFields.trackNumber = updatedTrack.trackNumber;
            if (parts[1]) writeFields.trackTotal = updatedTrack.trackTotal;
            break;
          }
          case "disc": {
            const parts = value.split("/");
            updatedTrack.discNumber = parseNum(parts[0]);
            updatedTrack.discTotal = parseNum(parts[1]);
            if (parts[0]) writeFields.discNumber = updatedTrack.discNumber;
            if (parts[1]) writeFields.discTotal = updatedTrack.discTotal;
            break;
          }
          default:
            (updatedTrack as Record<string, unknown>)[field] = value || null;
            writeFields[field] = value || null;
        }
      }

      dispatch({
        type: "UPDATE_TRACK",
        path: track.path,
        track: updatedTrack,
      });
      dispatch({ type: "SET_SAVING", saving: true });

      try {
        const result = await window.api.writeTrack(track.path, writeFields);
        dispatch({
          type: "UPDATE_TRACK",
          path: track.path,
          track: result,
        });

        // Refresh cover if album or title changed
        if (fields.album !== undefined || fields.title !== undefined) {
          try {
            const url = await window.api.getCoverDataUrl(
              dirPath(track.path),
            );
            dispatch({ type: "SET_COVER_URL", url });
          } catch {
            // ignore
          }
        }
      } catch (err: unknown) {
        dispatch({
          type: "UPDATE_TRACK",
          path: track.path,
          track,
        });
        const message =
          err instanceof Error ? err.message : "Failed to save tags";
        dispatch({ type: "SET_ERROR", error: message });
      } finally {
        dispatch({ type: "SET_SAVING", saving: false });
      }
    },
    [state.selectedTrack],
  );

  const handleSaveExtraTags = useCallback(
    async (tags: Array<{ key: string; value: string }>) => {
      if (!extraTagsTrack) return;
      dispatch({ type: "SET_SAVING", saving: true });
      dispatch({ type: "SET_ERROR", error: null });

      try {
        const result = await window.api.writeExtraTags(extraTagsTrack.path, tags);
        dispatch({ type: "UPDATE_TRACK", path: extraTagsTrack.path, track: result });
        setExtraTagsTrack(result);
      } catch (err: unknown) {
        const message =
          err instanceof Error ? err.message : "Failed to save extra tags";
        dispatch({ type: "SET_ERROR", error: message });
        throw err;
      } finally {
        dispatch({ type: "SET_SAVING", saving: false });
      }
    },
    [extraTagsTrack],
  );

  // --- Cover actions ---

  const handleChangeCover = useCallback(async () => {
    if (!state.selectedTrack) return;
    try {
      const url = await window.api.setCover(dirPath(state.selectedTrack.path));
      if (url) {
        dispatch({ type: "SET_COVER_URL", url });
      }
    } catch {
      dispatch({ type: "SET_ERROR", error: "Failed to set cover art" });
    }
  }, [state.selectedTrack]);

  const handleRemoveCover = useCallback(async () => {
    if (!state.selectedTrack) return;
    try {
      await window.api.removeCover(dirPath(state.selectedTrack.path));
      dispatch({ type: "SET_COVER_URL", url: null });
    } catch {
      dispatch({ type: "SET_ERROR", error: "Failed to remove cover art" });
    }
  }, [state.selectedTrack]);

  // --- Undo (triggered by Cmd+Z) ---

  const handleRevert = useCallback(() => {
    const op = state.undoManager.pop();
    if (!op) return;
    for (const snap of op.snapshots) {
      window.api.writeTrack(snap.path, { ...snap.fields }).then((track) => {
        dispatch({ type: "UPDATE_TRACK", path: snap.path, track });
      });
    }
    dispatch({ type: "POP_UNDO" });
  }, [state.undoManager]);

  // --- Auto-Tag ---

  const handleAutoTag = useCallback(async () => {
    if (!state.libraryPath || state.autoTagging) return;

    // Determine which album paths to tag
    const targetPaths = state.activeAlbumPath
      ? [state.activeAlbumPath]
      : state.albums.map((a) => a.path);

    if (targetPaths.length === 0) {
      dispatch({ type: "SET_ERROR", error: "No albums found to tag" });
      return;
    }

    const isBatch = targetPaths.length > 1;

    // Push undo snapshots for all tracks that will be touched
    const albumPathSet = new Set(targetPaths);
    const affectedTracks = state.tracks.filter((t) =>
      albumPathSet.has(t.path.split("/").slice(0, -1).join("/"))
    );
    const snapshots: TrackSnapshot[] = affectedTracks.map((t) => ({
      path: t.path,
      fields: {
        title: t.title,
        artist: t.artist,
        artists: t.artists,
        album: t.album,
        albumArtist: t.albumArtist,
        albumArtists: t.albumArtists,
        year: t.year,
        trackNumber: t.trackNumber,
        trackTotal: t.trackTotal,
        discNumber: t.discNumber,
        discTotal: t.discTotal,
        genre: t.genre,
        composer: t.composer,
        comment: t.comment ?? null,
        musicbrainzTrackId: t.musicbrainzTrackId,
        musicbrainzAlbumId: t.musicbrainzAlbumId,
        musicbrainzArtistId: t.musicbrainzArtistId,
      },
    }));
    dispatch({
      type: "PUSH_UNDO",
      description: `Auto-tag (${targetPaths.length} album${targetPaths.length !== 1 ? "s" : ""})`,
      snapshots,
    });

    dispatch({ type: "SET_AUTO_TAGGING", autoTagging: true });
    dispatch({ type: "SET_ERROR", error: null });

    let completed = 0;
    let totalErrors = 0;

    try {
      for (const albumPath of targetPaths) {
        const albumName = albumPath.split("/").pop() ?? albumPath;
        dispatch({
          type: "SET_AUTO_TAG_PROGRESS",
          progress: isBatch
            ? { current: completed, total: targetPaths.length, message: `${albumName}` }
            : { current: 0, total: 9, message: `Auto-tagging: ${albumName}` },
        });

        const taskId = await window.api.autoTagAlbum(albumPath);
        const unsubscribe = window.api.onAutoTagEvent((event) => {
          if (event.taskId !== taskId) return;
          dispatch({
            type: "SET_AUTO_TAG_PROGRESS",
            progress: isBatch
              ? { current: completed, total: targetPaths.length, message: event.message }
            : { current: event.progress, total: event.total, message: event.message },
          });
        });

        try {
          let done = false;
          while (!done) {
            const progress = await window.api.getTaskProgress(taskId);
            if (!progress) {
              done = true;
              break;
            }

            dispatch({
              type: "SET_AUTO_TAG_PROGRESS",
              progress: isBatch
                ? { current: completed, total: targetPaths.length, message: progress.message }
                : { current: progress.progress, total: progress.total, message: progress.message },
            });

            if (
              progress.status === "completed" ||
              progress.status === "failed" ||
              progress.status === "cancelled"
            ) {
              done = true;
              if (progress.status === "failed") {
                totalErrors++;
                console.debug(`[auto-tag] Auto-tag failed for ${albumName}: ${progress.message}`);
              }
            } else {
              await new Promise((resolve) => setTimeout(resolve, 300));
            }
          }
        } finally {
          unsubscribe();
        }

        completed++;
      }

      // Re-scan after all albums tagged
      const albums = await window.api.scanLibrary(state.libraryPath);
      dispatch({ type: "SET_ALBUMS", albums });

      if (state.activeAlbumPath) {
        try {
          const detail = await window.api.readAlbum(state.activeAlbumPath);
          dispatch({ type: "SET_TRACKS", tracks: detail.tracks });
        } catch {
          dispatch({ type: "SET_ERROR", error: "Failed to re-read album after auto-tag" });
        }
      } else {
        await loadAlbumTracks(albums);
      }

      if (totalErrors > 0) {
        dispatch({
          type: "SET_ERROR",
          error: `Auto-tag completed with ${totalErrors} album(s) with errors`,
        });
      }
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : "Auto-tag failed";
      dispatch({ type: "SET_ERROR", error: message });
    } finally {
      dispatch({ type: "SET_AUTO_TAGGING", autoTagging: false });
      dispatch({ type: "SET_AUTO_TAG_PROGRESS", progress: null });
    }
  }, [
    state.libraryPath,
    state.activeAlbumPath,
    state.albums,
    state.autoTagging,
    loadAlbumTracks,
  ]);

  // --- Audit: LLM-based metadata verification against file paths ---

  const handleAudit = useCallback(async () => {
    if (!state.libraryPath || state.auditing) {
      console.log("[audit] handleAudit skipped — libraryPath=%s auditing=%s", state.libraryPath, state.auditing);
      return;
    }

    const scopeLabel =
      state.selectedTrackPaths.length > 0
        ? `${state.selectedTrackPaths.length} selected track(s)`
        : state.activeAlbumPath
          ? `album “${state.activeAlbumPath.split("/").pop() ?? ""}”`
          : `library “${state.libraryPath}”`;

    console.log("[audit] handleAudit: starting audit for %s", scopeLabel);

    dispatch({ type: "SET_AUDITING", auditing: true });
    dispatch({ type: "CLEAR_AUDIT_RESULTS" });
    dispatch({ type: "SET_ERROR", error: null });

    let unsubscribe: (() => void) | null = null;

    try {
      unsubscribe = window.api.onAuditEvent((event) => {
        console.log("[audit] event received — type=%s msg=%s", event.type, event.message ?? "");
        switch (event.type) {
          case "progress":
            dispatch({
              type: "SET_AUDIT_PROGRESS",
              progress: {
                current: event.current ?? 0,
                total: event.total ?? 1,
                message: event.message ?? "Auditing...",
              },
            });
            break;

          case "album-result":
            if (event.albumPath && event.results) {
              dispatch({
                type: "ADD_AUDIT_RESULTS",
                albumPath: event.albumPath,
                results: event.results.map((r) => ({
                  trackIndex: r.index,
                  field: r.field,
                  status: r.status as "correct" | "warning" | "error",
                  message: r.message ?? null,
                  suggestion: r.suggestion ?? null,
                })),
              });
            }
            break;

          case "completed":
            dispatch({
              type: "SET_AUDIT_PROGRESS",
              progress: {
                current: event.total ?? 0,
                total: event.total ?? 0,
                message: event.message ?? "Audit complete",
              },
            });
            break;

          case "failed":
            dispatch({ type: "SET_ERROR", error: event.message ?? "Audit failed" });
            break;

          case "cancelled":
            break;
        }
      });

      // Determine scope: selected tracks → active album → entire library
      if (state.selectedTrackPaths.length > 0) {
        await window.api.runAuditOnTracks(state.selectedTrackPaths);
      } else if (state.activeAlbumPath) {
        await window.api.runAuditOnAlbums([state.activeAlbumPath]);
      } else {
        await window.api.runAudit(state.libraryPath);
      }

      // Refresh the visible tracks so fixed metadata shows up
      if (state.activeAlbumPath) {
        const detail = await window.api.readAlbum(state.activeAlbumPath);
        dispatch({ type: "SET_TRACKS", tracks: detail.tracks });
      } else {
        const albums = await window.api.scanLibrary(state.libraryPath);
        dispatch({ type: "SET_ALBUMS", albums });
        await loadAlbumTracks(albums);
      }
      console.log("[audit] handleAudit: IPC completed successfully");
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : "Audit failed";
      console.error("[audit] handleAudit: failed — %s", message);
      dispatch({ type: "SET_ERROR", error: message });
    } finally {
      if (unsubscribe) unsubscribe();
      console.log("[audit] handleAudit: cleaning up — auditing=false");
      dispatch({ type: "SET_AUDITING", auditing: false });
      dispatch({ type: "SET_AUDIT_PROGRESS", progress: null });
    }
  }, [state.libraryPath, state.selectedTrackPaths, state.activeAlbumPath, state.auditing, loadAlbumTracks]);

  // --- Get Lyrics ---

  const handleGetLyrics = useCallback(async () => {
    if (!state.libraryPath || state.lyricsGetting) return;

    const targetPaths = state.activeAlbumPath
      ? [state.activeAlbumPath]
      : state.albums.map((a) => a.path);

    if (targetPaths.length === 0) return;

    dispatch({ type: "SET_LYRICS_GETTING", lyricsGetting: true });

    try {
      let totalDownloaded = 0;
      for (const albumPath of targetPaths) {
        const count = await window.api.downloadAlbumLyrics(albumPath);
        totalDownloaded += count;
      }

      if (totalDownloaded > 0) {
        // Refresh the active album to show new lyrics in sidebar
        if (state.activeAlbumPath) {
          const detail = await window.api.readAlbum(state.activeAlbumPath);
          dispatch({ type: "SET_TRACKS", tracks: detail.tracks });
        }
      }

      dispatch({
        type: "SET_ERROR",
        error:
          totalDownloaded > 0
            ? `Got lyrics for ${totalDownloaded} track(s)`
            : null,
      });
    } catch (err: unknown) {
      const message =
        err instanceof Error ? err.message : "Failed to get lyrics";
      dispatch({ type: "SET_ERROR", error: message });
    } finally {
      dispatch({ type: "SET_LYRICS_GETTING", lyricsGetting: false });
    }
  }, [state.libraryPath, state.activeAlbumPath, state.albums, state.lyricsGetting]);

  // --- Convert: prompt for direction + regex, then apply ---

  const handleConvert = useCallback(() => {
    if (!state.selectedTrack) {
      dispatch({
        type: "SET_ERROR",
        error: "Select a file first to convert",
      });
      return;
    }
    setShowConvertDialog(true);
  }, [state.selectedTrack]);

  const handleConvertAction = useCallback(
    async (direction: ConvertDirection, pattern: string) => {
      const track = state.selectedTrack;
      if (!track) return;

      const basename = track.path.split("/").pop() ?? track.path;

      let derived: string | null = null;
      let writeFields: Record<string, unknown> = {};
      let undoFields: Record<string, unknown> = {};
      let description = "";

      if (direction === "title-to-filename") {
        dispatch({
          type: "SET_ERROR",
          error: "Title→filename rename not yet supported by the backend",
        });
        return;
      }

      switch (direction) {
        case "filename-to-title": {
          let result = basename.replace(/\.[^.]+$/, "");
          if (pattern === "strip-number") {
            result = result.replace(/^\d+[\s.\-_)]*\s*/, "");
          } else if (pattern) {
            try {
              const regex = new RegExp(pattern);
              const m = result.match(regex);
              if (m && m[1]) result = m[1];
            } catch {
              dispatch({
                type: "SET_ERROR",
                error: `Invalid regex pattern: ${pattern}`,
              });
              return;
            }
          }
          derived = result.trim();
          writeFields = { title: derived };
          undoFields = { title: track.title };
          description = `Convert title: ${derived}`;
          break;
        }

        case "custom-regex": {
          let result = basename.replace(/\.[^.]+$/, "");
          try {
            const regex = new RegExp(pattern);
            const m = result.match(regex);
            derived = m && m[1] ? m[1] : null;
            if (!derived) {
              dispatch({
                type: "SET_ERROR",
                error: "Regex produced no capture groups",
              });
              return;
            }
          } catch {
            dispatch({
              type: "SET_ERROR",
              error: `Invalid regex: ${pattern}`,
            });
            return;
          }
          derived = derived!.trim();
          writeFields = { title: derived };
          undoFields = { title: track.title };
          description = `Convert (regex): ${derived}`;
          break;
        }

        default:
          return;
      }

      if (!derived || derived === (track.title ?? "")) {
        dispatch({
          type: "SET_ERROR",
          error: "Conversion produced no change",
        });
        return;
      }

      const snapshot: TrackSnapshot = { path: track.path, fields: undoFields };
      dispatch({
        type: "PUSH_UNDO",
        description,
        snapshots: [snapshot],
      });

      const updatedTrack = { ...track, ...writeFields };
      dispatch({
        type: "UPDATE_TRACK",
        path: track.path,
        track: updatedTrack,
      });
      dispatch({ type: "SET_SAVING", saving: true });

      try {
        const result = await window.api.writeTrack(track.path, writeFields);
        dispatch({
          type: "UPDATE_TRACK",
          path: track.path,
          track: result,
        });
      } catch (err: unknown) {
        dispatch({
          type: "UPDATE_TRACK",
          path: track.path,
          track,
        });
        const message =
          err instanceof Error ? err.message : "Failed to save conversion";
        dispatch({ type: "SET_ERROR", error: message });
      } finally {
        dispatch({ type: "SET_SAVING", saving: false });
      }
    },
    [state.selectedTrack],
  );



  // --- Settings ---

  const handleOpenSettings = useCallback(() => {
    dispatch({ type: "TOGGLE_SETTINGS", show: true });
  }, []);

  const handleCloseSettings = useCallback(() => {
    dispatch({ type: "TOGGLE_SETTINGS", show: false });
  }, []);

  // --- Filter ---

  const handleFilterChange = useCallback((text: string) => {
    dispatch({ type: "SET_FILTER", filter: text });
  }, []);

  // --- Debug log subscription (non-critical) ---

  useEffect(() => {
    window.api.subscribeDebugLogs().catch(() => {});
  }, []);

  // --- Dark mode toggle ---

  useEffect(() => {
    document.documentElement.classList.toggle("dark", state.darkMode);
  }, [state.darkMode]);

  const handleToggleDarkMode = useCallback(() => {
    dispatch({ type: "TOGGLE_DARK_MODE" });
  }, []);

  // --- Keyboard shortcuts ---

  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "o") {
        e.preventDefault();
        handleOpenLibrary();
      }
      if ((e.metaKey || e.ctrlKey) && e.key === "t") {
        e.preventDefault();
        handleAutoTag();
      }
      if ((e.metaKey || e.ctrlKey) && e.key === "z" && !e.shiftKey) {
        e.preventDefault();
        handleRevert();
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [handleOpenLibrary, handleAutoTag, handleRevert]);

  // --- File watching: re-scan on page visibility change ---

  useEffect(() => {
    const handleVisibility = async () => {
      if (document.visibilityState === "visible") {
        try {
          await window.api.onFocus();
        } catch {
          // Best-effort
        }
      }
    };
    document.addEventListener("visibilitychange", handleVisibility);
    return () =>
      document.removeEventListener("visibilitychange", handleVisibility);
  }, []);

  // Filter tracks by active album — currently a pass-through (logic in handleSelectAlbum)
  const filteredTracks = state.tracks;

  // Tracks for the currently multi-selected paths
  const selectedTracksForBatch = useMemo(() => {
    const pathSet = new Set(state.selectedTrackPaths);
    return state.tracks.filter((t) => pathSet.has(t.path));
  }, [state.selectedTrackPaths, state.tracks]);

  // Handle batch field save from BatchEditor
  const handleBatchSave = useCallback(
    async (fields: Record<string, string>) => {
      const paths = state.selectedTrackPaths;
      if (paths.length === 0) return;

      const snapshots: TrackSnapshot[] = [];

      for (const path of paths) {
        const track = state.tracks.find((t) => t.path === path);
        if (!track) continue;

        snapshots.push({
          path,
          fields: {
            title: track.title,
            artist: track.artist,
            artists: track.artists,
            album: track.album,
            albumArtist: track.albumArtist,
            albumArtists: track.albumArtists,
            year: track.year,
            trackNumber: track.trackNumber,
            trackTotal: track.trackTotal,
            discNumber: track.discNumber,
            discTotal: track.discTotal,
            genre: track.genre,
            composer: track.composer,
            comment: track.comment ?? null,
            musicbrainzTrackId: track.musicbrainzTrackId,
            musicbrainzAlbumId: track.musicbrainzAlbumId,
            musicbrainzArtistId: track.musicbrainzArtistId,
          },
        });
      }

      dispatch({ type: "PUSH_UNDO", description: "Batch edit", snapshots });

      const writeFields: Record<string, unknown> = {};
      for (const [key, value] of Object.entries(fields)) {
        writeFields[key] = value || null;
      }

      dispatch({ type: "SET_SAVING", saving: true });

      try {
        for (const path of paths) {
          const result = await window.api.writeTrack(path, writeFields);
          dispatch({ type: "UPDATE_TRACK", path, track: result });
        }
      } catch (err: unknown) {
        const message = err instanceof Error ? err.message : "Batch save failed";
        dispatch({ type: "SET_ERROR", error: message });
      } finally {
        dispatch({ type: "SET_SAVING", saving: false });
      }
    },
    [state.selectedTrackPaths, state.tracks],
  );

  // Handle batch extra tags save
  const handleBatchExtraTagsSave = useCallback(
    async (tags: Array<{ key: string; value: string }>) => {
      const paths = state.selectedTrackPaths;
      if (paths.length === 0) return;

      dispatch({ type: "SET_SAVING", saving: true });
      dispatch({ type: "SET_ERROR", error: null });

      try {
        const updates = paths.map((path) => ({ path, tags }));
        const results = await window.api.writeExtraTagsBatch(updates);
        for (let i = 0; i < paths.length; i++) {
          dispatch({ type: "UPDATE_TRACK", path: paths[i], track: results[i] });
        }
        setBatchExtraTagsOpen(false);
      } catch (err: unknown) {
        const message = err instanceof Error ? err.message : "Batch extra tags save failed";
        dispatch({ type: "SET_ERROR", error: message });
      } finally {
        dispatch({ type: "SET_SAVING", saving: false });
      }
    },
    [state.selectedTrackPaths],
  );

  return (
    <div className="flex flex-col h-screen bg-surface text-text-primary overflow-hidden">
      <TitleBar
        libraryPath={state.libraryPath}
        trackCount={filteredTracks.length}
        filterText={state.filterText}
        onFilterChange={handleFilterChange}
        selectedFilePath={state.selectedTrackPath}
        saving={state.saving}
        autoTagging={state.autoTagging}
        lyricsGetting={state.lyricsGetting}
        auditing={state.auditing}
        error={state.error}
        onOpenLibrary={handleOpenLibrary}
        onConvert={handleConvert}
        onAutoTag={handleAutoTag}
        onGetLyrics={handleGetLyrics}
        onAudit={handleAudit}
        darkMode={state.darkMode}
        onToggleDarkMode={handleToggleDarkMode}
        onOpenSettings={handleOpenSettings}
      />

      <ScanProgressBar
        scanning={state.scanning || state.autoTagging}
        progress={
          state.autoTagProgress
            ? {
                current: state.autoTagProgress.current,
                total: state.autoTagProgress.total,
              }
            : state.scanningProgress
        }
        label={state.autoTagProgress?.message ?? null}
      />

      <ScanProgressBar
        scanning={state.auditing}
        progress={
          state.auditProgress
            ? {
                current: state.auditProgress.current,
                total: state.auditProgress.total,
              }
            : null
        }
        label={state.auditProgress?.message ?? null}
      />

      <AuditBanner
        results={state.auditResults}
        onDismiss={() => dispatch({ type: "CLEAR_AUDIT_RESULTS" })}
      />

      <div className="flex flex-1 overflow-hidden">
        <Sidebar
          albums={state.albums}
          libraryPath={state.libraryPath}
          activeAlbumPath={state.activeAlbumPath}
          onSelectAlbum={handleSelectAlbum}
          onOpenLibrary={handleOpenLibrary}
        />

        <div className="flex-1 flex flex-col min-w-0 border-r border-border">
          <FileGrid
            tracks={filteredTracks}
            selectedTrackPath={state.selectedTrackPath}
            selectedTrackPaths={state.selectedTrackPaths}
            filterText={state.filterText}
            onSelectTrack={handleSelectTrack}
            onMultiSelect={handleMultiSelect}
            onEditExtraTags={handleEditExtraTagsFromSelection}
          />
        </div>

        <div className="w-[300px] min-w-[280px] max-w-[360px] flex flex-col overflow-y-auto">
          {state.selectedTrackPaths.length > 1 ? (
            <BatchEditor
              tracks={selectedTracksForBatch}
              coverDataUrl={state.coverDataUrl}
              saving={state.saving}
              onSave={handleBatchSave}
            />
          ) : state.selectedTrack ? (
            <MetadataEditor
              track={state.selectedTrack}
              dirPath={dirPath(state.selectedTrack.path)}
              coverDataUrl={state.coverDataUrl}
              saving={state.saving}
              onSave={handleSaveMetadata}
              onChangeCover={handleChangeCover}
              onRemoveCover={handleRemoveCover}
            />
          ) : state.activeAlbumPath && state.auditResults[state.activeAlbumPath] ? (
            <AuditPanel
              results={state.auditResults[state.activeAlbumPath]}
              albumName={state.activeAlbumPath.split("/").pop() ?? ""}
            />
          ) : (
            <div className="flex items-center justify-center h-full">
              <div className="flex flex-col items-center gap-3 text-text-muted px-8 text-center">
                <svg
                  width="36"
                  height="36"
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="1"
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  className="opacity-30"
                >
                  <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
                  <polyline points="14 2 14 8 20 8" />
                  <line x1="12" y1="18" x2="12" y2="12" />
                  <line x1="9" y1="15" x2="15" y2="15" />
                </svg>
                <div className="text-[12px] leading-relaxed">
                  {state.tracks.length > 0
                    ? "Select a file to edit its tags"
                    : "Open a music library\nto get started"}
                </div>
              </div>
            </div>
          )}
        </div>
      </div>

      <SettingsModal
        open={state.showSettings}
        onClose={handleCloseSettings}
      />

      <ConvertDialog
        open={showConvertDialog}
        onClose={() => setShowConvertDialog(false)}
        onConvert={handleConvertAction}
      />

      {extraTagsTrack && (
        <ExtraTagsEditor
          track={extraTagsTrack}
          saving={state.saving}
          onClose={() => setExtraTagsTrack(null)}
          onSave={handleSaveExtraTags}
        />
      )}

      {batchExtraTagsOpen && state.selectedTrackPaths.length > 1 && (
        <BatchExtraTagsEditor
          tracks={selectedTracksForBatch}
          saving={state.saving}
          onClose={() => setBatchExtraTagsOpen(false)}
          onSave={handleBatchExtraTagsSave}
        />
      )}
    </div>
  );
}

/** Parse a string as track/disc number, returning null on invalid input. */
function parseNum(s: string): number | null {
  return s ? parseInt(s, 10) || null : null;
}
