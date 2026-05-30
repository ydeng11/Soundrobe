import React, { useMemo, useState, useRef, useCallback, useEffect, useLayoutEffect, memo } from "react";
import type { TrackData } from "../../electron/preload";

type SortKey =
  | "filename"
  | "title"
  | "artist"
  | "albumArtist"
  | "album"
  | "year"
  | "track"
  | "genre"
  | "duration"
  | "bitrate";

interface Column {
  key: SortKey;
  label: string;
  width: string;
  align: "left" | "right";
  defaultHidden?: boolean;
}

// Flex proportions used to compute initial pixel widths
const COLUMN_FLEX: Record<string, number> = {
  filename: 3,
  title: 2,
  artist: 1.5,
  albumArtist: 1.5,
  album: 1.5,
  year: 0.7,
  track: 0.7,
  genre: 1,
  duration: 0.85,
  bitrate: 0.85,
};

// Minimum pixel width per column
const MIN_COL_WIDTH = 40;
const HANDLE_WIDTH = 5;
const EMPTY_SELECTED_TRACK_PATHS: string[] = [];

const ALL_COLUMNS: Column[] = [
  { key: "filename", label: "Path", width: "flex-[3]", align: "left" },
  { key: "title", label: "Title", width: "flex-[2]", align: "left" },
  { key: "artist", label: "Artist", width: "flex-[1.5]", align: "left" },
  { key: "albumArtist", label: "Album Artist", width: "flex-[1.5]", align: "left" },
  { key: "album", label: "Album", width: "flex-[1.5]", align: "left" },
  { key: "year", label: "Year", width: "w-16", align: "right" },
  { key: "track", label: "Track", width: "w-16", align: "right" },
  { key: "genre", label: "Genre", width: "flex-[1]", align: "left" },
  { key: "duration", label: "Duration", width: "w-20", align: "right" },
  { key: "bitrate", label: "Bitrate", width: "w-20", align: "right" },
];

interface FileGridProps {
  tracks: TrackData[];
  selectedTrackPath: string | null;
  selectedTrackPaths?: string[];
  filterText: string;
  onSelectTrack: (path: string, track: TrackData) => void;
  onMultiSelect?: (paths: string[]) => void;
  onEditExtraTags?: (track: TrackData, selectedPaths: string[]) => void;
  onDeleteFiles?: (paths: string[]) => void;
}

export function FileGrid({
  tracks,
  selectedTrackPath,
  selectedTrackPaths = EMPTY_SELECTED_TRACK_PATHS,
  filterText,
  onSelectTrack,
  onMultiSelect,
  onEditExtraTags,
  onDeleteFiles,
}: FileGridProps) {
  const [sortKey, setSortKey] = useState<SortKey>("track");
  const [sortAsc, setSortAsc] = useState(true);
  const lastClickedRef = useRef<number>(-1);

  const [visibleColumns, setVisibleColumns] = useState<Set<string>>(
    () => new Set(ALL_COLUMNS.filter((c) => !c.defaultHidden).map((c) => c.key))
  );

  // Memoize visible column descriptors — stable identity while visible set is unchanged
  const COLUMNS = useMemo(
    () => ALL_COLUMNS.filter((c) => visibleColumns.has(c.key)),
    [visibleColumns]
  );

  // Derive multi-selected paths from props instead of mirroring in local state
  const multiSelected = useMemo(
    () => new Set(selectedTrackPaths),
    [selectedTrackPaths]
  );

  // Draggable column widths (pixel values)
  const headerRef = useRef<HTMLDivElement>(null);
  const [columnWidths, setColumnWidths] = useState<Record<string, number> | null>(null);

  // Compute initial pixel widths — always sum exactly to container width
  useLayoutEffect(() => {
    if (columnWidths) return; // Already initialized
    const container = headerRef.current?.parentElement;
    if (!container) return;
    const visKeys = ALL_COLUMNS.filter((c) => visibleColumns.has(c.key));
    // Account for resize handle widths between columns
    const handleTotal = (visKeys.length - 1) * HANDLE_WIDTH;
    const totalWidth = container.clientWidth - 24 - handleTotal;
    if (totalWidth <= 0) return;

    const totalFlex = visKeys.reduce(
      (sum, c) => sum + (COLUMN_FLEX[c.key] ?? 1),
      0
    );
    const widths: Record<string, number> = {};

    // Proportional widths, clamped to minimum
    visKeys.forEach((c) => {
      widths[c.key] = Math.max(
        MIN_COL_WIDTH,
        Math.round(((COLUMN_FLEX[c.key] ?? 1) / totalFlex) * totalWidth)
      );
    });

    // Redistribute pixel by pixel so total equals container width minus handles
    let sum = Object.values(widths).reduce((a, b) => a + b, 0);
    let iter = 0;
    while (sum !== totalWidth && iter < visKeys.length * 2) {
      for (const c of visKeys) {
        if (sum === totalWidth) break;
        if (sum < totalWidth) {
          widths[c.key]++;
          sum++;
        } else if (widths[c.key] > MIN_COL_WIDTH) {
          widths[c.key]--;
          sum--;
        }
      }
      iter++;
    }

    setColumnWidths(widths);
  }, [visibleColumns, columnWidths]);

  // Column resize drag state
  const dragRef = useRef<{
    colIndex: number;
    startX: number;
    startW: number;
    nextStartW: number;
  } | null>(null);

  const handleResizeStart = useCallback(
    (e: React.MouseEvent, colIndex: number) => {
      e.preventDefault();
      e.stopPropagation();
      const container = headerRef.current?.parentElement;
      if (!container || !visibleColumns || !columnWidths) {
        return;
      }
      const visKeys = ALL_COLUMNS.filter((c) =>
        visibleColumns.has(c.key)
      );
      const curCol = visKeys[colIndex];
      const nextCol = visKeys[colIndex + 1];
      if (!curCol || !nextCol) return;

      dragRef.current = {
        colIndex,
        startX: e.clientX,
        startW: columnWidths[curCol.key],
        nextStartW: columnWidths[nextCol.key],
      };

      const handleMouseMove = (ev: MouseEvent) => {
        if (!dragRef.current) return;
        const container = headerRef.current?.parentElement;
        if (!container) return;
        const { startX, startW, nextStartW } = dragRef.current;
        // Compute the current total width to cap overflow
        const visKeys = ALL_COLUMNS.filter((c) =>
          visibleColumns.has(c.key)
        );
        const handleTotal = (visKeys.length - 1) * HANDLE_WIDTH;
        const availableForCols = container.clientWidth - 24 - handleTotal;
        const delta = ev.clientX - startX;
        let newW = Math.max(MIN_COL_WIDTH, startW + delta);
        let newNext = Math.max(MIN_COL_WIDTH, nextStartW - delta);

        // Cap total width so columns don't overflow the container
        const otherSum = visKeys.reduce((sum, c) => {
          if (c.key === curCol.key || c.key === nextCol.key) return sum;
          return sum + (columnWidths?.[c.key] ?? MIN_COL_WIDTH);
        }, 0);
        const totalAllowed = newW + newNext + otherSum;
        if (totalAllowed > availableForCols && newNext > MIN_COL_WIDTH) {
          // Shrink next column to fit
          newNext = Math.max(MIN_COL_WIDTH, availableForCols - otherSum - newW);
        } else if (totalAllowed > availableForCols && newW > MIN_COL_WIDTH) {
          // Shrink current column to fit
          newW = Math.max(MIN_COL_WIDTH, availableForCols - otherSum - newNext);
        }

        setColumnWidths((prev) => {
          if (!prev) return prev;
          return {
            ...prev,
            [curCol.key]: newW,
            [nextCol.key]: newNext,
          };
        });
      };

      const handleMouseUp = () => {
        dragRef.current = null;
        window.removeEventListener("mousemove", handleMouseMove);
        window.removeEventListener("mouseup", handleMouseUp);
        document.body.style.cursor = "";
        document.body.style.userSelect = "";
      };

      window.addEventListener("mousemove", handleMouseMove);
      window.addEventListener("mouseup", handleMouseUp);
      document.body.style.cursor = "col-resize";
      document.body.style.userSelect = "none";
    },
    [visibleColumns, columnWidths]
  );

  // Right-click context menu state
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number } | null>(null);

  // Close context menu on any click outside
  useEffect(() => {
    const close = () => setContextMenu(null);
    if (contextMenu) {
      window.addEventListener("click", close);
      return () => window.removeEventListener("click", close);
    }
  }, [contextMenu]);

  const toggleColumn = useCallback((key: string) => {
    setVisibleColumns((prev) => {
      const next = new Set(prev);
      // Don't allow hiding the last column
      if (next.has(key)) {
        if (next.size <= 1) return prev;
        next.delete(key);
      } else {
        next.add(key);
      }
      return next;
    });
  }, []);

  const toggleSort = useCallback((key: SortKey) => {
    if (key === sortKey) {
      setSortAsc((prev) => !prev);
    } else {
      setSortKey(key);
      setSortAsc(true);
    }
  }, [sortKey]);

  const sorted = useMemo(() => {
    let list = tracks;

    // Apply filter
    if (filterText) {
      const lower = filterText.toLowerCase();
      list = list.filter(
        (t) =>
          (t.title ?? "").toLowerCase().includes(lower) ||
          (t.artist ?? "").toLowerCase().includes(lower) ||
          (t.album ?? "").toLowerCase().includes(lower) ||
          shortPath(t.path).toLowerCase().includes(lower)
      );
    }

    return [...list].sort((a, b) => {
      let cmp = 0;
      switch (sortKey) {
        case "filename":
          cmp = shortPath(a.path).localeCompare(shortPath(b.path));
          break;
        case "title":
          cmp = (a.title ?? "").localeCompare(b.title ?? "");
          break;
        case "artist":
          cmp = (a.artist ?? "").localeCompare(b.artist ?? "");
          break;
        case "albumArtist":
          cmp = (a.albumArtist ?? "").localeCompare(b.albumArtist ?? "");
          break;
        case "album":
          cmp = (a.album ?? "").localeCompare(b.album ?? "");
          break;
        case "year":
          cmp = ((a.year ?? "") as string).localeCompare(b.year ?? "");
          break;
        case "track":
          cmp = (a.trackNumber ?? 999) - (b.trackNumber ?? 999);
          break;
        case "genre":
          cmp = (a.genre ?? "").localeCompare(b.genre ?? "");
          break;
        case "duration":
          cmp = a.duration - b.duration;
          break;
        case "bitrate":
          cmp = (a.bitrate ?? 0) - (b.bitrate ?? 0);
          break;
      }
      return sortAsc ? cmp : -cmp;
    });
  }, [tracks, filterText, sortKey, sortAsc]);

  // Memoize the sorted paths array for quick lookups in the Cmd+A handler
  const sortedPaths = useMemo(() => sorted.map((t) => t.path), [sorted]);

  // Select all with Cmd+A (when focus is not in an input/textarea)
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key === "a") {
        const active = document.activeElement?.tagName?.toLowerCase();
        if (active === "input" || active === "textarea" || active === "select") {
          return; // Let native select-all work in text fields
        }
        e.preventDefault();
        onMultiSelect?.(sortedPaths);
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [sortedPaths, onMultiSelect]);

  const showTrackMenu = useCallback(
    async (track: TrackData) => {
      return window.api.showTrackContextMenu(track.path, {
        title: track.title ?? "",
        artist: track.artist ?? "",
        albumArtist: track.albumArtist ?? "",
        album: track.album ?? "",
        year: track.year ?? "",
        track:
          track.trackNumber != null
            ? `${track.trackNumber}${track.trackTotal ? `/${track.trackTotal}` : ""}`
            : "",
        genre: track.genre ?? "",
      });
    },
    [],
  );

  const handleRowContextMenu = useCallback(
    async (e: React.MouseEvent, track: TrackData) => {
      e.preventDefault();
      e.stopPropagation();

      const currentPaths = selectedTrackPaths;
      const isSelected = currentPaths.includes(track.path);
      const menuSelectedPaths =
        isSelected && currentPaths.length > 1
          ? currentPaths
          : [track.path];

      if (!isSelected || currentPaths.length <= 1) {
        onMultiSelect?.([track.path]);
        onSelectTrack(track.path, track);
      }

      const action = await showTrackMenu(track);

      if (action === "extra-tags") {
        onEditExtraTags?.(track, menuSelectedPaths);
      } else if (action === "delete-files") {
        onDeleteFiles?.(menuSelectedPaths);
      }
    },
    [onEditExtraTags, onDeleteFiles, onMultiSelect, onSelectTrack, selectedTrackPaths, showTrackMenu],
  );

  const handleFileAreaContextMenu = useCallback(
    async (e: React.MouseEvent) => {
      e.preventDefault();

      if (selectedTrackPaths.length === 0) {
        return;
      }

      const primary =
        sorted.find((track) => track.path === selectedTrackPaths[0]) ??
        tracks.find((track) => track.path === selectedTrackPaths[0]);
      if (!primary) {
        return;
      }

      const action = await showTrackMenu(primary);

      if (action === "extra-tags") {
        onEditExtraTags?.(primary, selectedTrackPaths);
      } else if (action === "delete-files") {
        onDeleteFiles?.(selectedTrackPaths);
      }
    },
    [onEditExtraTags, onDeleteFiles, selectedTrackPaths, showTrackMenu, sorted, tracks],
  );

  const handleRowClick = useCallback(
    (track: TrackData, index: number, event: React.MouseEvent) => {
      if (event.shiftKey && lastClickedRef.current >= 0) {
        const from = Math.min(lastClickedRef.current, index);
        const to = Math.max(lastClickedRef.current, index);
        const range: string[] = [];
        for (let i = from; i <= to; i++) {
          range.push(sorted[i].path);
        }
        onMultiSelect?.(range);
        // Don't call onSelectTrack for range selects — the SELECT_TRACK action would
        // overwrite selectedTrackPaths to a single element, hiding the BatchEditor.
        // The BatchEditor (shown when selectedTrackPaths.length > 1) gets cover art
        // from handleMultiSelect's first-track logic anyway.
      } else {
        onMultiSelect?.([track.path]);
        lastClickedRef.current = index;
        onSelectTrack(track.path, track);
      }
    },
    [sorted, onSelectTrack, onMultiSelect]
  );

  const handleHeaderContextMenu = useCallback((e: React.MouseEvent) => {
    e.preventDefault();
    setContextMenu({ x: e.clientX, y: e.clientY });
  }, []);

  return (
    <div className="flex flex-col h-full bg-white">
      {/* Column headers */}
      <div
        ref={headerRef}
        className="flex items-center px-3 py-1.5 text-[11px] font-medium text-text-muted bg-surface-alt/60 border-b border-border select-none overflow-hidden"
        onContextMenu={handleHeaderContextMenu}
      >
        {COLUMNS.map((col, ci) => (
          <React.Fragment key={col.key}>
            <button
              onClick={() => toggleSort(col.key)}
              className={`flex items-center gap-1.5 px-1.5 hover:text-text-secondary transition-all duration-150 truncate shrink-0 rounded-md ${
                col.key === sortKey
                  ? "bg-accent/10 text-accent font-semibold"
                  : "hover:bg-surface-hover"
              }`}
              style={{
                width: columnWidths?.[col.key] ?? (col.width.includes("-") ? 80 : 120),
                minWidth: MIN_COL_WIDTH,
              }}
            >
              <span className="truncate">{col.label}</span>
              <span className="inline-flex items-center justify-center w-3.5 h-3.5 shrink-0">
                {col.key === sortKey ? (
                  <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3" strokeLinecap="round" strokeLinejoin="round" className="text-accent transition-transform duration-200" style={{ transform: sortAsc ? 'rotate(0deg)' : 'rotate(180deg)' }}>
                    <polyline points="18 15 12 9 6 15" />
                  </svg>
                ) : (
                  <svg width="9" height="9" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="opacity-20 group-hover:opacity-40 transition-opacity">
                    <line x1="12" y1="5" x2="12" y2="19" />
                    <polyline points="19 12 12 19 5 12" />
                  </svg>
                )}
              </span>
            </button>
            {/* Resize handle — visible divider between headers */}
            {ci < COLUMNS.length - 1 && (
              <div
                onMouseDown={(e) => handleResizeStart(e, ci)}
                className="shrink-0 w-[5px] cursor-col-resize bg-border hover:bg-accent/40 active:bg-accent/60 transition-all duration-200 self-stretch mx-0 rounded-sm group-hover:bg-accent/20"
              />
            )}
          </React.Fragment>
        ))}
      </div>

      {/* Column visibility context menu */}
      {contextMenu && (
        <div
          className="fixed z-50 bg-white border border-border rounded-lg shadow-lg py-1 min-w-[140px]"
          style={{ left: contextMenu.x, top: contextMenu.y }}
        >
          <div className="px-3 py-1.5 text-[10px] font-semibold uppercase tracking-widest text-text-muted">
            Columns
          </div>
          <div className="h-px bg-border/50 mx-2 my-0.5" />
          {ALL_COLUMNS.map((col) => (
            <button
              key={col.key}
              onClick={(e) => {
                e.stopPropagation();
                toggleColumn(col.key);
              }}
              className={`w-full flex items-center gap-2 px-3 py-1.5 text-[12px] text-left hover:bg-surface-hover transition-colors ${
                visibleColumns.has(col.key)
                  ? "text-text-primary font-medium"
                  : "text-text-muted"
              }`}
            >
              <span className="w-4 h-4 flex items-center justify-center">
                {visibleColumns.has(col.key) ? (
                  <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" strokeLinejoin="round" className="text-accent">
                    <polyline points="20 6 9 17 4 12" />
                  </svg>
                ) : (
                  <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
                    <rect x="3" y="3" width="18" height="18" rx="2" ry="2" />
                  </svg>
                )}
              </span>
              {col.label}
            </button>
          ))}
        </div>
      )}

      {/* File rows */}
      <div
        className="flex-1 overflow-y-auto"
        data-testid="file-grid-body"
        onContextMenu={handleFileAreaContextMenu}
      >
        {sorted.length === 0 ? (
          <div className="flex items-center justify-center h-full text-text-muted text-[12px]">
            <div className="flex flex-col items-center gap-2">
              <svg width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1" strokeLinecap="round" strokeLinejoin="round" className="opacity-40">
                <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z" />
                <polyline points="14 2 14 8 20 8" />
                <line x1="12" y1="18" x2="12" y2="12" />
                <line x1="9" y1="15" x2="15" y2="15" />
              </svg>
              {tracks.length === 0
                ? "No audio files found"
                : "No files match the filter"}
            </div>
          </div>
        ) : (
          sorted.map((track, i) => (
            <FileGridRow
              key={track.path}
              track={track}
              index={i}
              isAltRow={i % 2 === 0}
              isPrimary={track.path === selectedTrackPath}
              isMulti={multiSelected.has(track.path)}
              columns={COLUMNS}
              columnWidths={columnWidths}
              onRowClick={handleRowClick}
              onRowContextMenu={handleRowContextMenu}
            />
          ))
        )}
      </div>

      {/* Footer bar */}
      <div className="flex items-center px-4 py-1.5 text-[10.5px] text-text-muted bg-surface-alt/60 border-t border-border gap-3">
        <span className="tabular-nums">{sorted.length} file{sorted.length !== 1 ? "s" : ""}</span>
        {multiSelected.size > 1 && (
          <span className="text-accent tabular-nums font-medium">{multiSelected.size} selected</span>
        )}
        {filterText && (
          <span className="text-text-muted/50 tabular-nums">
            filtered from {tracks.length}
          </span>
        )}
        <div className="flex-1" />
        <button
          onClick={() => setContextMenu({ x: 0, y: 0 })}
          className="inline-flex items-center gap-1.5 text-[10px] hover:text-text-secondary transition-all duration-150 hover:scale-[1.05]"
          title="Toggle columns"
        >
          <svg width="11" height="11" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
            <rect x="3" y="3" width="18" height="18" rx="2" ry="2" />
            <line x1="3" y1="9" x2="21" y2="9" />
            <line x1="9" y1="21" x2="9" y2="9" />
          </svg>
          Columns
        </button>
        {sorted.length > 0 && (
          <span className="text-text-muted/30 text-[10px]">
            Click to sort · Shift+click range
          </span>
        )}
      </div>
    </div>
  );
}

function getCellValue(track: TrackData, key: SortKey): string {
  switch (key) {
    case "filename":
      return shortPath(track.path);
    case "title":
      return track.title ?? "—";
    case "artist":
      return track.artist ?? "—";
    case "albumArtist":
      return track.albumArtist ?? "—";
    case "album":
      return track.album ?? "—";
    case "year":
      return track.year ?? "—";
    case "track":
      return track.trackNumber != null
        ? track.trackTotal != null
          ? `${track.trackNumber}/${track.trackTotal}`
          : String(track.trackNumber)
        : "—";
    case "genre":
      return track.genre ?? "—";
    case "duration":
      return formatDuration(track.duration);
    case "bitrate":
      return track.bitrate ? `${Math.round(track.bitrate / 1000)}k` : "—";
  }
}

// ── Memoized row component ──

interface FileGridRowProps {
  track: TrackData;
  index: number;
  isAltRow: boolean;
  isPrimary: boolean;
  isMulti: boolean;
  columns: Column[];
  columnWidths: Record<string, number> | null;
  onRowClick: (track: TrackData, index: number, event: React.MouseEvent) => void;
  onRowContextMenu: (e: React.MouseEvent, track: TrackData) => void;
}

const FileGridRow = memo(function FileGridRow({
  track,
  index,
  isAltRow,
  isPrimary,
  isMulti,
  columns,
  columnWidths,
  onRowClick,
  onRowContextMenu,
}: FileGridRowProps) {
  const rowClass = [
    "flex items-center px-3 py-1 text-[12.5px] cursor-pointer select-none border-b border-border/30",
    "transition-colors duration-75",
    isPrimary
      ? "bg-table-selected border-table-selectedBorder shadow-[inset_2px_0_0_0_rgba(0,122,255,0.5)]"
      : isMulti
        ? "bg-table-selected/60"
        : isAltRow
          ? "bg-table-alt"
          : "bg-table-row",
    "hover:bg-table-selected/40",
  ].join(" ");

  return (
    <div
      onClick={(e) => onRowClick(track, index, e)}
      onContextMenu={(e) => onRowContextMenu(e, track)}
      className={rowClass}
    >
      {columns.map((col, ci) => (
        <React.Fragment key={col.key}>
          <span
            className={`shrink-0 ${
              col.align === "right" ? "text-right" : "text-left"
            } px-1.5 truncate ${
              isPrimary
                ? "text-text-primary font-medium"
                : "text-text-secondary"
            }`}
            style={{
              width: columnWidths?.[col.key] ?? (col.width.includes("-") ? 80 : 120),
              minWidth: MIN_COL_WIDTH,
            }}
          >
            {col.key === "filename" ? (
              <span className="flex items-center gap-2">
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" className="shrink-0 text-text-muted">
                  <path d="M9 18V5l12-2v13" />
                  <circle cx="6" cy="18" r="3" />
                  <circle cx="18" cy="16" r="3" />
                </svg>
                <span>{getCellValue(track, col.key)}</span>
              </span>
            ) : (
              getCellValue(track, col.key)
            )}
          </span>
          {ci < columns.length - 1 && (
            <div className="shrink-0 w-[5px]" />
          )}
        </React.Fragment>
      ))}
    </div>
  );
});

function shortPath(p: string): string {
  return p.split("/").slice(-4).join("/").replace(/^\//, "");
}

function formatDuration(sec: number): string {
  const m = Math.floor(sec / 60);
  const s = Math.floor(sec % 60);
  return `${m}:${s.toString().padStart(2, "0")}`;
}
