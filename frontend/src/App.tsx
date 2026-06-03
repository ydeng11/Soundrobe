import React, { useReducer, useCallback, useEffect, useMemo, useRef } from "react";
import { appReducer, initialAppState } from "./state/AppState";
import type { TrackSnapshot } from "./state/UndoManager";
import { TitleBar } from "./components/TitleBar";
import { dirname as dirPath, basename } from "./utils/path";
import { AssistantPanel } from "./components/AssistantPanel";
import { ErrorBoundary } from "./components/ErrorBoundary";
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
import type { ConvertResult } from "./components/ConvertDialog";
import type { ExtraTagUndoSnapshot, TrackData, AlbumInfo } from "../electron/preload";

// dirPath is now imported from ./utils/path

const EXTRA_TAG_UNDO_FIELD = "__assistantExtraTags";

export default function App() {
  const [state, dispatch] = useReducer(appReducer, initialAppState);
  const [showConvertDialog, setShowConvertDialog] = React.useState(false);
  const [extraTagsTrack, setExtraTagsTrack] = React.useState<TrackData | null>(null);
  const [batchExtraTagsOpen, setBatchExtraTagsOpen] = React.useState(false);
  const [showAssistant, setShowAssistant] = React.useState(false);
  const [assistantApiKey, setAssistantApiKey] = React.useState("");
  const [assistantModel, setAssistantModel] = React.useState("");

  // Cover URL cache: albumPath → dataUrl | null
  const coverUrlCacheRef = useRef<Map<string, string | null>>(new Map());
  // Abort controller for stale cover responses
  const coverAbortRef = useRef<AbortController | null>(null);
  // Debounce timer for rapid cover navigation
  const coverDebounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  /** Fetch cover data URL with caching and stale-response guarding. */
  const fetchCover = useCallback(
    (albumPath: string, signal?: AbortSignal) => {
      const cached = coverUrlCacheRef.current.get(albumPath);
      if (cached !== undefined) {
        dispatch({ type: "SET_COVER_URL", url: cached });
        return;
      }

      window.api
        .getCoverDataUrl(albumPath)
        .then(
          (url) => {
            if (signal?.aborted) return;
            coverUrlCacheRef.current.set(albumPath, url);
            dispatch({ type: "SET_COVER_URL", url });
          },
          () => {
            if (signal?.aborted) return;
            coverUrlCacheRef.current.set(albumPath, null);
            dispatch({ type: "SET_COVER_URL", url: null });
          },
        );
    },
    [],
  );

  /** Debounced cover fetch — cancels previous in-flight request. */
  const debouncedFetchCover = useCallback(
    (albumPath: string) => {
      if (coverDebounceRef.current) {
        clearTimeout(coverDebounceRef.current);
      }
      if (coverAbortRef.current) {
        coverAbortRef.current.abort();
      }
      const abort = new AbortController();
      coverAbortRef.current = abort;

      coverDebounceRef.current = setTimeout(() => {
        fetchCover(albumPath, abort.signal);
      }, 80);
    },
    [fetchCover],
  );

  // Cleanup debounce and abort on unmount
  useEffect(() => {
    return () => {
      if (coverDebounceRef.current) clearTimeout(coverDebounceRef.current);
      if (coverAbortRef.current) coverAbortRef.current.abort();
    };
  }, []);

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
          fetchCover(dirPath(primary.path));
        }
      }
    },
    [state.tracks, fetchCover],
  );

  // --- Track selection ---

  const handleSelectTrack = useCallback(
    (path: string, track: TrackData) => {
      dispatch({ type: "SELECT_TRACK", path, track });
      debouncedFetchCover(dirPath(path));
    },
    [debouncedFetchCover],
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

  // --- Delete files ---

  const handleDeleteFiles = useCallback(
    async (paths: string[]) => {
      if (paths.length === 0) return;

      const plural = paths.length !== 1;
      const confirmMsg = `Delete ${paths.length} file${plural ? "s" : ""} permanently?\n\nThis cannot be undone.`;
      if (!window.confirm(confirmMsg)) return;

      dispatch({ type: "SET_SAVING", saving: true });
      dispatch({ type: "SET_ERROR", error: null });

      try {
        const results = await window.api.deleteFiles(paths);

        const failed = results.filter((r) => !r.success);
        if (failed.length > 0) {
          const messages = failed.map((r) => `${r.path}: ${r.error}`).join("; ");
          dispatch({ type: "SET_ERROR", error: `Failed to delete ${failed.length} file(s): ${messages}` });
        }

        // Remove deleted paths from state
        const deletedSet = new Set(results.filter((r) => r.success).map((r) => r.path));
        const remaining = state.tracks.filter((t) => !deletedSet.has(t.path));
        dispatch({ type: "SET_TRACKS", tracks: remaining });

        // Clear selection if selected files were deleted
        const hadSelected = state.selectedTrackPaths.some((p) => deletedSet.has(p));
        if (hadSelected) {
          dispatch({ type: "CLEAR_SELECTION" });
        }
      } catch (err: unknown) {
        const message =
          err instanceof Error ? err.message : "Failed to delete files";
        dispatch({ type: "SET_ERROR", error: message });
      } finally {
        dispatch({ type: "SET_SAVING", saving: false });
      }
    },
    [state.tracks, state.selectedTrackPaths],
  );

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

        // Refresh cover if album or title changed (clear cache so re-fetch is fresh)
        if (fields.album !== undefined || fields.title !== undefined) {
          coverUrlCacheRef.current.delete(dirPath(track.path));
          fetchCover(dirPath(track.path));
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

  const handleRevert = useCallback(async () => {
    const op = state.undoManager.pop();
    if (!op) return;
    if (op.snapshots.length === 0) {
      console.warn("Undo popped an operation with 0 snapshots — nothing to revert");
      dispatch({ type: "POP_UNDO" });
      return;
    }
    await Promise.all(
      op.snapshots.map(async (snap) => {
        // Detect rename undo: fields contains a "path" key (string) =
        // this was a file rename operation, not a tag write
        const oldPath =
          typeof snap.fields.path === "string" ? snap.fields.path : null;
        const extraTags = snap.fields[EXTRA_TAG_UNDO_FIELD];
        if (Array.isArray(extraTags)) {
          try {
            const track = await window.api.writeExtraTags(
              snap.path,
              extraTags as Array<{ key: string; value: string }>,
            );
            dispatch({ type: "UPDATE_TRACK", path: snap.path, track });
          } catch {
            console.warn("Undo extra tags failed for:", snap.path);
          }
        } else if (oldPath && snap.path !== oldPath) {
          // Rename the file back to its original path
          try {
            const track = await window.api.renameTrack(snap.path, oldPath);
            dispatch({ type: "UPDATE_TRACK", path: snap.path, track: { ...track, path: oldPath } });
          } catch {
            console.warn("Undo rename failed for:", snap.path);
          }
        } else {
          // Normal tag write undo
          try {
            const { path: _path, ...fields } = snap.fields;
            const track = await window.api.writeTrack(snap.path, fields);
            dispatch({ type: "UPDATE_TRACK", path: snap.path, track });
          } catch {
            console.warn("Undo write failed for:", snap.path);
          }
        }
      }),
    );
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
      albumPathSet.has(dirPath(t.path))
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
        const albumName = basename(albumPath) ?? albumPath;
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
    state.tracks,
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
          ? `album “${basename(state.activeAlbumPath) ?? ""}”`
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
    if (state.selectedTrackPaths.length === 0) {
      dispatch({
        type: "SET_ERROR",
        error: "Select a file first to convert",
      });
      return;
    }
    // Clear any stale error before opening
    if (state.error) {
      dispatch({ type: "SET_ERROR", error: null });
    }
    setShowConvertDialog(true);
  }, [state.selectedTrackPaths.length, state.error]);

  const handleConvertAction = useCallback(
    async (result: ConvertResult) => {
      // Use the primary selected track or find from the first selected path
      let track = state.selectedTrack;
      if (!track && state.selectedTrackPaths.length > 0) {
        const firstPath = state.selectedTrackPaths[0];
        track = state.tracks.find((t) => t.path === firstPath) ?? null;
      }
      if (!track) return;

      if (result.direction === "filename-to-tags") {
        // ── Filename → Tags: extract fields from filename and write to tags ──
        const writeFields = result.writeFields as Record<string, unknown>;
        const undoFields: Record<string, unknown> = {};

        // Build undo fields from current track values
        const trackRecord = track as unknown as Record<string, unknown>;
        for (const key of Object.keys(writeFields)) {
          undoFields[key] = trackRecord[key] ?? null;
        }

        if (Object.keys(writeFields).length === 0) {
          dispatch({
            type: "SET_ERROR",
            error: "No fields to write from the conversion",
          });
          return;
        }

        const descriptionLines = Object.entries(writeFields)
          .map(([k, v]) => `${k}=${v}`)
          .join(", ");

        const snapshot: TrackSnapshot = {
          path: track.path,
          fields: undoFields,
        };
        dispatch({
          type: "PUSH_UNDO",
          description: `Convert: ${descriptionLines}`,
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
          const apiResult = await window.api.writeTrack(
            track.path,
            writeFields
          );
          dispatch({
            type: "UPDATE_TRACK",
            path: track.path,
            track: apiResult,
          });
        } catch (err: unknown) {
          dispatch({
            type: "UPDATE_TRACK",
            path: track.path,
            track,
          });
          const message =
            err instanceof Error
              ? err.message
              : "Failed to save conversion";
          dispatch({ type: "SET_ERROR", error: message });
        } finally {
          dispatch({ type: "SET_SAVING", saving: false });
        }
      } else if (result.direction === "tags-to-filename") {
        // ── Tags → Filename: rename the file on disk ──
        if (!result.newFilename) {
          dispatch({
            type: "SET_ERROR",
            error: "No new filename from conversion",
          });
          return;
        }

        const oldDir = track.path.substring(
          0,
          track.path.lastIndexOf("/") + 1
        );
        const newPath = oldDir + result.newFilename;

        if (newPath === track.path) {
          dispatch({
            type: "SET_ERROR",
            error: "New filename is identical to current filename",
          });
          return;
        }

        // Check if target file already exists
        try {
          const exists = await window.api.checkFileExists(newPath);
          if (exists) {
            dispatch({
              type: "SET_ERROR",
              error: `Target file already exists: ${result.newFilename}`,
            });
            return;
          }
        } catch {
          // Ignore check errors, proceed with rename
        }

        // Undo: current path = newPath (to find the file), fields.path = old path (to restore)
        const undoSnapshot: TrackSnapshot = {
          path: newPath,
          fields: { path: track.path },
        };

        dispatch({
          type: "PUSH_UNDO",
          description: `Rename: ${basename(track.path)} → ${result.newFilename}`,
          snapshots: [undoSnapshot],
        });

        dispatch({ type: "SET_SAVING", saving: true });

        try {
          const updatedTrack = await window.api.renameTrack(
            track.path,
            newPath
          );
          // Update the track in the list with the new path
          dispatch({
            type: "UPDATE_TRACK",
            path: track.path,
            track: { ...updatedTrack, path: newPath },
          });
          // Refresh album to get a clean track list
          const albumPath = newPath.substring(0, newPath.lastIndexOf("/"));
          const refreshed = await window.api.readAlbum(albumPath);
          // Replace the full track list with refreshed data
          dispatch({
            type: "SET_TRACKS",
            tracks: refreshed.tracks,
          });
          // Select the renamed track by its new path
          const renamedTrackInList = refreshed.tracks.find(
            (t) => t.path === newPath
          );
          if (renamedTrackInList) {
            dispatch({
              type: "SELECT_TRACK",
              path: newPath,
              track: renamedTrackInList,
            });
          }
        } catch (err: unknown) {
          const message =
            err instanceof Error
              ? err.message
              : "Failed to rename file";
          dispatch({ type: "SET_ERROR", error: message });
        } finally {
          dispatch({ type: "SET_SAVING", saving: false });
        }
      }
    },
    [state.selectedTrack, state.selectedTrackPaths, state.tracks]
  );



  // --- Settings ---

  const handleOpenSettings = useCallback(() => {
    dispatch({ type: "TOGGLE_SETTINGS", show: true });
  }, []);

  const handleCloseSettings = useCallback(() => {
    dispatch({ type: "TOGGLE_SETTINGS", show: false });
  }, []);

  // --- Assistant ---

  const handleToggleAssistant = useCallback(() => {
    setShowAssistant((prev) => !prev);
  }, []);

  const handleCloseAssistant = useCallback(() => {
    setShowAssistant(false);
  }, []);

  // Load API key for assistant on mount, when assistant panel opens, or settings change
  useEffect(() => {
    window.api.getConfig().then(
      (cfg) => {
        setAssistantApiKey((cfg.llmApiKey as string) ?? "");
        setAssistantModel((cfg.llmModel as string) ?? "");
      },
      () => {
        // Silently fail — assistant just won't work until API key is configured
      },
    );
  }, [state.showSettings, showAssistant]); // Re-read when settings or assistant opens

  const handleAssistantRefresh = useCallback(async () => {
    // Re-read current album or library after assistant applies changes
    if (state.activeAlbumPath) {
      try {
        const detail = await window.api.readAlbum(state.activeAlbumPath);
        dispatch({ type: "SET_TRACKS", tracks: detail.tracks });
      } catch {
        // Ignore — refresh best-effort
      }
    } else if (state.libraryPath) {
      try {
        const albums = await window.api.scanLibrary(state.libraryPath);
        dispatch({ type: "SET_ALBUMS", albums });
      } catch {
        // Ignore
      }
    }
  }, [state.activeAlbumPath, state.libraryPath]);

  const handleAssistantApplyUndo = useCallback(
    (
      description: string,
      snapshots: Array<{ path: string; metadata?: Record<string, unknown> } | ExtraTagUndoSnapshot>,
      kind: "tag-update" | "extra-tag-update",
    ) => {
      const trackSnapshots = snapshots.map((s) => ({
        path: s.path,
        fields: kind === "extra-tag-update"
          ? { [EXTRA_TAG_UNDO_FIELD]: (s as ExtraTagUndoSnapshot).extraTags }
          : ((s as { metadata?: Record<string, unknown> }).metadata ?? {}),
      }));
      dispatch({ type: "PUSH_UNDO", description, snapshots: trackSnapshots });
    },
    [],
  );

  const handleAssistantRunTask = useCallback(
    async (task: "auto_tag" | "audit", trackPaths: string[]) => {
      if (!state.libraryPath || trackPaths.length === 0) return;

      if (task === "audit") {
        if (state.auditing) return;

        dispatch({ type: "SET_AUDITING", auditing: true });
        dispatch({ type: "CLEAR_AUDIT_RESULTS" });
        dispatch({ type: "SET_ERROR", error: null });

        let unsubscribe: (() => void) | null = null;
        try {
          unsubscribe = window.api.onAuditEvent((event) => {
            if (event.type === "progress") {
              dispatch({
                type: "SET_AUDIT_PROGRESS",
                progress: {
                  current: event.current ?? 0,
                  total: event.total ?? 1,
                  message: event.message ?? "Auditing...",
                },
              });
            } else if (event.type === "album-result" && event.albumPath && event.results) {
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
            } else if (event.type === "failed") {
              dispatch({ type: "SET_ERROR", error: event.message ?? "Audit failed" });
            }
          });

          await window.api.runAuditOnTracks(trackPaths);
          await handleAssistantRefresh();
        } catch (err: unknown) {
          const message = err instanceof Error ? err.message : "Audit failed";
          dispatch({ type: "SET_ERROR", error: message });
        } finally {
          if (unsubscribe) unsubscribe();
          dispatch({ type: "SET_AUDITING", auditing: false });
          dispatch({ type: "SET_AUDIT_PROGRESS", progress: null });
        }
        return;
      }

      if (state.autoTagging) return;

      const albumPaths = Array.from(new Set(trackPaths.map(dirPath)));
      const affectedAlbums = new Set(albumPaths);
      const snapshots: TrackSnapshot[] = state.tracks
        .filter((track) => affectedAlbums.has(dirPath(track.path)))
        .map((track) => ({
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
        }));
      if (snapshots.length > 0) {
        dispatch({
          type: "PUSH_UNDO",
          description: `Assistant auto-tag (${albumPaths.length} album${albumPaths.length !== 1 ? "s" : ""})`,
          snapshots,
        });
      }

      dispatch({ type: "SET_AUTO_TAGGING", autoTagging: true });
      dispatch({ type: "SET_ERROR", error: null });

      try {
        let completed = 0;
        for (const albumPath of albumPaths) {
          const taskId = await window.api.autoTagAlbum(albumPath);
          let done = false;
          while (!done) {
            const progress = await window.api.getTaskProgress(taskId);
            if (!progress) break;

            dispatch({
              type: "SET_AUTO_TAG_PROGRESS",
              progress: {
                current: completed,
                total: albumPaths.length,
                message: progress.message,
              },
            });

            if (
              progress.status === "completed" ||
              progress.status === "failed" ||
              progress.status === "cancelled"
            ) {
              done = true;
            } else {
              await new Promise((resolve) => setTimeout(resolve, 300));
            }
          }
          completed++;
        }
        await handleAssistantRefresh();
      } catch (err: unknown) {
        const message = err instanceof Error ? err.message : "Auto-tag failed";
        dispatch({ type: "SET_ERROR", error: message });
      } finally {
        dispatch({ type: "SET_AUTO_TAGGING", autoTagging: false });
        dispatch({ type: "SET_AUTO_TAG_PROGRESS", progress: null });
      }
    },
    [
      handleAssistantRefresh,
      state.auditing,
      state.autoTagging,
      state.libraryPath,
      state.tracks,
    ],
  );

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

  // --- Auto-dismiss errors after 5 seconds ---

  useEffect(() => {
    if (!state.error) return;
    const timer = setTimeout(() => {
      dispatch({ type: "SET_ERROR", error: null });
    }, 5000);
    return () => clearTimeout(timer);
  }, [state.error]);

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

  // Handle batch field save from BatchEditor — single IPC call
  const handleBatchSave = useCallback(
    async (fields: Record<string, string>) => {
      const paths = state.selectedTrackPaths;
      if (paths.length === 0) return;

      const snapshots: TrackSnapshot[] = [];
      const updates: Array<{ path: string; fields: Record<string, unknown> }> = [];

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

        const writeFields: Record<string, unknown> = {};
        for (const [key, value] of Object.entries(fields)) {
          writeFields[key] = value || null;
        }
        updates.push({ path, fields: writeFields });
      }

      dispatch({ type: "PUSH_UNDO", description: "Batch edit", snapshots });
      dispatch({ type: "SET_SAVING", saving: true });

      try {
        const results = await window.api.writeTracks(updates);
        dispatch({ type: "UPDATE_TRACKS", tracks: results });
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
        dispatch({ type: "UPDATE_TRACKS", tracks: results });
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
        onToggleAssistant={handleToggleAssistant}
        onErrorDismiss={() => dispatch({ type: "SET_ERROR", error: null })}
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
            onDeleteFiles={handleDeleteFiles}
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
              albumName={basename(state.activeAlbumPath) ?? ""}
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

      <ErrorBoundary>
        <AssistantPanel
          isOpen={showAssistant}
          onClose={handleCloseAssistant}
          apiKey={assistantApiKey}
          model={assistantModel}
          libraryPath={state.libraryPath}
          activeAlbumPath={state.activeAlbumPath}
          selectedTrackPaths={state.selectedTrackPaths}
          allTracks={state.tracks}
          allAlbums={state.albums}
          autonomous={false}
          onRefreshRequest={handleAssistantRefresh}
          onAssistantRunTask={handleAssistantRunTask}
          onAssistantApplyUndo={handleAssistantApplyUndo}
        />
      </ErrorBoundary>

      <SettingsModal
        open={state.showSettings}
        onClose={handleCloseSettings}
      />

      <ConvertDialog
        open={showConvertDialog}
        onClose={() => setShowConvertDialog(false)}
        onConvert={handleConvertAction}
        track={
          state.selectedTrack
            ? {
                filename:
                  basename(state.selectedTrack.path) ??
                  state.selectedTrack.path,
                title: state.selectedTrack.title,
                artist: state.selectedTrack.artist,
                album: state.selectedTrack.album,
                year: state.selectedTrack.year,
                track: state.selectedTrack.trackNumber,
                genre: state.selectedTrack.genre,
                albumArtist: state.selectedTrack.albumArtist,
                composer: state.selectedTrack.composer,
                comment: state.selectedTrack.comment,
                discNumber: state.selectedTrack.discNumber,
              }
            : null
        }
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
