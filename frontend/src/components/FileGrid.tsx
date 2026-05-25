import React, { useMemo, useState, useRef, useCallback } from "react";
import type { TrackData } from "../../electron/preload";

type SortKey =
  | "filename"
  | "path"
  | "title"
  | "artist"
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
}

const COLUMNS: Column[] = [
  { key: "filename", label: "Filename", width: "flex-[2]", align: "left" },
  { key: "path", label: "Path", width: "flex-[3]", align: "left" },
  { key: "title", label: "Title", width: "flex-[2]", align: "left" },
  { key: "artist", label: "Artist", width: "flex-[1.5]", align: "left" },
  { key: "album", label: "Album", width: "flex-[1.5]", align: "left" },
  { key: "year", label: "Year", width: "w-14", align: "right" },
  { key: "track", label: "Track", width: "w-12", align: "right" },
  { key: "genre", label: "Genre", width: "flex-[1]", align: "left" },
  { key: "duration", label: "Duration", width: "w-16", align: "right" },
  { key: "bitrate", label: "Bitrate", width: "w-16", align: "right" },
];

interface FileGridProps {
  tracks: TrackData[];
  selectedTrackPath: string | null;
  filterText: string;
  onSelectTrack: (path: string, track: TrackData) => void;
}

export function FileGrid({
  tracks,
  selectedTrackPath,
  filterText,
  onSelectTrack,
}: FileGridProps) {
  const [sortKey, setSortKey] = useState<SortKey>("filename");
  const [sortAsc, setSortAsc] = useState(true);
  const [multiSelected, setMultiSelected] = useState<Set<string>>(new Set());
  const lastClickedRef = useRef<number>(-1);

  const toggleSort = (key: SortKey) => {
    if (key === sortKey) {
      setSortAsc(!sortAsc);
    } else {
      setSortKey(key);
      setSortAsc(true);
    }
  };

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
          t.path.toLowerCase().includes(lower) ||
          pathBasename(t.path).toLowerCase().includes(lower)
      );
    }

    return [...list].sort((a, b) => {
      let cmp = 0;
      switch (sortKey) {
        case "filename":
          cmp = pathBasename(a.path).localeCompare(pathBasename(b.path));
          break;
        case "path":
          cmp = a.path.localeCompare(b.path);
          break;
        case "title":
          cmp = (a.title ?? "").localeCompare(b.title ?? "");
          break;
        case "artist":
          cmp = (a.artist ?? "").localeCompare(b.artist ?? "");
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

  const formatDuration = (sec: number): string => {
    const m = Math.floor(sec / 60);
    const s = Math.floor(sec % 60);
    return `${m}:${s.toString().padStart(2, "0")}`;
  };

  const handleRowClick = useCallback((track: TrackData, index: number, event: React.MouseEvent) => {
    if (event.shiftKey && lastClickedRef.current >= 0) {
      // Shift+click: select a range from last clicked to current
      const from = Math.min(lastClickedRef.current, index);
      const to = Math.max(lastClickedRef.current, index);
      const range = new Set<string>();
      for (let i = from; i <= to; i++) {
        range.add(sorted[i].path);
      }
      setMultiSelected(range);
    } else {
      // Regular click: select just this track
      setMultiSelected(new Set([track.path]));
    }
    lastClickedRef.current = index;
    onSelectTrack(track.path, track);
  }, [sorted, onSelectTrack]);

  const getCellValue = (track: TrackData, key: SortKey): string => {
    switch (key) {
      case "filename":
        return pathBasename(track.path);
      case "path":
        return track.path;
      case "title":
        return track.title ?? "—";
      case "artist":
        return track.artist ?? "—";
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
        return track.duration ? formatDuration(track.duration) : "—";
      case "bitrate":
        return track.bitrate ? `${Math.round(track.bitrate / 1000)}k` : "—";
    }
  };

  return (
    <div className="flex flex-col h-full">
      {/* Column headers */}
      <div className="flex items-center px-2 py-1.5 text-[10px] font-semibold uppercase tracking-wider text-text-muted bg-surface-alt/80 border-b border-gray-700/30">
        {COLUMNS.map((col) => (
          <button
            key={col.key}
            onClick={() => toggleSort(col.key)}
            className={`${col.width} ${col.align === "right" ? "text-right" : "text-left"} px-1.5 hover:text-text-secondary transition-colors truncate`}
          >
            {col.label}
            <span className="ml-0.5 w-3 inline-block text-center">
              {col.key === sortKey ? (
                <span className="text-accent-light">{sortAsc ? "▲" : "▼"}</span>
              ) : (
                <span className="text-text-muted/30">⇅</span>
              )}
            </span>
          </button>
        ))}
      </div>

      {/* File rows */}
      <div className="flex-1 overflow-y-auto">
        {sorted.length === 0 ? (
          <div className="flex items-center justify-center h-full text-text-muted text-xs">
            {tracks.length === 0
              ? "No audio files found"
              : "No files match the filter"}
          </div>
        ) : (
          sorted.map((track, i) => {
            const isPrimary = track.path === selectedTrackPath;
            const isMulti = multiSelected.has(track.path);
            const altRow = i % 2 === 0;
            return (
              <div
                key={track.path}
                className={`flex items-center px-2 py-1 text-xs cursor-pointer transition-colors select-none ${
                  isPrimary
                    ? "bg-accent/20 text-text-primary"
                    : isMulti
                      ? "bg-accent/10 text-text-primary"
                      : altRow
                        ? "bg-surface/30"
                        : "bg-surface/10"
                } hover:bg-surface-hover`}
                onClick={(e) => handleRowClick(track, i, e)}
              >
                {COLUMNS.map((col) => (
                  <span
                    key={col.key}
                    className={`${col.width} ${col.align === "right" ? "text-right" : "text-left"} px-1.5 truncate ${
                      isPrimary || isMulti ? "text-text-primary" : "text-text-secondary"
                    }`}
                  >
                    {getCellValue(track, col.key)}
                  </span>
                ))}
              </div>
            );
          })
        )}
      </div>

      {/* Footer bar */}
      <div className="flex items-center px-3 py-1 text-[10px] text-text-muted bg-surface-alt/50 border-t border-gray-700/30 gap-2">
        <span>{sorted.length} files</span>
        {multiSelected.size > 1 && (
          <span className="text-accent-light">{multiSelected.size} selected</span>
        )}
        {filterText && (
          <span className="text-accent-dim">
            (filtered from {tracks.length})
          </span>
        )}
      </div>
    </div>
  );
}

/** Extract filename from a path string (no path import in renderer). */
function pathBasename(p: string): string {
  const sep = p.replace(/\\/g, "/");
  return sep.split("/").pop() ?? p;
}
