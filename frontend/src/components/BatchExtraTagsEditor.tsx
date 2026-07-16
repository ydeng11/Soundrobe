import React, { useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import type { ExtraTag, TrackData } from "../shared/desktop-api";

interface BatchExtraTagsEditorProps {
  tracks: TrackData[];
  saving: boolean;
  onClose: () => void;
  onSave: (updates: Array<{ path: string; tags: Array<{ key: string; value: string }> }>) => Promise<void>;
}

interface DraftRow {
  id: string;
  key: string;
  value: string;
  /** Track paths that contributed this key/value pair. Empty = new tag. */
  origins: Set<string>;
}

export function BatchExtraTagsEditor({
  tracks,
  saving,
  onClose,
  onSave,
}: BatchExtraTagsEditorProps) {
  const [rows, setRows] = useState<DraftRow[]>([createNewRow()]);
  const [originalRows, setOriginalRows] = useState<DraftRow[]>([createNewRow()]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const newKeyRef = useRef<HTMLInputElement | null>(null);

  const trackCount = tracks.length;
  const trackPathsRef = useRef(tracks.map((t) => t.path));
  const originalTagsByPathRef = useRef(new Map<string, ExtraTag[]>());
  trackPathsRef.current = tracks.map((t) => t.path);

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);

    Promise.all(tracks.map((track) => window.api.readExtraTags(track.path))).then(
      (tagLists) => {
        if (cancelled) return;
        const paths = tracks.map((t) => t.path);
        const loadedRows = extraTagsToRows(tagLists, paths);
        const nextRows = loadedRows.length > 0 ? loadedRows : [createNewRow()];

        const originalByPath = new Map<string, ExtraTag[]>();
        for (let i = 0; i < paths.length; i++) {
          originalByPath.set(paths[i], tagLists[i]);
        }
        originalTagsByPathRef.current = originalByPath;

        setRows(nextRows);
        setOriginalRows(nextRows);
        setLoading(false);
      },
      (err: unknown) => {
        if (cancelled) return;
        originalTagsByPathRef.current = new Map();
        setRows([createNewRow()]);
        setOriginalRows([createNewRow()]);
        setError(err instanceof Error ? err.message : "Failed to read extra tags");
        setLoading(false);
      },
    );

    return () => {
      cancelled = true;
    };
  }, [tracks]);

  const dirty = useMemo(() => {
    return serializeRows(rows) !== serializeRows(originalRows);
  }, [rows, originalRows]);

  const requestClose = useCallback(() => {
    if (dirty && !window.confirm("You have unsaved changes. Discard them?")) {
      return;
    }
    onClose();
  }, [dirty, onClose]);

  const addRow = useCallback(() => {
    setRows((prev) => [...prev, createNewRow()]);
    requestAnimationFrame(() => newKeyRef.current?.focus());
  }, []);

  const updateRow = useCallback((id: string, patch: Partial<DraftRow>) => {
    setRows((prev) =>
      prev.map((row) => (row.id === id ? { ...row, ...patch } : row)),
    );
  }, []);

  const removeRow = useCallback((id: string) => {
    setRows((prev) => {
      const next = prev.filter((row) => row.id !== id);
      return next.length === 0 ? [createNewRow()] : next;
    });
  }, []);

  const handleSave = useCallback(async () => {
    const allPaths = trackPathsRef.current;

    // Build per-track tag arrays
    // - Rows with origins: only tracks in those origins get the tag
    // - Rows without origins (new tags): every selected track gets the tag
    // - Removed rows (in originalRows but not in rows): their origin tracks get empty tags to clear
    const trackTags = new Map<string, Array<{ key: string; value: string }>>();

    // First, process removed rows - their origin tracks should have those tags cleared
    const currentRowIds = new Set(rows.map((r) => r.id));
    for (const origRow of originalRows) {
      if (!currentRowIds.has(origRow.id) && origRow.key.trim() && origRow.value.trim()) {
        // This row was removed - add empty entry for its origin tracks
        for (const trackPath of origRow.origins) {
          // Only add if not already being modified by a current row
          if (!trackTags.has(trackPath)) {
            trackTags.set(trackPath, []);
          }
        }
      }
    }

    // Then, process current rows
    for (const row of rows) {
      const key = row.key.trim();
      const value = row.value.trim();
      if (!key || !value) continue;

      if (row.origins.size > 0) {
        // Existing tag: apply to its origin tracks only
        for (const trackPath of row.origins) {
          const list = trackTags.get(trackPath) ?? [];
          list.push({ key, value });
          trackTags.set(trackPath, list);
        }
      } else {
        // New tag: apply to all selected tracks
        for (const trackPath of allPaths) {
          const list = trackTags.get(trackPath) ?? [];
          list.push({ key, value });
          trackTags.set(trackPath, list);
        }
      }
    }

    // Build updates array — only include tracks that have actual changes
    const updates = allPaths
      .filter((path) => trackTags.has(path))
      .map((path) => ({
        path,
        tags: trackTags.get(path)!,
      }))
      .filter((update) => {
        const originalTags = originalTagsByPathRef.current.get(update.path) ?? [];
        return serializeTagListForComparison(update.tags)
          !== serializeTagListForComparison(originalTags);
      });

    if (updates.length > 0) {
      await onSave(updates);
    }

    const nextOriginalByPath = new Map(originalTagsByPathRef.current);
    for (const path of allPaths) {
      const nextTags = trackTags.get(path);
      if (!nextTags) continue;
      nextOriginalByPath.set(path, nextTags.map((tag) => ({ key: tag.key, value: tag.value, source: "batch" })));
    }
    originalTagsByPathRef.current = nextOriginalByPath;

    const savedRows = rowsToDraftRows(rows);
    setRows(savedRows);
    setOriginalRows(savedRows);
  }, [rows, onSave]);

  return (
    <div
      className="fixed inset-0 z-[80] flex items-center justify-center bg-black/20 px-6"
      role="dialog"
      aria-modal="true"
      aria-label="Batch Extra Tags"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) requestClose();
      }}
    >
      <div className="w-full max-w-[860px] max-h-[78vh] bg-white border border-border shadow-xl rounded-xl overflow-hidden flex flex-col">
        <div className="flex items-center gap-3 px-5 py-3.5 bg-surface-alt/70 border-b border-border">
          <div className="w-8 h-8 rounded-lg border border-border bg-white flex items-center justify-center text-text-muted">
            <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round">
              <rect x="3" y="3" width="18" height="18" rx="2" ry="2" />
              <line x1="3" y1="9" x2="21" y2="9" />
              <line x1="9" y1="21" x2="9" y2="9" />
            </svg>
          </div>
          <div className="min-w-0 flex-1">
            <h2 className="text-[13px] font-semibold text-text-primary">
              Batch Extra Tags
            </h2>
            <p className="text-[11px] text-text-muted tabular-nums">
              {trackCount} file{trackCount !== 1 ? "s" : ""} selected
            </p>
          </div>
        </div>

        <div className="px-5 py-3 bg-amber-50/80 border-b border-border text-[11px] text-amber-800 leading-relaxed">
          Tags shown here are grouped from all selected files. Tags that already exist are applied only to the files that originally had them; new tags (blank origin) are applied to <strong>all {trackCount} selected files</strong>.
        </div>

        <div className="grid grid-cols-[220px_1fr_44px] px-5 py-2 bg-white border-b border-border text-[10px] uppercase tracking-widest text-text-muted font-semibold">
          <div>Tag</div>
          <div>Value</div>
          <div />
        </div>

        <div className="flex-1 overflow-y-auto">
          {loading ? (
            <div className="h-32 flex items-center justify-center text-[12px] text-text-muted">
              Loading tags...
            </div>
          ) : error ? (
            <div className="h-32 flex items-center justify-center text-[12px] text-[#ff3b30]">
              {error}
            </div>
          ) : rows.length === 0 ? (
            <div className="h-32 flex items-center justify-center text-[12px] text-text-muted">
              No tags defined
            </div>
          ) : (
            rows.map((row) => (
              <BatchExtraTagRow
                key={row.id}
                row={row}
                trackCount={trackCount}
                newKeyRef={row === rows[rows.length - 1] ? newKeyRef : undefined}
                onUpdate={(patch) => updateRow(row.id, patch)}
                onRemove={() => removeRow(row.id)}
              />
            ))
          )}
        </div>

        <div className="flex items-center justify-between gap-3 px-5 py-3 bg-surface-alt/50 border-t border-border">
          <button
            type="button"
            onClick={addRow}
            className="inline-flex items-center gap-2 px-3 py-1.5 rounded-lg text-[12px] font-medium text-accent hover:bg-accent/10 transition-colors"
          >
            <span className="text-[15px] leading-none">+</span>
            Add Tag
          </button>
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={requestClose}
              disabled={saving}
              className="px-3 py-1.5 rounded-lg text-[12px] font-medium text-text-secondary hover:bg-surface-hover disabled:opacity-50"
            >
              Cancel
            </button>
            <button
              type="button"
              onClick={handleSave}
              disabled={!dirty || saving || loading}
              className="px-3 py-1.5 rounded-lg text-[12px] font-medium bg-accent text-white hover:bg-accent/90 disabled:opacity-40 disabled:cursor-not-allowed"
            >
              {saving ? "Saving..." : `Apply to ${trackCount} file${trackCount !== 1 ? "s" : ""}`}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

// ── Row sub-component ─────────────────────────────────────────

function BatchExtraTagRow({
  row,
  trackCount,
  newKeyRef,
  onUpdate,
  onRemove,
}: {
  row: DraftRow;
  trackCount: number;
  newKeyRef?: React.Ref<HTMLInputElement>;
  onUpdate: (patch: Partial<DraftRow>) => void;
  onRemove: () => void;
}) {
  const keyId = useId();

  const originCount = row.origins.size;
  const hasOrigin = originCount > 0;

  return (
    <div className="group grid grid-cols-[220px_1fr_44px] items-center gap-0 px-5 min-h-[42px] border-b border-border/40 bg-white">
      <div className="flex items-center gap-1.5">
        <input
          ref={newKeyRef}
          value={row.key}
          onChange={(event) => onUpdate({ key: event.target.value })}
          className="h-8 bg-transparent border border-transparent rounded-md px-2 text-[12px] font-medium outline-none focus:border-accent/60 focus:bg-white focus:shadow-[0_0_0_3px_rgba(0,122,255,0.14)] flex-1 min-w-0"
          placeholder="Tag key (e.g. MUSICBRAINZ_ALBUMID)"
          list={keyId}
        />
        {hasOrigin && (
          <span
            className="shrink-0 text-[10px] text-text-muted/60 font-mono"
            title={`Present in ${originCount} of ${trackCount} file(s)`}
          >
            {originCount}/{trackCount}
          </span>
        )}
      </div>
      <input
        value={row.value}
        onChange={(event) => onUpdate({ value: event.target.value })}
        className="h-8 bg-transparent border border-transparent rounded-md px-2 text-[12px] outline-none focus:border-accent/60 focus:bg-white focus:shadow-[0_0_0_3px_rgba(0,122,255,0.14)]"
        placeholder="Value"
      />
      <button
        type="button"
        onClick={onRemove}
        className="w-8 h-8 rounded-md flex items-center justify-center text-text-muted opacity-0 group-hover:opacity-100 hover:bg-red-50 hover:text-[#ff3b30] transition-all focus:opacity-100"
        aria-label="Remove tag"
        title="Remove tag"
      >
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
          <polyline points="3 6 5 6 21 6" />
          <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6" />
          <path d="M10 11v6" />
          <path d="M14 11v6" />
          <path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
        </svg>
      </button>
      <datalist id={keyId}>
        <option value="MUSICBRAINZ_ALBUMID" />
        <option value="MUSICBRAINZ_ARTISTID" />
        <option value="MUSICBRAINZ_TRACKID" />
        <option value="DISCOGS_ARTIST_ID" />
        <option value="DISCOGS_RELEASE_ID" />
        <option value="BARCODE" />
        <option value="CATALOGNUMBER" />
        <option value="LABEL" />
        <option value="ISRC" />
        <option value="RELEASETYPE" />
        <option value="MEDIA" />
        <option value="RATING" />
        <option value="ASIN" />
        <option value="SCRIPT" />
        <option value="LANGUAGE" />
        <option value="ALBUMARTISTS" />
        <option value="COMMENT" />
        <option value="DESCRIPTION" />
        <option value="COMPILATION" />
      </datalist>
    </div>
  );
}

let idCounter = 0;
function createNewRow(): DraftRow {
  return { id: `batch-extra-${++idCounter}`, key: "", value: "", origins: new Set() };
}

/**
 * Combine extra tags from multiple tracks into grouped rows.
 * Identical key+value pairs from different tracks are deduped
 * but track which origin paths they came from.
 */
function extraTagsToRows(
  tagLists: ExtraTag[][],
  trackPaths: string[],
): DraftRow[] {
  // Map: normalizedKey\0value → { origins: Set<trackPath>, originalKey: string }
  const grouped = new Map<string, { origins: Set<string>; originalKey: string }>();

  for (let i = 0; i < tagLists.length; i++) {
    const trackPath = trackPaths[i];
    const tags = tagLists[i];

    for (const tag of tags) {
      const key = tag.key.trim();
      const value = tag.value.trim();
      if (!key || !value) continue;

      const identity = `${key.toUpperCase()}\0${value}`;
      let entry = grouped.get(identity);
      if (!entry) {
        entry = { origins: new Set(), originalKey: key };
        grouped.set(identity, entry);
      }
      entry.origins.add(trackPath);
    }
  }

  const rows: DraftRow[] = [];
  for (const [identity, entry] of grouped) {
    const nullIdx = identity.indexOf("\0");
    const value = identity.slice(nullIdx + 1);
    rows.push({
      id: `batch-extra-${++idCounter}`,
      key: entry.originalKey,
      value,
      origins: new Set(entry.origins),
    });
  }

  return rows.sort((a, b) => a.key.localeCompare(b.key));
}

function rowsToDraftRows(rows: DraftRow[]): DraftRow[] {
  return rows
    .map((row) => ({
      id: `batch-extra-${++idCounter}`,
      key: row.key,
      value: row.value,
      origins: new Set(row.origins),
    }));
}

function serializeRows(rows: DraftRow[]): string {
  return JSON.stringify(
    rows
      .map((row) => ({
        key: row.key.trim(),
        value: row.value.trim(),
        origins: Array.from(row.origins).sort(),
      }))
      .filter((row) => row.key && row.value)
      .sort((a, b) => {
        const keyCmp = a.key.localeCompare(b.key);
        return keyCmp || a.value.localeCompare(b.value);
      }),
  );
}

function serializeTagListForComparison(tags: Array<{ key: string; value: string }>): string {
  const seen = new Set<string>();
  const normalized: Array<{ key: string; value: string }> = [];

  for (const tag of tags) {
    const key = tag.key.trim().toUpperCase();
    const value = tag.value.trim();
    if (!key || !value) continue;

    const identity = `${key}\0${value}`;
    if (seen.has(identity)) continue;
    seen.add(identity);
    normalized.push({ key, value });
  }

  normalized.sort((a, b) => {
    const keyCmp = a.key.localeCompare(b.key);
    return keyCmp || a.value.localeCompare(b.value);
  });

  return JSON.stringify(normalized);
}
