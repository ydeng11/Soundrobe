import React, {
  useReducer,
  useCallback,
  useEffect,
  useMemo,
  useRef,
} from "react";
import {
  appReducer,
  buildAuditApplyAlbumResults,
  buildAuditByTrackPath,
  getVisibleAuditResult,
  initialAppState,
  type AuditApplyAlbumResult,
} from "./state/AppState";
import {
  revertHistoryThrough,
  type TrackSnapshot,
} from "./state/UndoManager";
import { TitleBar } from "./components/TitleBar";
import { dirname as dirPath, basename, isInsideDirectory } from "./utils/path";
import { AssistantPanel } from "./components/AssistantPanel";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { Sidebar } from "./components/Sidebar";
import { FileGrid } from "./components/FileGrid";
import { MetadataEditor } from "./components/MetadataEditor";
import { BatchEditor } from "./components/BatchEditor";
import { ScanProgressBar } from "./components/ScanProgressBar";
import { AuditBanner } from "./components/AuditBanner";
import {
  AuditPanel,
  SelectedTrackAuditFindings,
} from "./components/AuditPanel";
import { SettingsModal } from "./components/SettingsModal";
import { ConvertDialog } from "./components/ConvertDialog";
import { SearchDialog } from "./components/SearchDialog";
import { ConfirmWriteDialog } from "./components/ConfirmWriteDialog";
import { ExtraTagsEditor } from "./components/ExtraTagsEditor";
import { BatchExtraTagsEditor } from "./components/BatchExtraTagsEditor";
import type { ConvertResult } from "./components/ConvertDialog";
import {
  parseFilenameWithConvertPattern,
  parseTextWithConvertPattern,
  buildFilenameFromConvertPattern,
  getConvertSourceValue,
  type ConvertTrackData,
} from "./shared/convert";
import { parseDiscField } from "./shared/fields";
import type {
  ExtraTagUndoSnapshot,
  TrackUndoSnapshot,
  TrackData,
  AlbumInfo,
  AlbumDetail,
  AuditRunSummary,
  AuditTrackResult,
  PreviewMatchResult,
  AlbumCandidate,
  ProviderAlbum,
} from "./shared/desktop-api";
import {
  computeNumberedTracks,
  type OrderingRule,
} from "./shared/track-numbering";

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
  const [showSearchDialog, setShowSearchDialog] = React.useState(false);
  const [showConfirmDialog, setShowConfirmDialog] = React.useState(false);
  const [searchPreviewResult, setSearchPreviewResult] = React.useState<PreviewMatchResult | null>(null);
  const [searchWriting, setSearchWriting] = React.useState(false);
  const [searchWriteError, setSearchWriteError] = React.useState<string | null>(null);
  const [extraTagsTrack, setExtraTagsTrack] = React.useState<TrackData | null>(
    null,
  );
  const [batchExtraTagsOpen, setBatchExtraTagsOpen] = React.useState(false);
  const [showAssistant, setShowAssistant] = React.useState(false);
  const [assistantApplying, setAssistantApplying] = React.useState(false);
  const [assistantApiKeyConfigured, setAssistantApiKeyConfigured] = React.useState(false);
  const [assistantModel, setAssistantModel] = React.useState("");

  // Cover URL cache: albumPath → dataUrl | null
  const coverUrlCacheRef = useRef<Map<string, string | null>>(new Map());
  // Abort controller for stale cover responses
  const coverAbortRef = useRef<AbortController | null>(null);
  // Debounce timer for rapid cover navigation
  const coverDebounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  /** Fetch cover data URL with caching and stale-response guarding.
   *
   * `preferredTrackPath` hints at a track known to have embedded cover art so
   * the Rust side can probe just that one file instead of scanning every audio
   * file in the album directory (the main cause of multi-second selection lag).
   */
  const fetchCover = useCallback(
    (albumPath: string, signal?: AbortSignal, preferredTrackPath?: string | null) => {
      const cached = coverUrlCacheRef.current.get(albumPath);
      if (cached !== undefined) {
        console.log(`[select] cover cached — ${albumPath.split("/").pop()}`);
        dispatch({ type: "SET_COVER_URL", url: cached });
        return;
      }

      const ipcStart = performance.now();
      window.api.getCoverDataUrl(albumPath, preferredTrackPath ?? undefined).then(
        (url) => {
          if (signal?.aborted) {
            console.log(`[select] cover aborted (${(performance.now() - ipcStart).toFixed(0)}ms)`);
            return;
          }
          const elapsed = performance.now() - ipcStart;
          coverUrlCacheRef.current.set(albumPath, url);
          dispatch({ type: "SET_COVER_URL", url });
          console.log(`[select] ${elapsed.toFixed(0)}ms — cover ready`);
        },
        () => {
          if (signal?.aborted) return;
          const elapsed = performance.now() - ipcStart;
          coverUrlCacheRef.current.set(albumPath, null);
          dispatch({ type: "SET_COVER_URL", url: null });
          console.log(`[select] ${elapsed.toFixed(0)}ms — no cover`);
        },
      );
    },
    [],
  );

  /** Debounced cover fetch — cancels previous in-flight request.
   *
   * `preferredTrackPath` is forwarded to `fetchCover` so Rust only probes
   * the one known cover-bearing file rather than scanning every audio file.
   */
  const debouncedFetchCover = useCallback(
    (albumPath: string, preferredTrackPath?: string | null) => {
      if (coverDebounceRef.current) {
        clearTimeout(coverDebounceRef.current);
      }
      if (coverAbortRef.current) {
        coverAbortRef.current.abort();
      }
      const abort = new AbortController();
      coverAbortRef.current = abort;

      coverDebounceRef.current = setTimeout(() => {
        fetchCover(albumPath, abort.signal, preferredTrackPath);
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
  const loadAlbumTracks = useCallback(async (albums: AlbumInfo[]) => {
    const trackGroups: TrackData[][] = Array.from(
      { length: albums.length },
      () => [],
    );
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
          progress: {
            current: Math.min(completed + 1, albums.length),
            total: albums.length,
          },
        });
        const tracks = await processAlbum(albums[idx]);
        trackGroups[idx] = tracks;
        completed += 1;
      }
    };

    const workers = Array.from(
      { length: Math.min(concurrency, albums.length) },
      () => worker(),
    );
    await Promise.all(workers);

    dispatch({ type: "SET_SCANNING_PROGRESS", progress: null });
    const allTracks = trackGroups.flat();
    dispatch({ type: "SET_TRACKS", tracks: allTracks });
  }, []);

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
        throw new Error("Tauri desktop bridge is unavailable");
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

  const handleSelectAlbum = useCallback((albumPath: string | null) => {
    // Just update the filter key — tracks are filtered at render time
    dispatch({ type: "SET_ACTIVE_ALBUM", path: albumPath });
  }, []);

  // --- Multi-track selection ---

  const handleMultiSelect = useCallback(
    (paths: string[]) => {
      dispatch({ type: "SET_SELECTED_TRACKS", paths });

      // Still show the primary (first) track's cover art.
      // Use debouncedFetchCover so that when handleSelectTrack follows
      // (single-click), the immediate and debounced calls collapse into
      // one IPC — avoiding a double Rust-side album scan (~500ms).
      if (paths.length > 0) {
        const primary = state.tracks.find((t) => t.path === paths[0]);
        if (primary) {
          debouncedFetchCover(dirPath(primary.path), primary.hasCover ? primary.path : null);
        }
      }
    },
    [state.tracks, debouncedFetchCover],
  );

  // --- Track selection ---

  const handleSelectTrack = useCallback(
    (path: string, track: TrackData) => {
      console.log(`[select] dispatch — ${track.path.split("/").pop()}`);
      dispatch({ type: "SELECT_TRACK", path, track });
      // Pass the clicked track as a preferred hint so Rust probes this one
      // file for embedded cover art instead of scanning every audio file.
      debouncedFetchCover(dirPath(path), track.hasCover ? path : null);
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
          const messages = failed
            .map((r) => `${r.path}: ${r.error}`)
            .join("; ");
          dispatch({
            type: "SET_ERROR",
            error: `Failed to delete ${failed.length} file(s): ${messages}`,
          });
        }

        // Remove deleted paths from state
        const deletedSet = new Set(
          results.filter((r) => r.success).map((r) => r.path),
        );
        const remaining = state.tracks.filter((t) => !deletedSet.has(t.path));
        dispatch({ type: "SET_TRACKS", tracks: remaining });

        // Clear selection if selected files were deleted
        const hadSelected = state.selectedTrackPaths.some((p) =>
          deletedSet.has(p),
        );
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

      // Build write fields
      const writeFields: Record<string, unknown> = {};

      for (const [field, value] of Object.entries(fields)) {
        switch (field) {
          case "track": {
            const parts = value.split("/");
            if (parts[0]) writeFields.trackNumber = parseNum(parts[0]);
            if (parts[1]) writeFields.trackTotal = parseNum(parts[1]);
            break;
          }
          case "disc":
            Object.assign(writeFields, parseDiscField(value));
            break;
          default:
            writeFields[field] = value || null;
        }
      }

      dispatch({ type: "SET_SAVING", saving: true });

      try {
        const snapshot = createOwnedTrackSnapshot(track, writeFields);
        const result = await window.api.writeTrack(track.path, writeFields);

        const changedSnapshots = filterChangedSnapshots([snapshot], [result]);
        if (changedSnapshots.length > 0) {
          dispatch({
            type: "PUSH_UNDO",
            description: "Metadata save",
            snapshots: changedSnapshots,
          });
        }

        // Treat the readback from the API as authoritative
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
        // State was never optimistically updated, so no rollback needed
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
        const previousTags = (await window.api.readExtraTags(
          extraTagsTrack.path,
        )).map(({ key, value }) => ({ key, value }));
        const result = await window.api.writeExtraTags(
          extraTagsTrack.path,
          tags,
        );
        let savedTags: Array<{ key: string; value: string }>;
        try {
          savedTags = (await window.api.readExtraTags(extraTagsTrack.path)).map(
            ({ key, value }) => ({ key, value }),
          );
        } catch (error) {
          if (!extraTagsEqual(previousTags, tags)) {
            dispatch({
              type: "PUSH_UNDO",
              description: "Extra Tags save (unverified)",
              snapshots: [
                {
                  path: extraTagsTrack.path,
                  fields: { [EXTRA_TAG_UNDO_FIELD]: previousTags },
                },
              ],
            });
          }
          throw new Error(
            `Extra Tags were written but readback failed: ${
              error instanceof Error ? error.message : "unknown error"
            }`,
          );
        }
        if (!extraTagsEqual(previousTags, savedTags)) {
          dispatch({
            type: "PUSH_UNDO",
            description: "Extra Tags save",
            snapshots: [
              {
                path: extraTagsTrack.path,
                fields: { [EXTRA_TAG_UNDO_FIELD]: previousTags },
              },
            ],
          });
        }
        dispatch({
          type: "UPDATE_TRACK",
          path: extraTagsTrack.path,
          track: result,
        });
        setExtraTagsTrack(result);
        if (!extraTagsEqual(tags, savedTags)) {
          throw new Error("Extra Tags readback did not match the requested tags");
        }
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
    // Fall back to the first multi-selected track so this button works in batch mode.
    const trackPath = state.selectedTrack?.path ?? state.selectedTrackPaths[0];
    if (!trackPath) return;
    const albumPath = dirPath(trackPath);
    try {
      const url = await window.api.setCover(albumPath);
      if (url) {
        dispatch({ type: "SET_COVER_URL", url });
      }
    } catch {
      dispatch({ type: "SET_ERROR", error: "Failed to set cover art" });
    }
  }, [state.selectedTrack, state.selectedTrackPaths]);

  const handleRemoveCover = useCallback(async () => {
    // Fall back to the first multi-selected track so this button works in batch mode.
    const trackPath = state.selectedTrack?.path ?? state.selectedTrackPaths[0];
    if (!trackPath) return;
    const albumPath = dirPath(trackPath);
    try {
      const removed = await window.api.removeCover(albumPath);
      if (!removed) {
        dispatch({
          type: "SET_ERROR",
          error: "Failed to remove cover art",
        });
        return;
      }
      coverUrlCacheRef.current.set(albumPath, null);
      dispatch({ type: "SET_COVER_URL", url: null });
    } catch {
      dispatch({ type: "SET_ERROR", error: "Failed to remove cover art" });
    }
  }, [state.selectedTrack, state.selectedTrackPaths]);

  const handleDownloadCover = useCallback(async () => {
    // Fall back to the first multi-selected track so this button works in batch mode.
    const trackPath = state.selectedTrack?.path ?? state.selectedTrackPaths[0];
    if (!trackPath) return;
    const albumPath = dirPath(trackPath);
    dispatch({ type: "SET_SAVING", saving: true });
    dispatch({ type: "SET_ERROR", error: null });
    try {
      const dataUrl = await window.api.downloadCoverArt(albumPath);
      if (dataUrl) {
        dispatch({ type: "SET_COVER_URL", url: dataUrl });
        coverUrlCacheRef.current.set(albumPath, dataUrl);
      } else {
        dispatch({
          type: "SET_ERROR",
          error: "No cover art found from any source",
        });
      }
    } catch (err: unknown) {
      const message =
        err instanceof Error ? err.message : "Cover download failed";
      dispatch({ type: "SET_ERROR", error: message });
    } finally {
      dispatch({ type: "SET_SAVING", saving: false });
    }
  }, [state.selectedTrack, state.selectedTrackPaths]);

  const handleDownloadArtistArt = useCallback(async () => {
    // Fall back to the first multi-selected track so this button works in batch mode.
    const trackPath = state.selectedTrack?.path ?? state.selectedTrackPaths[0];
    if (!trackPath) return;
    const albumPath = dirPath(trackPath);
    dispatch({ type: "SET_SAVING", saving: true });
    dispatch({ type: "SET_ERROR", error: null });
    try {
      const result = await window.api.downloadArtistArt(albumPath);
      if (result) {
        dispatch({
          type: "SET_ERROR",
          error: `Artist image saved from ${result.source}`,
        });
      } else {
        dispatch({
          type: "SET_ERROR",
          error: "No artist image found from any source",
        });
      }
    } catch (err: unknown) {
      const message =
        err instanceof Error ? err.message : "Artist image download failed";
      dispatch({ type: "SET_ERROR", error: message });
    } finally {
      dispatch({ type: "SET_SAVING", saving: false });
    }
  }, [state.selectedTrack, state.selectedTrackPaths]);

  // --- Session modification history ---

  const handleRevert = useCallback(
    async (operationId?: number) => {
      if (state.saving || state.reverting) return;
      const history = state.undoManager.history;
      const baseOperationIds = history.map((operation) => operation.id);
      const targetId = operationId ?? history[0]?.id;
      if (targetId === undefined) return;

      const targetIndex = history.findIndex(
        (operation) => operation.id === targetId,
      );
      if (targetIndex < 0) return;
      const commandCount = targetIndex + 1;
      if (
        commandCount > 1 &&
        !window.confirm(
          `Revert ${commandCount} modifications? This will undo the selected modification and every newer one.`,
        )
      ) {
        return;
      }

      dispatch({ type: "SET_REVERTING", reverting: true });
      dispatch({ type: "SET_ERROR", error: null });
      try {
        const result = await revertHistoryThrough(
          state.undoManager,
          targetId,
          async (snapshot) => {
            const remainingFields = { ...snapshot.fields };
            const oldPath =
              typeof remainingFields.path === "string"
                ? remainingFields.path
                : null;

            try {
              if (oldPath && snapshot.path !== oldPath) {
                const track = await window.api.renameTrack(
                  snapshot.path,
                  oldPath,
                );
                dispatch({
                  type: "UPDATE_TRACK",
                  path: snapshot.path,
                  track: { ...track, path: oldPath },
                });
                return null;
              }

              const extraTags = remainingFields[EXTRA_TAG_UNDO_FIELD];
              const standardFields = Object.fromEntries(
                Object.entries(remainingFields).filter(
                  ([key]) => key !== EXTRA_TAG_UNDO_FIELD && key !== "path",
                ),
              );
              if (Object.keys(standardFields).length > 0) {
                const track = await window.api.writeTrack(
                  snapshot.path,
                  standardFields,
                );
                dispatch({
                  type: "UPDATE_TRACK",
                  path: snapshot.path,
                  track,
                });
                for (const key of Object.keys(standardFields)) {
                  delete remainingFields[key];
                }
              }

              if (Array.isArray(extraTags)) {
                const track = await window.api.writeExtraTags(
                  snapshot.path,
                  extraTags as Array<{ key: string; value: string }>,
                );
                const restoredTags = (
                  await window.api.readExtraTags(snapshot.path)
                ).map(({ key, value }) => ({ key, value }));
                if (
                  !extraTagsEqual(
                    extraTags as Array<{ key: string; value: string }>,
                    restoredTags,
                  )
                ) {
                  throw new Error("Extra Tags readback did not match the undo snapshot");
                }
                dispatch({
                  type: "UPDATE_TRACK",
                  path: snapshot.path,
                  track,
                });
                delete remainingFields[EXTRA_TAG_UNDO_FIELD];
              }
              return null;
            } catch (error) {
              return {
                snapshot: { path: snapshot.path, fields: remainingFields },
                error:
                  error instanceof Error ? error.message : "Revert failed",
              };
            }
          },
        );
        dispatch({
          type: "APPLY_UNDO_RESULT",
          undoManager: result.manager,
          baseOperationIds,
        });
        if (result.failures.length > 0) {
          const details = result.failures
            .slice(0, 3)
            .map((failure) => `${failure.path}: ${failure.error}`)
            .join("; ");
          dispatch({
            type: "SET_ERROR",
            error: `Revert stopped after ${result.failures.length} file(s) failed. ${details}`,
          });
        }
      } finally {
        dispatch({ type: "SET_REVERTING", reverting: false });
      }
    },
    [state.reverting, state.saving, state.undoManager],
  );

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
    let snapshots: TrackSnapshot[] = [];
    const attemptedAlbumPaths: string[] = [];
    let autoTagReadback: TrackData[] = [];
    let historyRecorded = false;

    const recordAttemptedAutoTag = async () => {
      if (historyRecorded || attemptedAlbumPaths.length === 0) {
        return autoTagReadback;
      }
      const readbackFailures: string[] = [];
      for (const albumPath of attemptedAlbumPaths) {
        try {
          const detail = await window.api.readAlbum(albumPath);
          autoTagReadback.push(...detail.tracks);
        } catch (error) {
          readbackFailures.push(
            `${albumPath}: ${
              error instanceof Error ? error.message : "readback failed"
            }`,
          );
        }
      }
      if (autoTagReadback.length > 0) {
        dispatch({ type: "UPDATE_TRACKS", tracks: autoTagReadback });
      }
      const attempted = new Set(attemptedAlbumPaths);
      const changedSnapshots = filterChangedSnapshots(
        snapshots.filter((snapshot) => attempted.has(dirPath(snapshot.path))),
        autoTagReadback,
      );
      if (changedSnapshots.length > 0) {
        dispatch({
          type: "PUSH_UNDO",
          description: `Auto-tag (${targetPaths.length} album${targetPaths.length !== 1 ? "s" : ""})`,
          snapshots: changedSnapshots,
        });
      }
      historyRecorded = true;
      if (readbackFailures.length > 0) {
        throw new Error(
          `Auto-tag readback failed for ${readbackFailures
            .slice(0, 3)
            .join("; ")}`,
        );
      }
      return autoTagReadback;
    };

    try {
      snapshots = await buildAutoTagUndoSnapshots(
        targetPaths,
        state.tracks,
        window.api.readAlbum,
      );

      for (const albumPath of targetPaths) {
        const albumName = basename(albumPath) ?? albumPath;
        dispatch({
          type: "SET_AUTO_TAG_PROGRESS",
          progress: isBatch
            ? {
                current: completed,
                total: targetPaths.length,
                message: `${albumName}`,
              }
            : { current: 0, total: 9, message: `Auto-tagging: ${albumName}` },
        });

        const taskId = await window.api.autoTagAlbum(albumPath);
        attemptedAlbumPaths.push(albumPath);
        const unsubscribe = window.api.onAutoTagEvent((event) => {
          if (event.taskId !== taskId) return;
          dispatch({
            type: "SET_AUTO_TAG_PROGRESS",
            progress: isBatch
              ? {
                  current: completed,
                  total: targetPaths.length,
                  message: event.message,
                }
              : {
                  current: event.progress,
                  total: event.total,
                  message: event.message,
                },
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
                ? {
                    current: completed,
                    total: targetPaths.length,
                    message: progress.message,
                  }
                : {
                    current: progress.progress,
                    total: progress.total,
                    message: progress.message,
                  },
            });

            if (
              progress.status === "completed" ||
              progress.status === "failed" ||
              progress.status === "cancelled"
            ) {
              done = true;
              if (progress.status === "failed") {
                totalErrors++;
                console.debug(
                  `[auto-tag] Auto-tag failed for ${albumName}: ${progress.message}`,
                );
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
          ? {
              current: completed,
              total: targetPaths.length,
              message: "Refreshing tracks...",
            }
          : { current: 9, total: 9, message: "Refreshing tracks..." },
      });
      const updatedTrackList = await recordAttemptedAutoTag();
      const scannedAlbums = await window.api.scanLibrary(state.libraryPath);
      dispatch({ type: "SET_ALBUMS", albums: scannedAlbums });

      const taggedAlbumSet = new Set(targetPaths);
      if (state.activeAlbumPath) {
        taggedAlbumSet.add(state.activeAlbumPath);
      }

      for (const albumPath of taggedAlbumSet) {
        coverUrlCacheRef.current.delete(albumPath);
      }
      const visibleAlbumPath =
        state.activeAlbumPath ??
        (state.selectedTrack ? dirPath(state.selectedTrack.path) : null);
      if (visibleAlbumPath && taggedAlbumSet.has(visibleAlbumPath)) {
        const activeTracks = updatedTrackList.filter(
          (track) => dirPath(track.path) === visibleAlbumPath,
        );
        const preferredTrackPath =
          activeTracks.find((track) => track.hasCover)?.path ??
          activeTracks[0]?.path;
        fetchCover(visibleAlbumPath, undefined, preferredTrackPath);
      }

      if (totalErrors > 0) {
        dispatch({
          type: "SET_ERROR",
          error: `Auto-tag completed with ${totalErrors} album(s) with errors`,
        });
      }
    } catch (err: unknown) {
      let message = err instanceof Error ? err.message : "Auto-tag failed";
      try {
        await recordAttemptedAutoTag();
      } catch (readbackError) {
        const detail =
          readbackError instanceof Error
            ? readbackError.message
            : "readback failed";
        message = `${message}; completed changes could not be read back: ${detail}`;
      }
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
    state.selectedTrack,
    state.autoTagging,
    fetchCover,
    loadAlbumTracks,
  ]);

  // --- Audit: LLM-based metadata verification against file paths ---

  const handleAudit = useCallback(async () => {
    if (!state.libraryPath || state.auditing) {
      console.log(
        "[audit] handleAudit skipped — libraryPath=%s auditing=%s",
        state.libraryPath,
        state.auditing,
      );
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
        console.log(
          "[audit] event received — type=%s msg=%s",
          event.type,
          event.message ?? "",
        );
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
            dispatch({
              type: "SET_ERROR",
              error: event.message ?? "Audit failed",
            });
            break;

          case "cancelled":
            break;
        }
      });

      // Determine scope: selected tracks → active album → entire library
      let auditResult: AuditRunSummary;
      if (state.selectedTrackPaths.length > 0) {
        auditResult = await window.api.runAuditOnTracks(
          state.selectedTrackPaths,
        );
      } else if (state.activeAlbumPath) {
        auditResult = await window.api.runAuditOnAlbums([
          state.activeAlbumPath,
        ]);
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
  }, [
    state.libraryPath,
    state.selectedTrackPaths,
    state.activeAlbumPath,
    state.auditing,
    loadAlbumTracks,
  ]);

  // --- Get Lyrics ---

  const handleGetLyrics = useCallback(async () => {
    if (!state.libraryPath || state.lyricsGetting) return;

    const targetPaths = state.activeAlbumPath
      ? [state.activeAlbumPath]
      : state.albums.map((a) => a.path);

    if (targetPaths.length === 0) return;

    dispatch({ type: "SET_LYRICS_GETTING", lyricsGetting: true });

    try {
      const totals = {
        written: 0,
        embeddedPreserved: 0,
        noLyrics: 0,
        unsupported: 0,
        failed: 0,
      };
      for (const albumPath of targetPaths) {
        const report = await window.api.downloadAlbumLyrics(albumPath);
        totals.written += report.written;
        totals.embeddedPreserved += report.embeddedPreserved;
        totals.noLyrics += report.noLyrics;
        totals.unsupported += report.unsupported;
        totals.failed += report.failed;
      }

      if (totals.written > 0) {
        // Refresh the active album to show new lyrics in sidebar
        if (state.activeAlbumPath) {
          const detail = await window.api.readAlbum(state.activeAlbumPath);
          dispatch({ type: "SET_TRACKS", tracks: detail.tracks });
        }
      }

      dispatch({
        type: "SET_ERROR",
        error:
          `Lyrics: ${totals.written} embedded, ${totals.embeddedPreserved} preserved, ` +
          `${totals.noLyrics} unavailable, ${totals.unsupported} unsupported, ${totals.failed} failed`,
      });
    } catch (err: unknown) {
      const message =
        err instanceof Error ? err.message : "Failed to get lyrics";
      dispatch({ type: "SET_ERROR", error: message });
    } finally {
      dispatch({ type: "SET_LYRICS_GETTING", lyricsGetting: false });
    }
  }, [
    state.libraryPath,
    state.activeAlbumPath,
    state.albums,
    state.lyricsGetting,
  ]);

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
                  result.sourceTag ?? "title",
                );

          if (result.direction === "tag-to-tags" && !sourceValue.trim()) {
            errors.push(
              `${filename}: ${result.sourceTag ?? "title"} tag is empty`,
            );
            continue;
          }

          const parsed =
            result.direction === "filename-to-tags"
              ? parseFilenameWithConvertPattern(result.pattern, filename)
              : parseTextWithConvertPattern(result.pattern, sourceValue);

          if ("error" in parsed) {
            errors.push(`${filename}: ${parsed.error}`);
            continue;
          }

          const parsedFields = parsed.fields as Record<string, string>;
          const writeFields: Record<string, unknown> = {};
          for (const [key, value] of Object.entries(parsedFields)) {
            if (key === "track") {
              writeFields.trackNumber = parseNum(value);
            } else if (key === "disc") {
              writeFields.discNumber = parseNum(value);
            } else {
              writeFields[key] = value;
            }
          }
          if (Object.keys(writeFields).length === 0) {
            errors.push(`${filename}: No fields extracted`);
            continue;
          }

          const snapshot = createOwnedTrackSnapshot(track, writeFields);

          try {
            const apiResult = await window.api.writeTrack(
              track.path,
              writeFields,
            );
            dispatch({
              type: "UPDATE_TRACK",
              path: track.path,
              track: apiResult,
            });
            if (filterChangedSnapshots([snapshot], [apiResult]).length > 0) {
              undoSnapshots.push(snapshot);
            }
            successes.push(filename);
          } catch (err: unknown) {
            errors.push(
              `${filename}: ${
                err instanceof Error ? err.message : "write failed"
              }`,
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
            convertTrack,
          );

          if (!newFilename || newFilename === filename) {
            errors.push(`${filename}: no change needed`);
            continue;
          }

          const oldDir = track.path.substring(
            0,
            track.path.lastIndexOf("/") + 1,
          );
          const newPath = oldDir + newFilename;

          // Check if target exists
          try {
            const exists = await window.api.checkFileExists(newPath);
            if (exists) {
              // If same path (e.g. after prev rename), skip
              if (newPath !== track.path) {
                errors.push(
                  `${filename}: target already exists (${newFilename})`,
                );
                continue;
              }
            }
          } catch {
            // Ignore check errors
          }

          try {
            const updatedTrack = await window.api.renameTrack(
              track.path,
              newPath,
            );
            dispatch({
              type: "UPDATE_TRACK",
              path: track.path,
              track: { ...updatedTrack, path: newPath },
            });
            undoSnapshots.push({
              path: newPath,
              fields: { path: track.path },
            });
            successes.push(`${filename} → ${newFilename}`);
          } catch (err: unknown) {
            errors.push(
              `${filename}: ${
                err instanceof Error ? err.message : "rename failed"
              }`,
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
                t.path.substring(0, t.path.lastIndexOf("/")),
              ),
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
    [state.selectedTrackPaths, state.tracks],
  );

  // --- Number Tracks ---

  const handleNumberTracks = useCallback(
    async (rule: OrderingRule) => {
      const activeAlbumPath = state.activeAlbumPath;
      if (!activeAlbumPath) return;
      const albumTracks = state.tracks.filter(
        (t) => isInsideDirectory(t.path, activeAlbumPath),
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

      dispatch({ type: "SET_SAVING", saving: true });
      try {
        const result = await window.api.writeTracks(updates);

        const successSnapshots = filterChangedSnapshots(
          snapshots,
          result.tracks,
        );
        if (successSnapshots.length > 0) {
          dispatch({
            type: "PUSH_UNDO",
            description: `Number tracks (${rule})`,
            snapshots: successSnapshots,
          });
        }

        dispatch({ type: "UPDATE_TRACKS", tracks: result.tracks });
        if (result.failures.length > 0) {
          const errorMsg = result.failures
            .slice(0, 3)
            .map((f) => `${f.path}: ${f.error}`)
            .join("; ");
          const suffix =
            result.failures.length > 3
              ? ` (and ${result.failures.length - 3} more)`
              : "";
          dispatch({
            type: "SET_ERROR",
            error: `Numbering: ${result.failures.length} file(s) failed. ${errorMsg}${suffix}`,
          });
        }
      } catch (err: unknown) {
        const message = err instanceof Error ? err.message : "Numbering failed";
        dispatch({ type: "SET_ERROR", error: message });
      } finally {
        dispatch({ type: "SET_SAVING", saving: false });
      }
    },
    [state.activeAlbumPath, state.tracks],
  );

  // --- Manual Search ---

  const handleSearch = useCallback(() => {
    if (state.autoTagging || state.saving || !state.activeAlbumPath) return;
    setShowSearchDialog(true);
  }, [state.autoTagging, state.saving, state.activeAlbumPath]);

  const handleCloseSearch = useCallback(() => {
    setShowSearchDialog(false);
  }, []);

  const handleSelectRelease = useCallback(
    async (release: ProviderAlbum, provider: string) => {
      setShowSearchDialog(false);
      setShowConfirmDialog(true);
      setSearchPreviewResult(null);
      setSearchWriteError(null);
      try {
        const result = await window.api.previewReleaseMatch({
          albumPath: state.activeAlbumPath!,
          release,
          provider,
        });
        setSearchPreviewResult(result);
      } catch (err) {
        setSearchWriteError(err instanceof Error ? err.message : String(err));
        setShowConfirmDialog(true);
      }
    },
    [state.activeAlbumPath],
  );

  const handleConfirmWrite = useCallback(
    async (candidate: AlbumCandidate, selectedTrackIndices: number[]) => {
      const activeAlbumPath = state.activeAlbumPath;
      if (!activeAlbumPath) return;
      setSearchWriting(true);
      setSearchWriteError(null);
      let historyRecorded = false;
      let snapshots: TrackSnapshot[] = [];

      try {
        // Capture undo snapshots (mirrors handleAutoTag)
        const albumTracks = state.tracks.filter(
          (t) => isInsideDirectory(t.path, activeAlbumPath),
        );
        snapshots = await buildAutoTagUndoSnapshots(
          [activeAlbumPath],
          albumTracks,
          window.api.readAlbum,
        );
        const recordChangedSearchTracks = async () => {
          if (historyRecorded) return;
          const readback = await window.api.readAlbum(activeAlbumPath);
          const changedSnapshots = filterChangedSnapshots(
            snapshots,
            readback.tracks,
          );
          if (changedSnapshots.length > 0) {
            dispatch({
              type: "PUSH_UNDO",
              description: "Manual search tag",
              snapshots: changedSnapshots,
            });
            historyRecorded = true;
          }
        };
        const written = await window.api.searchApplyCandidate(
          activeAlbumPath,
          candidate,
          selectedTrackIndices,
        );
        if (written > 0) {
          await recordChangedSearchTracks();
          await handleRefresh();
        }
        setShowConfirmDialog(false);
      } catch (err) {
        if (!historyRecorded) {
          try {
            const readback = await window.api.readAlbum(activeAlbumPath);
            const changedSnapshots = filterChangedSnapshots(
              snapshots,
              readback.tracks,
            );
            if (changedSnapshots.length > 0) {
              dispatch({
                type: "PUSH_UNDO",
                description: "Manual search tag (partial)",
                snapshots: changedSnapshots,
              });
            }
          } catch {
            // The original write error remains authoritative.
          }
        }
        setSearchWriteError(err instanceof Error ? err.message : String(err));
      } finally {
        setSearchWriting(false);
      }
    },
    [state.activeAlbumPath, state.tracks, handleRefresh],
  );

  const handleCancelConfirm = useCallback(() => {
    setShowConfirmDialog(false);
    setSearchPreviewResult(null);
    setSearchWriteError(null);
  }, []);

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

  // Track whether an LLM API key is configured for the assistant UI state.
  // The actual key is resolved server-side from ConfigState (env/config file)
  // and is never sent through the renderer (getConfig() returns a masked copy).
  useEffect(() => {
    window.api.getConfig().then(
      (cfg) => {
        const configured = (cfg.llmApiKeyConfigured as boolean) ?? false;
        const model = (cfg.llmModel as string) ?? "";
        setAssistantApiKeyConfigured(configured);
        setAssistantModel(model);
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
    async (
      description: string,
      snapshots: TrackUndoSnapshot[],
      extraSnapshots: ExtraTagUndoSnapshot[],
      preserveUnverified: boolean,
    ) => {
      const standardCandidates = snapshots.map((snapshot) => ({
        path: snapshot.path,
        fields: { ...snapshot.metadata },
      }));
      const changedStandardSnapshots: TrackSnapshot[] = [];
      for (const albumPath of new Set(
        standardCandidates.map((snapshot) => dirPath(snapshot.path)),
      )) {
        const albumCandidates = standardCandidates.filter(
          (snapshot) => dirPath(snapshot.path) === albumPath,
        );
        try {
          const detail = await window.api.readAlbum(albumPath);
          changedStandardSnapshots.push(
            ...filterChangedSnapshots(albumCandidates, detail.tracks),
          );
        } catch {
          if (preserveUnverified) {
            changedStandardSnapshots.push(...albumCandidates);
          }
        }
      }

      const snapshotsByPath = new Map<string, TrackSnapshot>();
      for (const snapshot of changedStandardSnapshots) {
        snapshotsByPath.set(snapshot.path, {
          path: snapshot.path,
          fields: { ...snapshot.fields },
        });
      }
      for (const snapshot of extraSnapshots) {
        let currentTags: Array<{ key: string; value: string }>;
        let readVerified = true;
        try {
          currentTags = (await window.api.readExtraTags(snapshot.path)).map(
            ({ key, value }) => ({ key, value }),
          );
        } catch {
          readVerified = false;
          if (!preserveUnverified) continue;
          currentTags = [];
        }
        if (readVerified && extraTagsEqual(snapshot.extraTags, currentTags)) {
          continue;
        }
        const existing = snapshotsByPath.get(snapshot.path);
        snapshotsByPath.set(snapshot.path, {
          path: snapshot.path,
          fields: {
            ...existing?.fields,
            [EXTRA_TAG_UNDO_FIELD]: snapshot.extraTags,
          },
        });
      }
      const trackSnapshots = Array.from(snapshotsByPath.values()).filter(
        (snapshot) => Object.keys(snapshot.fields).length > 0,
      );
      dispatch({ type: "PUSH_UNDO", description, snapshots: trackSnapshots });
    },
    [],
  );

  const handleAssistantRunTask = useCallback(
    async (task: "auto_tag" | "audit", trackPaths: string[]) => {
      if (!state.libraryPath || trackPaths.length === 0) {
        throw new Error("No library tracks are available for the assistant task");
      }

      if (task === "audit") {
        if (state.auditing) {
          throw new Error("An audit is already running");
        }

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
            } else if (
              event.type === "album-result" &&
              event.albumPath &&
              event.results
            ) {
              dispatch({
                type: "ADD_AUDIT_RESULTS",
                albumPath: event.albumPath,
                results: event.results.map(mapAuditResultForState),
              });
            } else if (event.type === "failed") {
              dispatch({
                type: "SET_ERROR",
                error: event.message ?? "Audit failed",
              });
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
          throw err;
        } finally {
          if (unsubscribe) unsubscribe();
          dispatch({ type: "SET_AUDITING", auditing: false });
          dispatch({ type: "SET_AUDIT_PROGRESS", progress: null });
        }
        return;
      }

      if (state.autoTagging) {
        throw new Error("Auto-tagging is already running");
      }

      const albumPaths = Array.from(new Set(trackPaths.map(dirPath)));
      const snapshots = await buildAutoTagUndoSnapshots(
        albumPaths,
        state.tracks,
        window.api.readAlbum,
      );
      const attemptedAlbumPaths: string[] = [];
      let historyRecorded = false;
      const recordAttemptedAutoTag = async () => {
        if (historyRecorded || attemptedAlbumPaths.length === 0) return;
        const attempted = new Set(attemptedAlbumPaths);
        const readbacks: TrackData[] = [];
        const readbackFailures: string[] = [];
        for (const albumPath of attemptedAlbumPaths) {
          try {
            const detail = await window.api.readAlbum(albumPath);
            readbacks.push(...detail.tracks);
          } catch (error) {
            readbackFailures.push(
              `${albumPath}: ${
                error instanceof Error ? error.message : "readback failed"
              }`,
            );
          }
        }
        if (readbacks.length > 0) {
          dispatch({ type: "UPDATE_TRACKS", tracks: readbacks });
        }
        const changedSnapshots = filterChangedSnapshots(
          snapshots.filter((snapshot) => attempted.has(dirPath(snapshot.path))),
          readbacks,
        );
        if (changedSnapshots.length > 0) {
          dispatch({
            type: "PUSH_UNDO",
            description: `Assistant auto-tag (${albumPaths.length} album${albumPaths.length !== 1 ? "s" : ""})`,
            snapshots: changedSnapshots,
          });
        }
        historyRecorded = true;
        if (readbackFailures.length > 0) {
          throw new Error(
            `Assistant auto-tag readback failed for ${readbackFailures
              .slice(0, 3)
              .join("; ")}`,
          );
        }
      };
      dispatch({ type: "SET_AUTO_TAGGING", autoTagging: true });
      dispatch({ type: "SET_ERROR", error: null });

      try {
        let completed = 0;
        for (const albumPath of albumPaths) {
          attemptedAlbumPaths.push(albumPath);
          const taskId = await window.api.autoTagAlbum(albumPath);
          let done = false;
          while (!done) {
            const progress = await window.api.getTaskProgress(taskId);
            if (!progress) {
              throw new Error(`Auto-tag task progress disappeared: ${taskId}`);
            }

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
              if (progress.status !== "completed") {
                throw new Error(progress.message || `Auto-tag ${progress.status}`);
              }
              done = true;
            } else {
              await new Promise((resolve) => setTimeout(resolve, 300));
            }
          }
          completed++;
        }
        await recordAttemptedAutoTag();
        await handleAssistantRefresh();
      } catch (err: unknown) {
        try {
          await recordAttemptedAutoTag();
        } catch (readbackError) {
          const detail =
            readbackError instanceof Error
              ? readbackError.message
              : "readback failed";
          dispatch({
            type: "SET_ERROR",
            error: `Assistant auto-tag failed and completed changes could not be read back: ${detail}`,
          });
          throw err;
        }
        const message = err instanceof Error ? err.message : "Auto-tag failed";
        dispatch({ type: "SET_ERROR", error: message });
        throw err;
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
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [handleOpenLibrary, handleAutoTag, handleRefresh]);

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
    const activeAlbumPath = state.activeAlbumPath;
    if (!activeAlbumPath) return state.tracks;
    return state.tracks.filter((t) =>
      isInsideDirectory(t.path, activeAlbumPath),
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
    () =>
      buildAuditByTrackPath({
        auditResults: state.auditResults,
        tracks: state.tracks,
      }),
    [state.auditResults, state.tracks],
  );

  const selectedTrackAudit = state.selectedTrackPath
    ? auditByTrackPath[state.selectedTrackPath]
    : undefined;

  /** Build a single-fix payload from an inline audit result entry. */
  function singleAuditFixResult(
    result: {
      trackIndex: number;
      field: string;
      status: string;
      message: string | null;
      suggestion?: string | null;
      corrected?: Record<string, unknown> | null;
      source?: string;
      confidence?: number;
    },
    albumPath: string,
  ): AuditApplyAlbumResult[] {
    const trackResult: AuditTrackResult = {
      index: result.trackIndex,
      field: result.field,
      status: result.status as AuditTrackResult["status"],
      message: result.message ?? "",
      suggestion: result.suggestion,
      corrected: result.corrected as AuditTrackResult["corrected"],
      source: (result.source ?? "deterministic") as AuditTrackResult["source"],
      confidence: result.confidence ?? 0,
      autoFixEligible: true,
      autoFixed: false,
    };
    return [{ albumPath, results: [trackResult] }];
  }

  const handleApplyAuditFixes = useCallback(
    async (albumResults: AuditApplyAlbumResult[]) => {
      if (albumResults.length === 0 || state.reverting) return;

      dispatch({ type: "SET_SAVING", saving: true });
      dispatch({ type: "SET_ERROR", error: null });
      const auditSnapshots: TrackSnapshot[] = [];
      const auditTracksByAlbum = new Map<string, TrackData[]>();
      let writesStarted = false;
      let historyRecorded = false;
      try {
        for (const albumResult of albumResults) {
          const detail = await window.api.readAlbum(albumResult.albumPath);
          auditTracksByAlbum.set(albumResult.albumPath, detail.tracks);
          const fieldsByTrack = new Map<number, Record<string, unknown>>();
          for (const result of albumResult.results) {
            if (!result.autoFixEligible) continue;
            const fields = fieldsByTrack.get(result.index) ?? {};
            if (result.corrected) {
              for (const [key, value] of Object.entries(result.corrected)) {
                if (value !== undefined) fields[key] = value;
              }
            } else {
              const key = normalizeAuditField(result.field);
              if (key) fields[key] = result.suggestion ?? null;
            }
            fieldsByTrack.set(result.index, fields);
          }
          for (const [trackIndex, fields] of fieldsByTrack) {
            const track = detail.tracks[trackIndex];
            const current = track as unknown as Record<string, unknown>;
            const wouldChange = Object.entries(fields).some(
              ([key, value]) => !valuesEqual(current?.[key] ?? null, value),
            );
            if (track && wouldChange) {
              auditSnapshots.push(createOwnedTrackSnapshot(track, fields));
            }
          }
        }

        writesStarted = true;
        const summary = await window.api.applyAuditFixes(albumResults);
        for (const albumResult of summary.albumResults) {
          dispatch({
            type: "ADD_AUDIT_RESULTS",
            albumPath: albumResult.albumPath,
            results: albumResult.results.map(mapAuditResultForState),
          });
        }

        const refreshedTracks: TrackData[] = [];
        const refreshFailures: string[] = [];
        for (const albumResult of summary.albumResults) {
          try {
            const detail = await window.api.readAlbum(albumResult.albumPath);
            refreshedTracks.push(...detail.tracks);
          } catch (error) {
            refreshFailures.push(
              `${albumResult.albumPath}: ${
                error instanceof Error ? error.message : "readback failed"
              }`,
            );
          }
        }
        if (refreshedTracks.length > 0) {
          dispatch({ type: "UPDATE_TRACKS", tracks: refreshedTracks });
        }
        if (summary.fixed > 0) {
          const successfulPaths = new Set<string>();
          for (const albumResult of summary.albumResults) {
            const tracks = auditTracksByAlbum.get(albumResult.albumPath) ?? [];
            for (const result of albumResult.results) {
              if (result.autoFixed && tracks[result.index]) {
                successfulPaths.add(tracks[result.index].path);
              }
            }
          }
          const readbackPaths = new Set(
            refreshedTracks.map((track) => track.path),
          );
          const changedSnapshots = [
            ...filterChangedSnapshots(auditSnapshots, refreshedTracks),
            ...auditSnapshots.filter(
              (snapshot) =>
                successfulPaths.has(snapshot.path) &&
                !readbackPaths.has(snapshot.path),
            ),
          ].filter(
            (snapshot, index, all) =>
              all.findIndex((candidate) => candidate.path === snapshot.path) ===
              index,
          );
          if (changedSnapshots.length > 0) {
            dispatch({
              type: "PUSH_UNDO",
              description: `Audit fix (${summary.fixed})`,
              snapshots: changedSnapshots,
            });
            historyRecorded = true;
          }
        }
        dispatch({
          type: "SET_ERROR",
          error:
            refreshFailures.length > 0
              ? `Applied ${summary.fixed} audit fix(es), but readback failed: ${refreshFailures.slice(0, 3).join("; ")}`
              : summary.fixed > 0
              ? `Applied ${summary.fixed} audit fix(es)`
              : "No eligible audit fixes to apply",
        });
      } catch (err: unknown) {
        if (writesStarted && !historyRecorded && auditSnapshots.length > 0) {
          try {
            const readbacks: TrackData[] = [];
            for (const albumPath of new Set(
              albumResults.map((result) => result.albumPath),
            )) {
              const detail = await window.api.readAlbum(albumPath);
              readbacks.push(...detail.tracks);
            }
            const changedSnapshots = filterChangedSnapshots(
              auditSnapshots,
              readbacks,
            );
            if (changedSnapshots.length > 0) {
              dispatch({
                type: "PUSH_UNDO",
                description: "Audit fix (partial)",
                snapshots: changedSnapshots,
              });
            }
          } catch {
            // The original apply error remains authoritative.
          }
        }
        const message =
          err instanceof Error ? err.message : "Failed to apply audit fixes";
        dispatch({ type: "SET_ERROR", error: message });
      } finally {
        dispatch({ type: "SET_SAVING", saving: false });
      }
    },
    [state.reverting],
  );

  // Handle batch field save from BatchEditor — single IPC call
  const handleBatchSave = useCallback(
    async (fields: Record<string, string>) => {
      const paths = state.selectedTrackPaths;
      if (paths.length === 0) return;

      const snapshots: TrackSnapshot[] = [];
      const updates: Array<{ path: string; fields: Record<string, unknown> }> =
        [];

      // Build write fields once — same for all selected tracks.
      // The field keys from BatchEditor are:
      //   artist, album, albumArtist, genre, year, disc
      // Most map 1:1 to TrackPatch serde keys, but "disc" needs splitting
      // into discNumber and discTotal (same pattern as handleSaveMetadata).
      const writeFields: Record<string, unknown> = {};
      for (const [key, value] of Object.entries(fields)) {
        switch (key) {
          case "disc":
            Object.assign(writeFields, parseDiscField(value));
            break;
          default:
            writeFields[key] = value || null;
        }
      }

      for (const path of paths) {
        const track = state.tracks.find((t) => t.path === path);
        if (!track) continue;

        snapshots.push(createOwnedTrackSnapshot(track, writeFields));
        updates.push({ path, fields: writeFields });
      }

      dispatch({ type: "SET_SAVING", saving: true });

      // Subscribe to real-time progress from the Rust batch writer
      const unsubProgress = window.api.onTrackWriteEvent((event) => {
        dispatch({
          type: "SET_SAVE_PROGRESS",
          progress: { current: event.current, total: event.total },
        });
      });

      try {
        const result = await window.api.writeTracks(updates);

        const successSnapshots = filterChangedSnapshots(
          snapshots,
          result.tracks,
        );
        if (successSnapshots.length > 0) {
          dispatch({
            type: "PUSH_UNDO",
            description: "Batch edit",
            snapshots: successSnapshots,
          });
        }

        // Treat the readback from the API as authoritative
        dispatch({ type: "UPDATE_TRACKS", tracks: result.tracks });

        // Report per-track failures so the user knows which files failed
        if (result.failures.length > 0) {
          const errorMsg = result.failures
            .slice(0, 3)
            .map((f) => `${f.path}: ${f.error}`)
            .join("; ");
          const suffix =
            result.failures.length > 3
              ? ` (and ${result.failures.length - 3} more)`
              : "";
          dispatch({
            type: "SET_ERROR",
            error: `Batch save: ${result.failures.length} file(s) failed. ${errorMsg}${suffix}`,
          });
        }
      } catch (err: unknown) {
        // State was never optimistically updated, so no rollback needed
        const message =
          err instanceof Error ? err.message : "Batch save failed";
        dispatch({ type: "SET_ERROR", error: message });
      } finally {
        unsubProgress();
        dispatch({ type: "SET_SAVE_PROGRESS", progress: null });
        dispatch({ type: "SET_SAVING", saving: false });
      }
    },
    [state.selectedTrackPaths, state.tracks],
  );

  // Handle batch extra tags save with per-track origin-scoped updates
  const handleBatchExtraTagsSave = useCallback(
    async (
      updates: Array<{
        path: string;
        tags: Array<{ key: string; value: string }>;
      }>,
    ) => {
      if (updates.length === 0) return;

      dispatch({ type: "SET_SAVING", saving: true });
      dispatch({ type: "SET_ERROR", error: null });

      try {
        const previousTags = new Map(
          await Promise.all(
            updates.map(async (update) => [
              update.path,
              (await window.api.readExtraTags(update.path)).map(
                ({ key, value }) => ({ key, value }),
              ),
            ] as const),
          ),
        );
        let results: TrackData[] = [];
        let writeError: unknown = null;
        try {
          results = await window.api.writeExtraTagsBatch(updates);
        } catch (error) {
          writeError = error;
        }

        const snapshots: TrackSnapshot[] = [];
        const readbackFailures: string[] = [];
        const verificationFailures: string[] = [];
        const successfulPaths = new Set(results.map((track) => track.path));
        const writeErrorMessage =
          writeError instanceof Error ? writeError.message : String(writeError ?? "");
        const failedWritePaths = new Set(
          updates
            .filter((update) => writeErrorMessage.includes(`${update.path}:`))
            .map((update) => update.path),
        );
        for (const update of updates) {
          const before = previousTags.get(update.path)!;
          try {
            const after = (await window.api.readExtraTags(update.path)).map(
              ({ key, value }) => ({ key, value }),
            );
            const changed = !extraTagsEqual(before, after);
            if (changed) {
              snapshots.push({
                path: update.path,
                fields: { [EXTRA_TAG_UNDO_FIELD]: before },
              });
            }
            if ((!writeError || changed) && !extraTagsEqual(update.tags, after)) {
              verificationFailures.push(update.path);
            }
          } catch (error) {
            readbackFailures.push(
              `${update.path}: ${
                error instanceof Error ? error.message : "readback failed"
              }`,
            );
            if (
              (successfulPaths.has(update.path) ||
                (writeError && !failedWritePaths.has(update.path))) &&
              !extraTagsEqual(before, update.tags)
            ) {
              snapshots.push({
                path: update.path,
                fields: { [EXTRA_TAG_UNDO_FIELD]: before },
              });
            }
          }
        }
        if (snapshots.length > 0) {
          dispatch({
            type: "PUSH_UNDO",
            description: writeError
              ? "Batch Extra Tags save (partial)"
              : "Batch Extra Tags save",
            snapshots,
          });
        }
        if (results.length > 0) {
          dispatch({ type: "UPDATE_TRACKS", tracks: results });
        } else if (writeError && snapshots.length > 0) {
          for (const albumPath of new Set(updates.map((update) => dirPath(update.path)))) {
            try {
              const detail = await window.api.readAlbum(albumPath);
              dispatch({ type: "UPDATE_TRACKS", tracks: detail.tracks });
            } catch {
              // The batch error below remains the useful failure to surface.
            }
          }
        }
        if (writeError) throw writeError;
        if (verificationFailures.length > 0) {
          throw new Error(
            `Extra Tags readback did not match the requested tags for: ${verificationFailures
              .slice(0, 3)
              .join("; ")}`,
          );
        }
        if (readbackFailures.length > 0) {
          throw new Error(
            `Extra Tags were written but readback failed: ${readbackFailures
              .slice(0, 3)
              .join("; ")}`,
          );
        }
        setBatchExtraTagsOpen(false);
      } catch (err: unknown) {
        const message =
          err instanceof Error ? err.message : "Batch extra tags save failed";
        dispatch({ type: "SET_ERROR", error: message });
      } finally {
        dispatch({ type: "SET_SAVING", saving: false });
      }
    },
    [],
  );

  const mutationBusy = state.saving || state.reverting || assistantApplying;

  return (
    <div className="flex flex-col h-screen bg-surface text-text-primary overflow-hidden">
      <TitleBar
        libraryPath={state.libraryPath}
        trackCount={filteredTracks.length}
        filterText={state.filterText}
        onFilterChange={handleFilterChange}
        selectedFilePath={state.selectedTrackPath}
        saving={mutationBusy}
        autoTagging={state.autoTagging}
        lyricsGetting={state.lyricsGetting}
        auditing={state.auditing}
        error={state.error}
        modificationHistory={state.undoManager.history}
        reverting={state.reverting}
        onOpenLibrary={handleOpenLibrary}
        onRefresh={handleRefresh}
        onConvert={handleConvert}
        onAutoTag={handleAutoTag}
        onSearch={handleSearch}
        onGetLyrics={handleGetLyrics}
        onAudit={handleAudit}
        onNumberTracks={handleNumberTracks}
        activeAlbumPath={state.activeAlbumPath}
        darkMode={state.darkMode}
        assistantOpen={showAssistant}
        onToggleDarkMode={handleToggleDarkMode}
        onOpenSettings={handleOpenSettings}
        onToggleAssistant={handleToggleAssistant}
        onErrorDismiss={() => dispatch({ type: "SET_ERROR", error: null })}
        onUndoLatest={() => handleRevert()}
        onUndoThrough={handleRevert}
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

      <ScanProgressBar
        scanning={state.saving}
        progress={state.saveProgress}
        label={state.saving ? "Saving tracks…" : null}
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
              saving={mutationBusy}
              onSave={handleBatchSave}
              onChangeCover={handleChangeCover}
              onRemoveCover={handleRemoveCover}
              onDownloadCover={handleDownloadCover}
              onDownloadArtistArt={handleDownloadArtistArt}
            />
          ) : state.selectedTrack ? (
            <>
              {selectedTrackAudit && (
                <SelectedTrackAuditFindings
                  results={selectedTrackAudit.results}
                  onApplyFixes={() =>
                    handleApplyAuditFixes(
                      buildAuditApplyAlbumResults({
                        auditResults: state.auditResults,
                        tracks: state.tracks,
                        trackPath: state.selectedTrackPath,
                      }),
                    )
                  }
                  onApplyFix={(result) =>
                    handleApplyAuditFixes(
                      singleAuditFixResult(
                        result,
                        dirPath(state.selectedTrackPath!),
                      ),
                    )
                  }
                  applying={mutationBusy}
                />
              )}
              <MetadataEditor
                track={state.selectedTrack}
                dirPath={dirPath(state.selectedTrack.path)}
                coverDataUrl={state.coverDataUrl}
                saving={mutationBusy}
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
              onApplyFixes={() =>
                handleApplyAuditFixes(
                  buildAuditApplyAlbumResults({
                    auditResults: state.auditResults,
                    tracks: state.tracks,
                    albumPath: visibleAuditResult.albumPath,
                  }),
                )
              }
              onApplyFix={(result) =>
                handleApplyAuditFixes(
                  singleAuditFixResult(
                    result,
                    visibleAuditResult.albumPath,
                  ),
                )
              }
              applying={mutationBusy}
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
          onOpenSettings={handleOpenSettings}
          keyConfigured={assistantApiKeyConfigured}
          model={assistantModel}
          libraryPath={state.libraryPath}
          activeAlbumPath={state.activeAlbumPath}
          selectedTrackPaths={state.selectedTrackPaths}
          allTracks={state.tracks}
          allAlbums={state.albums}
          autonomous={false}
          mutationsDisabled={state.reverting}
          onApplyingChange={setAssistantApplying}
          onRefreshRequest={handleAssistantRefresh}
          onAssistantRunTask={handleAssistantRunTask}
          onAssistantApplyUndo={handleAssistantApplyUndo}
        />
      </ErrorBoundary>

      <SettingsModal open={state.showSettings} onClose={handleCloseSettings} />

      <ConvertDialog
        open={showConvertDialog}
        onClose={() => setShowConvertDialog(false)}
        onConvert={handleConvertAction}
        tracks={selectedTracksForBatch.map(toConvertTrack)}
      />

      <SearchDialog
        open={showSearchDialog}
        albumPath={state.activeAlbumPath ?? ""}
        onClose={handleCloseSearch}
        onSelectRelease={handleSelectRelease}
      />

      <ConfirmWriteDialog
        open={showConfirmDialog}
        previewResult={searchPreviewResult}
        loading={showConfirmDialog && !searchPreviewResult && !searchWriteError}
        writing={searchWriting}
        writeError={searchWriteError}
        onConfirm={handleConfirmWrite}
        onCancel={handleCancelConfirm}
      />

      {extraTagsTrack && (
        <ExtraTagsEditor
          track={extraTagsTrack}
          saving={mutationBusy}
          onClose={() => setExtraTagsTrack(null)}
          onSave={handleSaveExtraTags}
        />
      )}

      {batchExtraTagsOpen && state.selectedTrackPaths.length > 1 && (
        <BatchExtraTagsEditor
          tracks={selectedTracksForBatch}
          saving={mutationBusy}
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
      artists: [...track.artists],
      album: track.album,
      albumArtist: track.albumArtist,
      albumArtists: [...track.albumArtists],
      year: track.year,
      trackNumber: track.trackNumber,
      trackTotal: track.trackTotal,
      discNumber: track.discNumber,
      discTotal: track.discTotal,
      genre: track.genre,
      composer: track.composer,
      comment: track.comment ?? null,
      description: track.description ?? null,
      compilation: track.compilation,
      musicbrainzTrackId: track.musicbrainzTrackId,
      musicbrainzAlbumId: track.musicbrainzAlbumId,
      musicbrainzArtistId: track.musicbrainzArtistId,
      discogsArtistId: track.discogsArtistId,
      discogsReleaseId: track.discogsReleaseId,
    },
  };
}

/** Capture only values the pending standard metadata patch can change. */
export function createOwnedTrackSnapshot(
  track: TrackData,
  writeFields: Record<string, unknown>,
): TrackSnapshot {
  const trackValues = track as unknown as Record<string, unknown>;
  const fields: Record<string, unknown> = {};
  for (const key of Object.keys(writeFields)) {
    const value = trackValues[key];
    fields[key] = Array.isArray(value) ? [...value] : (value ?? null);
  }
  return { path: track.path, fields };
}

function valuesEqual(left: unknown, right: unknown): boolean {
  return JSON.stringify(left) === JSON.stringify(right);
}

/** Keep only successful readbacks that differ from their pre-write values. */
export function filterChangedSnapshots(
  snapshots: TrackSnapshot[],
  readbacks: TrackData[],
): TrackSnapshot[] {
  const readbackByPath = new Map(readbacks.map((track) => [track.path, track]));
  return snapshots.filter((snapshot) => {
    const readback = readbackByPath.get(snapshot.path);
    if (!readback) return false;
    const values = readback as unknown as Record<string, unknown>;
    return Object.entries(snapshot.fields).some(
      ([key, oldValue]) => !valuesEqual(values[key] ?? null, oldValue),
    );
  });
}

export function extraTagsEqual(
  left: Array<{ key: string; value: string }>,
  right: Array<{ key: string; value: string }>,
): boolean {
  const normalize = (tags: Array<{ key: string; value: string }>) => {
    const seen = new Set<string>();
    const normalized = tags.flatMap(({ key, value }) => {
      const rawKey = key.trim();
      const normalizedProviderKey = rawKey
        .replace(/^TXXX:/i, "")
        .replace(/[ _-]/g, "")
        .toUpperCase()
        .replace(/^MUSICBRAINS/, "MUSICBRAINZ");
      const canonicalKey =
        {
          MUSICBRAINZTRACKID: "MUSICBRAINZ_TRACKID",
          MUSICBRAINZRECORDINGID: "MUSICBRAINZ_TRACKID",
          MUSICBRAINZALBUMID: "MUSICBRAINZ_ALBUMID",
          MUSICBRAINZRELEASEID: "MUSICBRAINZ_ALBUMID",
          MUSICBRAINZARTISTID: "MUSICBRAINZ_ARTISTID",
          DISCOGSARTISTID: "DISCOGS_ARTIST_ID",
          DISCOGSRELEASEID: "DISCOGS_RELEASE_ID",
        }[normalizedProviderKey] ??
        (rawKey.toUpperCase() === "COMM" ? "COMMENT" : rawKey.toUpperCase());
      const normalizedValue = value.trim();
      const identity = `${canonicalKey}\0${normalizedValue}`;
      if (!canonicalKey || !normalizedValue || seen.has(identity)) return [];
      seen.add(identity);
      return [{ key: canonicalKey, value: normalizedValue }];
    });
    return normalized.sort((a, b) => a.key.localeCompare(b.key));
  };
  return valuesEqual(normalize(left), normalize(right));
}

function normalizeAuditField(field: string): keyof TrackData | null {
  const aliases: Record<string, keyof TrackData> = {
    album_artist: "albumArtist",
    album_artists: "albumArtists",
    track_number: "trackNumber",
    track_total: "trackTotal",
    disc_number: "discNumber",
    disc_total: "discTotal",
  };
  const normalized = aliases[field] ?? field;
  const supported = new Set<keyof TrackData>([
    "title",
    "artist",
    "artists",
    "album",
    "albumArtist",
    "albumArtists",
    "year",
    "genre",
    "trackNumber",
    "trackTotal",
    "discNumber",
    "discTotal",
  ]);
  return supported.has(normalized as keyof TrackData)
    ? (normalized as keyof TrackData)
    : null;
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
        throw new Error(
          `Cannot auto-tag without undo snapshot for ${albumPath}: ${message}`,
        );
      }
    }

    if (tracks.length === 0) {
      throw new Error(
        `Cannot auto-tag without undo snapshot for ${albumPath}: no tracks found`,
      );
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
