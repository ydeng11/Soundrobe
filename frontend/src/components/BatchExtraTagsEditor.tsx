import React, { useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import type { ExtraTag, TrackData } from "../../electron/preload";

interface BatchExtraTagsEditorProps {
  tracks: TrackData[];
  saving: boolean;
  onClose: () => void;
  onSave: (tags: Array<{ key: string; value: string }>) => Promise<void>;
}

interface DraftRow {
  id: string;
  key: string;
  value: string;
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

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);

    Promise.all(tracks.map((track) => window.api.readExtraTags(track.path))).then(
      (tagLists) => {
        if (cancelled) return;
        const loadedRows = extraTagsToRows(tagLists.flat());
        const nextRows = loadedRows.length > 0 ? loadedRows : [createNewRow()];
        setRows(nextRows);
        setOriginalRows(nextRows);
        setLoading(false);
      },
      (err: unknown) => {
        if (cancelled) return;
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
    const tags = rows
      .map((row) => ({ key: row.key.trim(), value: row.value.trim() }))
      .filter((row) => row.key && row.value);

    await onSave(tags);
    const savedRows = tags.length > 0 ? tagsToRows(tags) : [createNewRow()];
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
          Tags shown here are the combined extra tags from all selected files and will be <strong>set on all {trackCount} selected files</strong>.
          Leave a row empty to skip it.
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
  newKeyRef,
  onUpdate,
  onRemove,
}: {
  row: DraftRow;
  newKeyRef?: React.Ref<HTMLInputElement>;
  onUpdate: (patch: Partial<DraftRow>) => void;
  onRemove: () => void;
}) {
  const keyId = useId();

  return (
    <div className="group grid grid-cols-[220px_1fr_44px] items-center gap-0 px-5 min-h-[42px] border-b border-border/40 bg-white">
      <input
        ref={newKeyRef}
        value={row.key}
        onChange={(event) => onUpdate({ key: event.target.value })}
        className="h-8 bg-transparent border border-transparent rounded-md px-2 text-[12px] font-medium outline-none focus:border-accent/60 focus:bg-white focus:shadow-[0_0_0_3px_rgba(0,122,255,0.14)]"
        placeholder="Tag key (e.g. MUSICBRAINZ_ALBUMID)"
        list={keyId}
      />
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
      </datalist>
    </div>
  );
}

let idCounter = 0;
function createNewRow(): DraftRow {
  return { id: `batch-extra-${++idCounter}`, key: "", value: "" };
}

function extraTagsToRows(tags: ExtraTag[]): DraftRow[] {
  const seen = new Set<string>();
  const rows: DraftRow[] = [];

  for (const tag of tags) {
    const key = tag.key.trim();
    const value = tag.value.trim();
    const identity = `${key.toUpperCase()}\0${value}`;
    if (!key || !value || seen.has(identity)) continue;
    seen.add(identity);
    rows.push({ id: `batch-extra-${++idCounter}`, key, value });
  }

  return rows.sort((a, b) => a.key.localeCompare(b.key));
}

function tagsToRows(tags: Array<{ key: string; value: string }>): DraftRow[] {
  return tags.map((tag) => ({
    id: `batch-extra-${++idCounter}`,
    key: tag.key,
    value: tag.value,
  }));
}

function serializeRows(rows: DraftRow[]): string {
  return JSON.stringify(
    rows
      .map((row) => ({ key: row.key.trim(), value: row.value.trim() }))
      .filter((row) => row.key && row.value)
      .sort((a, b) => {
        const keyCmp = a.key.localeCompare(b.key);
        return keyCmp || a.value.localeCompare(b.value);
      }),
  );
}
