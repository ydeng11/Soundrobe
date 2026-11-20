/**
 * Workspace file table.
 *
 * Renders the reference's exact ten columns in the reference's order with the
 * reference's measured widths. Rows are selected by clicking the file-name
 * cell; the reference offers no checkbox column, so additive selection uses
 * Ctrl/Cmd-click and Shift-click (range) — see `selectionModel` on the parent.
 */

import React from "react";
import { formatDuration } from "./TrackDetailEditor";
import type { TrackData } from "../shared/desktop-api";
import {
  WEB_TABLE_COLUMNS,
  WORKSPACE_TRACK_STATUS_COLUMN,
  WORKSPACE_TRACK_TIME_COLUMN,
  fileNameOf,
  hasSidecarLrc,
  type WebTableColumnKey,
} from "./columns";

export interface WebTableSort {
  key: WebTableColumnKey;
  ascending: boolean;
}

export interface TrackStatus {
  label: string;
  detail?: string;
  tone?: "notice" | "error" | "warning" | "success";
}

export interface TrackTableProps {
  tracks: TrackData[];
  selectedPaths: string[];
  /** Lowercased names in the current directory, used for LRC sidecar detection. */
  directoryNames?: ReadonlySet<string>;
  /** Cover artwork for the current directory, when one is available. */
  coverDataUrl?: string | null;
  sort?: WebTableSort;
  onSortChange?: (sort: WebTableSort) => void;
  onSelectTrack: (path: string) => void;
  onSelectionChange?: (paths: string[]) => void;
  /**
   * Columns to render, in reference order. Omitted means every column.
   *
   * The rendered columns share the pane's width in the reference's proportions.
   * Hiding columns recomputes those proportions across the columns that remain,
   * so the table keeps filling the pane instead of leaving a gap.
   */
  visibleColumns?: readonly WebTableColumnKey[];
  trackStatus?: ReadonlyMap<string, TrackStatus>;
}

export function TrackTable({
  tracks,
  selectedPaths,
  directoryNames,
  coverDataUrl,
  sort,
  onSortChange,
  onSelectTrack,
  onSelectionChange,
  visibleColumns,
  trackStatus,
}: TrackTableProps) {
  const selected = React.useMemo(() => new Set(selectedPaths), [selectedPaths]);
  const anchorRef = React.useRef<string | null>(null);

  const columns = React.useMemo(
    () =>
      visibleColumns && visibleColumns.length > 0
        ? WEB_TABLE_COLUMNS.filter((column) => visibleColumns.includes(column.key))
        : WEB_TABLE_COLUMNS,
    [visibleColumns],
  );
  const referenceWidth = React.useMemo(
    () => columns.reduce((total, column) => total + column.width, 0),
    [columns],
  );

  const utilityWidth = trackStatus
    ? WORKSPACE_TRACK_STATUS_COLUMN.width + WORKSPACE_TRACK_TIME_COLUMN.width
    : 0;

  const stackTitle = Boolean(trackStatus) && !columns.some(({ key }) => key === "title");

  const handleActivate = (event: React.MouseEvent, path: string) => {
    const additive = event.metaKey || event.ctrlKey;
    const range = event.shiftKey;

    if (additive) {
      const next = new Set(selected);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      anchorRef.current = path;
      onSelectionChange?.([...next]);
      return;
    }

    if (range && anchorRef.current) {
      const from = tracks.findIndex((t) => t.path === anchorRef.current);
      const to = tracks.findIndex((t) => t.path === path);
      if (from >= 0 && to >= 0) {
        const [lo, hi] = from <= to ? [from, to] : [to, from];
        onSelectionChange?.(tracks.slice(lo, hi + 1).map((t) => t.path));
        return;
      }
    }

    anchorRef.current = path;
    onSelectTrack(path);
  };

  const toggleSort = (key: WebTableColumnKey) => {
    if (!onSortChange) return;
    const ascending = sort?.key === key ? !sort.ascending : true;
    onSortChange({ key, ascending });
  };

  return (
    <div className="sr-table-wrap">
      {/* The table fills its pane (width: 100%) but never drops below the
          reference column widths, where text would start to clip; the wrap
          scrolls instead. Each column keeps its share of the reference total,
          so the columns stay in proportion however wide the pane is. */}
      <table
        className={trackStatus ? "sr-table sr-table--workspace" : "sr-table"}
        data-testid="workspace-table"
        style={{ minWidth: referenceWidth + utilityWidth }}
      >
        <colgroup>
          {columns.map((column) => (
            <col
              key={column.key}
              data-testid={`workspace-col-${column.key}`}
              style={{ width: utilityWidth
                ? `calc((100% - ${utilityWidth}px) * ${column.width / referenceWidth})`
                : `${((column.width / referenceWidth) * 100).toFixed(4)}%` }}
            />
          ))}
          {trackStatus && <>
            <col style={{ width: WORKSPACE_TRACK_STATUS_COLUMN.width }} />
            <col style={{ width: WORKSPACE_TRACK_TIME_COLUMN.width }} />
          </>}
        </colgroup>
        <thead>
          <tr>
            {columns.map((column) => {
              const sortable = column.key !== "cover" && Boolean(onSortChange);
              const active = sort?.key === column.key;
              return (
                <th
                  key={column.key}
                  scope="col"
                  aria-sort={active ? (sort!.ascending ? "ascending" : "descending") : undefined}
                >
                  {sortable ? (
                    <button type="button" onClick={() => toggleSort(column.key)}>
                      {trackStatus && column.key === "fileName" ? "Track" : column.label}
                      {active ? (sort!.ascending ? " ▲" : " ▼") : ""}
                    </button>
                  ) : (
                    column.label
                  )}
                </th>
              );
            })}
            {trackStatus && <>
              <th scope="col">{WORKSPACE_TRACK_STATUS_COLUMN.label}</th>
              <th scope="col">{WORKSPACE_TRACK_TIME_COLUMN.label}</th>
            </>}
          </tr>
        </thead>
        <tbody>
          {tracks.map((track) => {
            const isSelected = selected.has(track.path);
            const artists = track.artists.length > 0 ? track.artists.join(", ") : track.artist ?? "";
            const albumArtist =
              track.albumArtist ??
              (track.albumArtists.length > 0 ? track.albumArtists.join(", ") : "");
            const lrc = directoryNames ? hasSidecarLrc(track.path, directoryNames) : false;

            const status = trackStatus?.get(track.path);

            return (
              <tr
                key={track.path}
                data-testid="workspace-row"
                aria-selected={isSelected}
                className={isSelected ? "is-selected" : undefined}
              >
                {columns.map((column) => (
                  <td
                    key={column.key}
                    className={
                      column.key === "fileName"
                        ? "sr-cell--name"
                        : column.key === "lyrics" || column.key === "lrc" || column.key === "year"
                          ? "sr-cell--muted"
                          : undefined
                    }
                  >
                    {renderCell(column.key, {
                      track,
                      stackTitle,
                      coverDataUrl,
                      lrc,
                      artists,
                      albumArtist,
                      onActivate: (event) => handleActivate(event, track.path),
                      onSelect: () => onSelectTrack(track.path),
                    })}
                  </td>
                ))}
                {trackStatus && <>
                  <td className="sr-track-status">
                    {status && <span data-tone={status.tone} title={status.detail}>{status.label}</span>}
                  </td>
                  <td className="sr-track-time">{formatDuration(track.duration)}</td>
                </>}
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

interface CellContext {
  stackTitle: boolean;
  track: TrackData;
  coverDataUrl: string | null | undefined;
  lrc: boolean;
  artists: string;
  albumArtist: string;
  onActivate: (event: React.MouseEvent) => void;
  onSelect: () => void;
}

/** One cell per column, so hiding a column hides its header and cell together. */
function renderCell(key: WebTableColumnKey, context: CellContext): React.ReactNode {
  const { stackTitle, track, coverDataUrl, lrc, artists, albumArtist, onActivate, onSelect } = context;
  switch (key) {
    case "cover":
      return coverDataUrl && track.hasCover ? (
        <img className="sr-cover" src={coverDataUrl} alt="" />
      ) : (
        <span aria-hidden="true" className="sr-cover" />
      );
    case "fileName":
      return (
        <span
          role="button"
          tabIndex={0}
          aria-label={stackTitle ? fileNameOf(track.path) : undefined}
          title={stackTitle ? fileNameOf(track.path) : undefined}
          onClick={onActivate}
          onKeyDown={(event) => {
            if (event.key === "Enter" || event.key === " ") {
              event.preventDefault();
              onSelect();
            }
          }}
        >
          {stackTitle && track.title?.trim() ? (
            <>
              <strong>{track.title}</strong>
              <small>{fileNameOf(track.path)}</small>
            </>
          ) : fileNameOf(track.path)}
        </span>
      );
    case "title":
      return track.title ?? "";
    case "artists":
      return artists;
    case "albums":
      return track.album ?? "";
    case "albumArtist":
      return albumArtist;
    case "lyrics":
      return track.lyrics ? "Yes" : "No";
    case "lrc":
      return lrc ? "Yes" : "No";
    case "year":
      return track.year ?? "";
    case "genres":
      return track.genre ?? "";
    default:
      return null;
  }
}
