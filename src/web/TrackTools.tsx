/**
 * Fix-up tools for an album the workspace has loaded.
 *
 * These exist because background tagging can finish an album and still leave it
 * needing a person: the match was ambiguous, or no release was authoritative.
 * The desktop app has offered Search / Number / Convert for that job; these are
 * the same three, on the same shared logic and the same Rust writer, so an
 * album can be repaired where it was flagged instead of in another app.
 *
 * Scope follows the desktop: Number covers the whole open album, Convert covers
 * the current selection (falling back to the album when nothing is selected),
 * and Search works from the album's own artist and title. Number and Convert
 * reuse the desktop's pure modules (`shared/track-numbering`, `shared/convert`),
 * so no transform is reimplemented here.
 */

import React from "react";
import type { TrackData } from "../shared/desktop-api";
import {
  ORDERING_RULE_LABELS,
  computeNumberedTracks,
  type OrderingRule,
} from "../shared/track-numbering";
import { ConvertTool } from "./ConvertTool";
import { ToolDrawer, useToolRun } from "./ToolDrawer";
import { SearchTool } from "./SearchTool";

const NUMBERING_RULES = Object.keys(ORDERING_RULE_LABELS) as OrderingRule[];

export interface TrackToolsProps {
  /** The folder being viewed; tools act on this album. */
  albumPath: string | null;
  /** Every track in the open album, regardless of the table's filter. */
  tracks: TrackData[];
  /** The user's selection, which Convert prefers over the whole album. */
  selectedPaths?: readonly string[];
  /** Reload after a write so the table shows what was actually written. */
  onApplied: () => Promise<void> | void;
  /** Injected for tests; defaults to the installed facade. */
  api?: Window["api"];
}

export function TrackTools({
  albumPath,
  tracks,
  selectedPaths = [],
  onApplied,
  api = window.api,
}: TrackToolsProps) {
  const [open, setOpen] = React.useState<null | "search" | "number" | "convert">(null);
  const selected = React.useMemo(() => {
    const wanted = new Set(selectedPaths);
    return tracks.filter((track) => wanted.has(track.path));
  }, [selectedPaths, tracks]);
  // The desktop converts the selection and numbers the album; with no selection
  // Convert falls back to the album so the tool is usable from a plain folder.
  const convertScope = selected.length > 0 ? selected : tracks;
  const disabled = !albumPath || tracks.length === 0;

  return (
    <>
      <button
        type="button"
        className="sr-btn"
        data-testid="tools-search"
        disabled={disabled}
        title="Search MusicBrainz or Discogs for this album"
        onClick={() => setOpen("search")}
      >
        Find album metadata
      </button>
      <button
        type="button"
        className="sr-btn"
        data-testid="tools-number"
        disabled={disabled}
        title="Auto-number this album's tracks"
        onClick={() => setOpen("number")}
      >
        Number
      </button>
      <button
        type="button"
        className="sr-btn"
        data-testid="tools-convert"
        disabled={disabled}
        title="Rewrite file names and tags from a pattern"
        onClick={() => setOpen("convert")}
      >
        Convert
      </button>

      {open === "number" && albumPath && (
        <NumberDrawer
          tracks={tracks}
          api={api}
          onApplied={onApplied}
          onClose={() => setOpen(null)}
        />
      )}
      {open === "convert" && albumPath && (
        <ConvertTool
          tracks={convertScope}
          selectedCount={selected.length}
          api={api}
          onApplied={onApplied}
          onClose={() => setOpen(null)}
        />
      )}
      {open === "search" && albumPath && (
        <SearchTool
          albumPath={albumPath}
          tracks={tracks}
          api={api}
          onApplied={onApplied}
          onClose={() => setOpen(null)}
        />
      )}
    </>
  );
}

// ── Number ────────────────────────────────────────────────────

function NumberDrawer({
  tracks,
  api,
  onApplied,
  onClose,
}: {
  tracks: TrackData[];
  api: Window["api"];
  onApplied: () => Promise<void> | void;
  onClose: () => void;
}) {
  const [rule, setRule] = React.useState<OrderingRule>(NUMBERING_RULES[0]);
  const { busy, error, failures, message, run } = useToolRun();

  const apply = () =>
    run(async () => {
      const updates = computeNumberedTracks(
        tracks.map((track) => ({
          path: track.path,
          title: track.title,
          trackNumber: track.trackNumber,
          duration: track.duration,
        })),
        rule,
      );
      if (updates.length === 0) throw new Error("this folder has no tracks to number");
      const result = await api.writeTracks(updates);
      await onApplied();
      const failed = (result.failures ?? []).map(
        (failure) => `${failure.path}: ${failure.error}`,
      );
      return {
        message: `Numbered ${updates.length - failed.length} of ${updates.length} tracks.`,
        failures: failed,
      };
    });

  return (
    <ToolDrawer
      title="Number tracks"
      busy={busy}
      error={error}
      errors={failures}
      onClose={onClose}
      testId="number-drawer"
    >
      <section className="sr-section">
        <h3 className="sr-section__title">Ordering</h3>
        <div className="sr-field sr-field--stacked">
          <label className="sr-field__label" htmlFor="number-rule">
            Number tracks by
          </label>
          <select
            id="number-rule"
            className="sr-input"
            data-testid="number-rule"
            value={rule}
            onChange={(event) => setRule(event.target.value as OrderingRule)}
          >
            {NUMBERING_RULES.map((candidate) => (
              <option key={candidate} value={candidate}>
                {ORDERING_RULE_LABELS[candidate]}
              </option>
            ))}
          </select>
          <p className="sr-field__static">
            Writes a sequential track number and a total of {tracks.length} onto every
            track in this album.
          </p>
        </div>
        <button
          type="button"
          className="sr-btn sr-btn--primary"
          data-testid="number-apply"
          disabled={busy}
          onClick={() => void apply()}
        >
          {busy ? "Numbering…" : `Number ${tracks.length} tracks`}
        </button>
        {message && (
          <p className="sr-banner sr-banner--notice" data-testid="number-result">
            {message}
          </p>
        )}
      </section>
    </ToolDrawer>
  );
}
