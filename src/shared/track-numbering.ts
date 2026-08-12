/**
 * TrackNumberingService — auto-numbers tracks within an album
 * according to user-selectable ordering rules.
 *
 * Pure functions only; no side effects, no file I/O.
 */

/** Ordering rules for track numbering. */
export type OrderingRule =
  | "filename-asc"
  | "filename-desc"
  | "title-asc"
  | "title-desc"
  | "existing-track-asc"
  | "existing-track-desc"
  | "duration-asc"
  | "duration-desc";

/** Human-readable label for each ordering rule. */
export const ORDERING_RULE_LABELS: Record<OrderingRule, string> = {
  "filename-asc": "By filename (A-Z)",
  "filename-desc": "By filename (Z-A)",
  "title-asc": "By title (A-Z)",
  "title-desc": "By title (Z-A)",
  "existing-track-asc": "By existing track # (asc)",
  "existing-track-desc": "By existing track # (desc)",
  "duration-asc": "By duration (short→long)",
  "duration-desc": "By duration (long→short)",
};

/** Input shape for a single track to be numbered. */
export interface NumberingInput {
  path: string;
  title: string | null;
  trackNumber: number | null;
  duration: number;
}

/** Output shape: path + fields to write via batch-write. */
export interface NumberingUpdate {
  path: string;
  fields: { trackNumber: number; trackTotal: number };
}

/**
 * Compute sequential track numbers for a set of tracks based on an ordering rule.
 *
 * @param tracks - The tracks in the album to number.
 * @param rule   - The ordering rule to apply.
 * @param startFrom - First track number (default: 1).
 * @returns Array of updates suitable for `window.api.writeTracks()`.
 */
export function computeNumberedTracks(
  tracks: NumberingInput[],
  rule: OrderingRule,
  startFrom: number = 1,
): NumberingUpdate[] {
  if (tracks.length === 0) return [];

  const sorted = [...tracks].sort(buildComparator(rule));

  return sorted.map((track, index) => ({
    path: track.path,
    fields: {
      trackNumber: index + startFrom,
      trackTotal: tracks.length,
    },
  }));
}

// ── Helpers ───────────────────────────────────────────────────

/** Browser-safe basename: everything after the last `/` or `\\`. */
function basename(p: string): string {
  return p.replace(/[/\\]$/, "").split(/[/\\]/).pop() ?? p;
}

// ── Comparator factory ───────────────────────────────────────

function buildComparator(
  rule: OrderingRule,
): (a: NumberingInput, b: NumberingInput) => number {
  switch (rule) {
    case "filename-asc":
      return (a, b) => basename(a.path).localeCompare(basename(b.path));
    case "filename-desc":
      return (a, b) => basename(b.path).localeCompare(basename(a.path));

    case "title-asc":
      return (a, b) => {
        const ta = a.title ?? basename(a.path);
        const tb = b.title ?? basename(b.path);
        return ta.localeCompare(tb);
      };
    case "title-desc":
      return (a, b) => {
        const ta = a.title ?? basename(a.path);
        const tb = b.title ?? basename(b.path);
        return tb.localeCompare(ta);
      };

    case "existing-track-asc":
      return (a, b) => {
        const na = a.trackNumber ?? 9999;
        const nb = b.trackNumber ?? 9999;
        if (na !== nb) return na - nb;
        return basename(a.path).localeCompare(basename(b.path));
      };
    case "existing-track-desc":
      return (a, b) => {
        const na = a.trackNumber ?? -1;
        const nb = b.trackNumber ?? -1;
        if (na !== nb) return nb - na;
        return basename(a.path).localeCompare(basename(b.path));
      };

    case "duration-asc":
      return (a, b) => {
        if (a.duration !== b.duration) return a.duration - b.duration;
        return basename(a.path).localeCompare(basename(b.path));
      };
    case "duration-desc":
      return (a, b) => {
        if (a.duration !== b.duration) return b.duration - a.duration;
        return basename(a.path).localeCompare(basename(b.path));
      };

    default:
      return () => 0;
  }
}
