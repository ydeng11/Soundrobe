/**
 * File-table column contract for the browser workspace.
 *
 * The column set, order and labels are the exact columns measured on the
 * Music Tag Web V2 workspace file table (`table table-sm table-pin-rows`).
 *
 * `width` is the reference's measured pixel width and is the single source for
 * column sizing: the table renders each column as its share of these widths, so
 * the columns keep the reference's proportions at any pane width without the CSS
 * carrying a second copy of the numbers.
 */

export interface WebTableColumn {
  key: string;
  label: string;
  /** Numeric width in CSS pixels, mirrored from the reference DOM. */
  width: number;
}

export const WEB_TABLE_COLUMNS: readonly WebTableColumn[] = [
  { key: "cover", label: "Cover", width: 80 },
  { key: "fileName", label: "File Name", width: 174 },
  { key: "title", label: "Title", width: 118 },
  { key: "artists", label: "Artists", width: 92 },
  { key: "albums", label: "Albums", width: 148 },
  { key: "albumArtist", label: "Album Artist", width: 100 },
  { key: "lyrics", label: "Lyrics", width: 62 },
  { key: "lrc", label: "LRC", width: 62 },
  { key: "year", label: "Year", width: 56 },
  { key: "genres", label: "Genres", width: 78 },
] as const;

export type WebTableColumnKey = (typeof WEB_TABLE_COLUMNS)[number]["key"];

/** Reference column labels, in reference order. */
export const WEB_TABLE_COLUMN_LABELS: readonly string[] = WEB_TABLE_COLUMNS.map((c) => c.label);

/** Total pixel width of the reference column set (970px against a 971px table). */
export const WEB_TABLE_TOTAL_WIDTH: number = WEB_TABLE_COLUMNS.reduce((sum, c) => sum + c.width, 0);

/** Basename of a filesystem path, handling both separators. */
export function fileNameOf(path: string): string {
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] ?? path;
}

/**
 * Whether a lyrics sidecar exists for a track in the same directory.
 *
 * The browser runtime derives this from the directory listing rather than
 * from a per-track backend field, so the table reports real sidecar presence
 * without an extra round trip per row.
 */
export function hasSidecarLrc(trackPath: string, directoryNames: ReadonlySet<string>): boolean {
  const name = fileNameOf(trackPath);
  const stem = name.replace(/\.[^.]+$/, "");
  if (!stem) return false;
  return directoryNames.has(`${stem}.lrc`.toLowerCase());
}
