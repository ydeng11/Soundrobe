import React, { useReducer, useCallback, useEffect, useMemo, useRef } from "react";
import {
  appReducer,
  buildAuditApplyAlbumResults,
  buildAuditByTrackPath,
  getVisibleAuditResult,
  initialAppState,
  type AuditApplyAlbumResult,
} from "./state/AppState";
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
import { AuditPanel, SelectedTrackAuditFindings } from "./components/AuditPanel";
import { SettingsModal } from "./components/SettingsModal";
import { ConvertDialog } from "./components/ConvertDialog";
import { ExtraTagsEditor } from "./components/ExtraTagsEditor";
import { BatchExtraTagsEditor } from "./components/BatchExtraTagsEditor";
import type { ConvertResult } from "./components/ConvertDialog";
import {
  parseFilenameWithConvertPattern,
  parseTextWithConvertPattern,
  buildFilenameFromConvertPattern,
  getConvertSourceValue,
  type ConvertTrackData,
} from "../electron/services/ConvertService";
import type { ExtraTagUndoSnapshot, TrackData, AlbumInfo, AlbumDetail, AuditRunSummary, AuditTrackResult } from "../electron/preload";
import {
  computeNumberedTracks,
  type OrderingRule,
} from "../electron/services/TrackNumberingService";

const EXTRA_TAG_UNDO_FIELD = "__assistantExtraTags";

function mapAuditResultForState(r: {
  index: number;
  field: string;
  status: "correct" | "warning" | "error";
  message?: string | null;
  suggestion?: string | null;
  source?: "deterministic" | "llm";
  confidence?: number;
  autoFixEligible?: boolean;
  autoFixed?: boolean;
  corrected?: AuditTrackResult["corrected"];
}) {
  return {
    trackIndex: r.index,
    field: r.field,
    status: r.status,
    message: r.message ?? null,
    suggestion: r.suggestion ?? null,
    source: r.source,
    confidence: r.confidence,
    autoFixEligible: r.autoFixEligible,
    autoFixed: r.autoFixed,
    corrected: r.corrected,
  };
}

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
  // Monotonic generation counter for save-rollback freshness guard
  const saveGenerationRef = useRef(0);

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
      const trackGroups: TrackData[][] = Array.from({ length: albums.length }, () => []);
      const concurrency = 4;
      let nextIndex = 0;
      let completed = 0;

      const processAlbum = async (album: AlbumInfo) => {
        try {
          const detail = await window.api.readAlbum(album.path);
          return detail.tracks;
        } catch {
          return [] as TrackData[];
        }
      };

      // Process albums concurrently with a concurrency limit
      const worker = async () => {
        while (true) {
          const idx = nextIndex++;
          if (idx >= albums.length) break;
          dispatch({
            type: "SET_SCANNING_PROGRESS",
            progress: { current: Math.min(completed + 1, albums.length), total: albums.length },
          });
          const tracks = await processAlbum(albums[idx]);
          trackGroups[idx] = tracks;
          completed += 1;
        }
      };

      const workers = Array.from({ length: Math.min(concurrency, albums.length) }, () => worker());
      await Promise.all(workers);

      dispatch({ type: "SET_SCANNING_PROGRESS", progress: null });
      const allTracks = trackGroups.flat();
      dispatch({ type: "SET_TRACKS", tracks: allTracks });
    },
    [],
  );

  /** Full library re-scan — called on manual refresh. */
  const handleRefresh = useCallback(async () => {
    if (!state.libraryPath) return;
    dispatch({ type: "SET_SCANNING", scanning: true });
    dispatch({ type: "SET_ERROR", error: null });
    try {
      const albums = await window.api.scanLibrary(state.libraryPath);
      dispatch({ type: "SET_ALBUMS", albums });
      await loadAlbumTracks(albums);
    } catch (err: unknown) {
      const message =
        err instanceof Error ? err.message : "Failed to refresh library";
      dispatch({ type: "SET_ERROR", error: message });
    } finally {
      dispatch({ type: "SET_SCANNING", scanning: false });
    }
  }, [state.libraryPath, loadAlbumTracks]);

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

  // --- Album selection (in-memory filter, no disk reads) ---

  const handleSelectAlbum = useCallback(
    (albumPath: string | null) => {
      // Just update the filter key — tracks are filtered at render time
      dispatch({ type: "SET_ACTIVE_ALBUM", path: albumPath });
    },
    [],
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

      const snapshot = createTrackSnapshot(track);
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

      // Capture save generation for rollback freshness guard
      const thisGeneration = ++saveGenerationRef.current;

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
        // Only roll back if no further save has been attempted since our optimistic write
        if (saveGenerationRef.current === thisGeneration) {
          dispatch({
            type: "UPDATE_TRACK",
            path: track.path,
            track,
          });
        }
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
    const albumPath = dirPath(state.selectedTrack.path);
    try {
      await window.api.removeCover(albumPath);
      coverUrlCacheRef.current.set(albumPath, null);
      dispatch({ type: "SET_COVER_URL", url: null });
    } catch {
      dispatch({ type: "SET_ERROR", error: "Failed to remove cover art" });
    }
  }, [state.selectedTrack]);

  const handleDownloadCover = useCallback(async () => {
    if (!state.selectedTrack) return;
    const albumPath = dirPath(state.selectedTrack.path);
    dispatch({ type: "SET_SAVING", saving: true });
    dispatch({ type: "SET_ERROR", error: null });
    try {
      const dataUrl = await window.api.downloadCoverArt(albumPath);
      if (dataUrl) {
        dispatch({ type: "SET_COVER_URL", url: dataUrl });
        coverUrlCacheRef.current.set(albumPath, dataUrl);
      } else {
        dispatch({ type: "SET_ERROR", error: "No cover art found from any source" });
      }
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : "Cover download failed";
      dispatch({ type: "SET_ERROR", error: message });
    } finally {
      dispatch({ type: "SET_SAVING", saving: false });
    }
  }, [state.selectedTrack]);

  const handleDownloadArtistArt = useCallback(async () => {
    if (!state.selectedTrack) return;
    const albumPath = dirPath(state.selectedTrack.path);
    dispatch({ type: "SET_SAVING", saving: true });
    dispatch({ type: "SET_ERROR", error: null });
    try {
      const result = await window.api.downloadArtistArt(albumPath);
      if (result) {
        dispatch({ type: "SET_ERROR", error: `Artist image saved from ${result.source}` });
      } else {
        dispatch({ type: "SET_ERROR", error: "No artist image found from any source" });
      }
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : "Artist image download failed";
      dispatch({ type: "SET_ERROR", error: message });
    } finally {
      dispatch({ type: "SET_SAVING", saving: false });
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

    dispatch({ type: "SET_AUTO_TAGGING", autoTagging: true });
    dispatch({ type: "SET_ERROR", error: null });

    let completed = 0;
    let totalErrors = 0;

    try {
      const snapshots = await buildAutoTagUndoSnapshots(
        targetPaths,
        state.tracks,
        window.api.readAlbum,
      );
      dispatch({
        type: "PUSH_UNDO",
        description: `Auto-tag (${targetPaths.length} album${targetPaths.length !== 1 ? "s" : ""})`,
        snapshots,
      });

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

      // Scoped refresh: only re-read tracks for tagged albums
      dispatch({
        type: "SET_AUTO_TAG_PROGRESS",
        progress: isBatch
          ? { current: completed, total: targetPaths.length, message: "Refreshing tracks..." }
          : { current: 9, total: 9, message: "Refreshing tracks..." },
      });
      const scannedAlbums = await window.api.scanLibrary(state.libraryPath);
      dispatch({ type: "SET_ALBUMS", albums: scannedAlbums });

      const taggedAlbumSet = new Set(targetPaths);
      if (state.activeAlbumPath) {
        taggedAlbumSet.add(state.activeAlbumPath);
      }
      const updatedTrackList: TrackData[] = [];
      for (const albumPath of taggedAlbumSet) {
        try {
          const detail = await window.api.readAlbum(albumPath);
          updatedTrackList.push(...detail.tracks);
        } catch {
          // Skip albums that fail to read
        }
      }
      if (updatedTrackList.length > 0) {
        dispatch({ type: "UPDATE_TRACKS", tracks: updatedTrackList });
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
                results: event.results.map(mapAuditResultForState),
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
      let auditResult: AuditRunSummary;
      if (state.selectedTrackPaths.length > 0) {
        auditResult = await window.api.runAuditOnTracks(state.selectedTrackPaths);
      } else if (state.activeAlbumPath) {
        auditResult = await window.api.runAuditOnAlbums([state.activeAlbumPath]);
      } else {
        auditResult = await window.api.runAudit(state.libraryPath);
      }

      for (const albumResult of auditResult.albumResults ?? []) {
        dispatch({
          type: "ADD_AUDIT_RESULTS",
          albumPath: albumResult.albumPath,
          results: albumResult.results.map(mapAuditResultForState),
        });
      }

      // Scoped refresh: only re-read tracks for audited albums
      dispatch({
        type: "SET_AUDIT_PROGRESS",
        progress: { current: 0, total: 1, message: "Refreshing tracks..." },
      });
      if (state.activeAlbumPath) {
        const detail = await window.api.readAlbum(state.activeAlbumPath);
        dispatch({ type: "UPDATE_TRACKS", tracks: detail.tracks });
      } else if (state.selectedTrackPaths.length > 0) {
        // Re-read albums containing selected tracks
        const albumPaths = [...new Set(state.selectedTrackPaths.map(dirPath))];
        const updatedAuditTracks: TrackData[] = [];
        for (const ap of albumPaths) {
          try {
            const detail = await window.api.readAlbum(ap);
            updatedAuditTracks.push(...detail.tracks);
          } catch {
            // Skip albums that fail to read
          }
        }
        if (updatedAuditTracks.length > 0) {
          dispatch({ type: "UPDATE_TRACKS", tracks: updatedAuditTracks });
        }
        // Also refresh album metadata
        const albums = await window.api.scanLibrary(state.libraryPath);
        dispatch({ type: "SET_ALBUMS", albums });
      } else {
        // Full library audit — re-read everything
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

  // --- Convert: prompt for direction + placeholder pattern, then apply ---

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

  /** Build a ConvertTrackData from a TrackData for the ConvertService functions. */
  function toConvertTrack(track: TrackData): ConvertTrackData {
    return {
      filename: basename(track.path) ?? track.path,
      title: track.title,
      artist: track.artist,
      album: track.album,
      year: track.year,
      track: track.trackNumber,
      genre: track.genre,
      albumArtist: track.albumArtist,
      composer: track.composer,
      comment: track.comment,
      discNumber: track.discNumber,
    };
  }

  const handleConvertAction = useCallback(
    async (result: ConvertResult) => {
      const pathSet = new Set(state.selectedTrackPaths);
      const targetTracks = state.tracks.filter((t) => pathSet.has(t.path));
      if (targetTracks.length === 0) {
        dispatch({
          type: "SET_ERROR",
          error: "No tracks found to convert — try selecting the files again",
        });
        return;
      }

      dispatch({ type: "SET_SAVING", saving: true });
      const errors: string[] = [];
      const successes: string[] = [];
      const undoSnapshots: TrackSnapshot[] = [];

      if (
        result.direction === "filename-to-tags" ||
        result.direction === "tag-to-tags"
      ) {
        for (const track of targetTracks) {
          const filename = basename(track.path) ?? track.path;
          const convertTrack = toConvertTrack(track);

          // Parse this track's source with the pattern
          const sourceValue =
            result.direction === "filename-to-tags"
              ? filename
              : getConvertSourceValue(
                  convertTrack,
                  result.sourceTag ?? "title"
                );

          if (
            result.direction === "tag-to-tags" &&
            !sourceValue.trim()
          ) {
            errors.push(
              `${filename}: ${result.sourceTag ?? "title"} tag is empty`
            );
            continue;
          }

          const parsed =
            result.direction === "filename-to-tags"
              ? parseFilenameWithConvertPattern(
                  result.pattern,
                  filename
                )
              : parseTextWithConvertPattern(
                  result.pattern,
                  sourceValue
                );

          if ("error" in parsed) {
            errors.push(`${filename}: ${parsed.error}`);
            continue;
          }

          const writeFields = parsed.fields as Record<string, unknown>;
          if (Object.keys(writeFields).length === 0) {
            errors.push(`${filename}: No fields extracted`);
            continue;
          }

          // Build undo fields
          const undoFields: Record<string, unknown> = {};
          for (const key of Object.keys(writeFields)) {
            if (key === "track") {
              undoFields.track = track.trackNumber ?? null;
            } else if (key === "disc") {
              undoFields.disc = track.discNumber ?? null;
            } else {
              const tr = track as unknown as Record<string, unknown>;
              undoFields[key] = tr[key] ?? null;
            }
          }
          undoSnapshots.push({
            path: track.path,
            fields: undoFields,
          });

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
            successes.push(filename);
          } catch (err: unknown) {
            errors.push(
              `${filename}: ${
                err instanceof Error ? err.message : "write failed"
              }`
            );
          }
        }

        if (undoSnapshots.length > 0) {
          dispatch({
            type: "PUSH_UNDO",
            description: `Convert ${undoSnapshots.length} track(s) using "${result.pattern}"`,
            snapshots: undoSnapshots,
          });
        }
      } else if (result.direction === "tags-to-filename") {
        // ── Tags → Filename: rename each file ──
        if (!result.filenameTemplate) {
          dispatch({
            type: "SET_ERROR",
            error: "No filename template from conversion",
          });
          dispatch({ type: "SET_SAVING", saving: false });
          return;
        }

        for (const track of targetTracks) {
          const filename = basename(track.path) ?? track.path;
          const convertTrack = toConvertTrack(track);

          const newFilename = buildFilenameFromConvertPattern(
            result.filenameTemplate,
            convertTrack
          );

          if (!newFilename || newFilename === filename) {
            errors.push(`${filename}: no change needed`);
            continue;
          }

          const oldDir = track.path.substring(
            0,
            track.path.lastIndexOf("/") + 1
          );
          const newPath = oldDir + newFilename;

          // Check if target exists
          try {
            const exists = await window.api.checkFileExists(newPath);
            if (exists) {
              // If same path (e.g. after prev rename), skip
              if (newPath !== track.path) {
                errors.push(
                  `${filename}: target already exists (${newFilename})`
                );
                continue;
              }
            }
          } catch {
            // Ignore check errors
          }

          undoSnapshots.push({
            path: newPath,
            fields: { path: track.path },
          });

          try {
            const updatedTrack = await window.api.renameTrack(
              track.path,
              newPath
            );
            dispatch({
              type: "UPDATE_TRACK",
              path: track.path,
              track: { ...updatedTrack, path: newPath },
            });
            successes.push(`${filename} → ${newFilename}`);
          } catch (err: unknown) {
            errors.push(
              `${filename}: ${
                err instanceof Error ? err.message : "rename failed"
              }`
            );
          }
        }

        if (undoSnapshots.length > 0) {
          dispatch({
            type: "PUSH_UNDO",
            description: `Rename ${undoSnapshots.length} track(s)`,
            snapshots: undoSnapshots,
          });
        }

        // Refresh album after renames
        if (successes.length > 0) {
          const albumPaths = [
            ...new Set(
              targetTracks.map((t) =>
                t.path.substring(0, t.path.lastIndexOf("/"))
              )
            ),
          ];
          for (const albumPath of albumPaths) {
            try {
              const refreshed = await window.api.readAlbum(albumPath);
              dispatch({ type: "UPDATE_TRACKS", tracks: refreshed.tracks });
            } catch {
              // Best effort
            }
          }
        }
      }

      dispatch({ type: "SET_SAVING", saving: false });

      // Show aggregate result
      if (errors.length === 0) {
        if (successes.length > 0) {
          dispatch({
            type: "SET_ERROR",
            error: `Converted ${successes.length} track(s) successfully`,
          });
        }
      } else {
        const summary = `Convert completed with ${errors.length} error(s) out of ${targetTracks.length} track(s).`;
        const details = errors.slice(0, 5).join("; ");
        const fullMessage =
          errors.length > 5
            ? `${summary} First 5: ${details} (+${errors.length - 5} more)`
            : `${summary} ${details}`;
        dispatch({ type: "SET_ERROR", error: fullMessage });
      }
    },
    [state.selectedTrackPaths, state.tracks]
  );

  // --- Number Tracks ---

  const handleNumberTracks = useCallback(
    async (rule: OrderingRule) => {
      if (!state.activeAlbumPath) return;
      const albumTracks = state.tracks.filter((t) =>
        t.path.startsWith(state.activeAlbumPath + "/"),
      );
      if (albumTracks.length === 0) return;

      const inputs = albumTracks.map((t) => ({
        path: t.path,
        title: t.title,
        trackNumber: t.trackNumber,
        duration: t.duration,
      }));

      const updates = computeNumberedTracks(inputs, rule);

      // Undo snapshots: save current trackNumber/trackTotal for each track
      const snapshots: TrackSnapshot[] = albumTracks.map((t) => ({
        path: t.path,
        fields: { trackNumber: t.trackNumber, trackTotal: t.trackTotal },
      }));

      dispatch({
        type: "PUSH_UNDO",
        description: `Number tracks (${rule})`,
        snapshots,
      });

      dispatch({ type: "SET_SAVING", saving: true });
      try {
        const results = await window.api.writeTracks(updates);
        dispatch({ type: "UPDATE_TRACKS", tracks: results });
      } catch (err: unknown) {
        const message =
          err instanceof Error ? err.message : "Numbering failed";
        dispatch({ type: "SET_ERROR", error: message });
      } finally {
        dispatch({ type: "SET_SAVING", saving: false });
      }
    },
    [state.activeAlbumPath, state.tracks],
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
        dispatch({ type: "UPDATE_TRACKS", tracks: detail.tracks });
      } catch {
        // Ignore — refresh best-effort
      }
    } else if (state.libraryPath) {
      try {
        const albums = await window.api.scanLibrary(state.libraryPath);
        dispatch({ type: "SET_ALBUMS", albums });
        await loadAlbumTracks(albums);
      } catch {
        // Ignore
      }
    }
  }, [state.activeAlbumPath, state.libraryPath, loadAlbumTracks]);

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
                results: event.results.map(mapAuditResultForState),
              });
            } else if (event.type === "failed") {
              dispatch({ type: "SET_ERROR", error: event.message ?? "Audit failed" });
            }
          });

          const auditResult = await window.api.runAuditOnTracks(trackPaths);
          for (const albumResult of auditResult.albumResults ?? []) {
            dispatch({
              type: "ADD_AUDIT_RESULTS",
              albumPath: albumResult.albumPath,
              results: albumResult.results.map(mapAuditResultForState),
            });
          }
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
        .map(createTrackSnapshot);
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
      if ((e.metaKey || e.ctrlKey) && e.key === "r") {
        e.preventDefault();
        handleRefresh();
      }
      if ((e.metaKey || e.ctrlKey) && e.key === "z" && !e.shiftKey) {
        e.preventDefault();
        handleRevert();
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [handleOpenLibrary, handleAutoTag, handleRefresh, handleRevert]);

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

  // Filter tracks by active album — in-memory filter, no disk reads
  const filteredTracks = useMemo(() => {
    if (!state.activeAlbumPath) return state.tracks;
    return state.tracks.filter((t) =>
      t.path.startsWith(state.activeAlbumPath + "/"),
    );
  }, [state.tracks, state.activeAlbumPath]);

  // Tracks for the currently multi-selected paths
  const selectedTracksForBatch = useMemo(() => {
    const pathSet = new Set(state.selectedTrackPaths);
    return state.tracks.filter((t) => pathSet.has(t.path));
  }, [state.selectedTrackPaths, state.tracks]);

  const visibleAuditResult = useMemo(
    () => getVisibleAuditResult(state.auditResults, state.activeAlbumPath),
    [state.auditResults, state.activeAlbumPath],
  );

  const auditByTrackPath = useMemo(
    () => buildAuditByTrackPath({
      auditResults: state.auditResults,
      tracks: state.tracks,
    }),
    [state.auditResults, state.tracks],
  );

  const selectedTrackAudit = state.selectedTrackPath
    ? auditByTrackPath[state.selectedTrackPath]
    : undefined;

  const handleApplyAuditFixes = useCallback(async (albumResults: AuditApplyAlbumResult[]) => {
    if (albumResults.length === 0) return;

    dispatch({ type: "SET_SAVING", saving: true });
    dispatch({ type: "SET_ERROR", error: null });
    try {
      const summary = await window.api.applyAuditFixes(albumResults);
      for (const albumResult of summary.albumResults) {
        dispatch({
          type: "ADD_AUDIT_RESULTS",
          albumPath: albumResult.albumPath,
          results: albumResult.results.map(mapAuditResultForState),
        });
      }

      const refreshedTracks: TrackData[] = [];
      for (const albumResult of summary.albumResults) {
        try {
          const detail = await window.api.readAlbum(albumResult.albumPath);
          refreshedTracks.push(...detail.tracks);
        } catch {
          // Keep the fix result visible even if a post-write refresh fails.
        }
      }
      if (refreshedTracks.length > 0) {
        dispatch({ type: "UPDATE_TRACKS", tracks: refreshedTracks });
      }
      dispatch({
        type: "SET_ERROR",
        error: summary.fixed > 0 ? `Applied ${summary.fixed} audit fix(es)` : "No eligible audit fixes to apply",
      });
    } catch (err: unknown) {
      const message = err instanceof Error ? err.message : "Failed to apply audit fixes";
      dispatch({ type: "SET_ERROR", error: message });
    } finally {
      dispatch({ type: "SET_SAVING", saving: false });
    }
  }, []);

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

        snapshots.push(createTrackSnapshot(track));

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

  // Handle batch extra tags save with per-track origin-scoped updates
  const handleBatchExtraTagsSave = useCallback(
    async (updates: Array<{ path: string; tags: Array<{ key: string; value: string }> }>) => {
      if (updates.length === 0) return;

      dispatch({ type: "SET_SAVING", saving: true });
      dispatch({ type: "SET_ERROR", error: null });

      try {
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
    [],
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
        onRefresh={handleRefresh}
        onConvert={handleConvert}
        onAutoTag={handleAutoTag}
        onGetLyrics={handleGetLyrics}
        onAudit={handleAudit}
        onNumberTracks={handleNumberTracks}
        activeAlbumPath={state.activeAlbumPath}
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
            tracks={state.tracks}
            activeAlbumPath={state.activeAlbumPath}
            selectedTrackPath={state.selectedTrackPath}
            selectedTrackPaths={state.selectedTrackPaths}
            filterText={state.filterText}
            auditByTrackPath={auditByTrackPath}
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
            <>
              {selectedTrackAudit && (
                <SelectedTrackAuditFindings
                  results={selectedTrackAudit.results}
                  onApplyFixes={() => handleApplyAuditFixes(buildAuditApplyAlbumResults({
                    auditResults: state.auditResults,
                    tracks: state.tracks,
                    trackPath: state.selectedTrackPath,
                  }))}
                  applying={state.saving}
                />
              )}
              <MetadataEditor
                track={state.selectedTrack}
                dirPath={dirPath(state.selectedTrack.path)}
                coverDataUrl={state.coverDataUrl}
                saving={state.saving}
                onSave={handleSaveMetadata}
                onChangeCover={handleChangeCover}
                onRemoveCover={handleRemoveCover}
                onDownloadCover={handleDownloadCover}
                onDownloadArtistArt={handleDownloadArtistArt}
              />
            </>
          ) : visibleAuditResult ? (
            <AuditPanel
              results={visibleAuditResult.results}
              albumName={basename(visibleAuditResult.albumPath) ?? ""}
              onApplyFixes={() => handleApplyAuditFixes(buildAuditApplyAlbumResults({
                auditResults: state.auditResults,
                tracks: state.tracks,
                albumPath: visibleAuditResult.albumPath,
              }))}
              applying={state.saving}
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
        tracks={selectedTracksForBatch.map(toConvertTrack)}
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

/** Build an undo snapshot from a track's current field values. */
function createTrackSnapshot(track: TrackData): TrackSnapshot {
  return {
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
}

export async function buildAutoTagUndoSnapshots(
  targetPaths: string[],
  loadedTracks: TrackData[],
  readAlbum: (albumPath: string) => Promise<AlbumDetail>,
): Promise<TrackSnapshot[]> {
  const snapshots: TrackSnapshot[] = [];
  const seen = new Set<string>();
  const loadedByAlbum = new Map<string, TrackData[]>();

  for (const track of loadedTracks) {
    const albumPath = dirPath(track.path);
    const tracks = loadedByAlbum.get(albumPath) ?? [];
    tracks.push(track);
    loadedByAlbum.set(albumPath, tracks);
  }

  for (const albumPath of targetPaths) {
    let tracks = loadedByAlbum.get(albumPath) ?? [];
    try {
      tracks = (await readAlbum(albumPath)).tracks;
    } catch (err) {
      if (tracks.length === 0) {
        const message = err instanceof Error ? err.message : String(err);
        throw new Error(`Cannot auto-tag without undo snapshot for ${albumPath}: ${message}`);
      }
    }

    if (tracks.length === 0) {
      throw new Error(`Cannot auto-tag without undo snapshot for ${albumPath}: no tracks found`);
    }

    for (const track of tracks) {
      if (seen.has(track.path)) continue;
      seen.add(track.path);
      snapshots.push(createTrackSnapshot(track));
    }
  }

  return snapshots;
}

/** Parse a string as track/disc number, returning null on invalid input. */
function parseNum(s: string): number | null {
  return s ? parseInt(s, 10) || null : null;
}
