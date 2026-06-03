import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { ExtraTag, TrackData } from "../../electron/preload";
import { basename } from "../utils/path";

interface ExtraTagsEditorProps {
  track: TrackData;
  saving: boolean;
  onClose: () => void;
  onSave: (tags: Array<{ key: string; value: string }>) => Promise<void>;
}

interface DraftRow {
  id: string;
  key: string;
  value: string;
  source: string;
  deleted: boolean;
  isNew: boolean;
}

export function ExtraTagsEditor({
  track,
  saving,
  onClose,
  onSave,
}: ExtraTagsEditorProps) {
  const [rows, setRows] = useState<DraftRow[]>([]);
  const [original, setOriginal] = useState<DraftRow[]>([]);
  const [filter, setFilter] = useState("");
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const newKeyRef = useRef<HTMLInputElement | null>(null);

  const filename = basename(track.path) ?? track.path;

  useEffect(() => {
    let cancelled = false;
    setLoading(true);
    setError(null);

    window.api.readExtraTags(track.path).then(
      (tags) => {
        if (cancelled) return;
        const next = tags.map(extraTagToRow);
        setRows(next);
        setOriginal(next);
        setLoading(false);
      },
      (err: unknown) => {
        if (cancelled) return;
        setError(err instanceof Error ? err.message : "Failed to read tags");
        setLoading(false);
      },
    );

    return () => {
      cancelled = true;
    };
  }, [track.path]);

  const dirty = useMemo(
    () => serializeRows(rows) !== serializeRows(original),
    [rows, original],
  );

  const visibleRows = useMemo(() => {
    const query = filter.trim().toLowerCase();
    if (!query) return rows;
    return rows.filter(
      (row) =>
        row.key.toLowerCase().includes(query) ||
        row.value.toLowerCase().includes(query) ||
        row.source.toLowerCase().includes(query),
    );
  }, [rows, filter]);

  const requestClose = useCallback(() => {
    if (dirty && !window.confirm("You have unsaved changes. Discard them?")) {
      return;
    }
    onClose();
  }, [dirty, onClose]);

  useEffect(() => {
    const handler = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        requestClose();
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [requestClose]);

  const addRow = () => {
    const id = `new-${Date.now()}-${Math.random().toString(16).slice(2)}`;
    setRows((prev) => [
      ...prev,
      { id, key: "", value: "", source: "Custom", deleted: false, isNew: true },
    ]);
    requestAnimationFrame(() => newKeyRef.current?.focus());
  };

  const updateRow = (id: string, patch: Partial<DraftRow>) => {
    setRows((prev) =>
      prev.map((row) => (row.id === id ? { ...row, ...patch } : row)),
    );
  };

  const markDeleted = (id: string) => {
    setRows((prev) =>
      prev.map((row) =>
        row.id === id ? { ...row, deleted: !row.deleted } : row,
      ),
    );
  };

  const handleSave = async () => {
    const tags = rows
      .filter((row) => !row.deleted)
      .map((row) => ({ key: row.key.trim(), value: row.value.trim() }))
      .filter((row) => row.key && row.value);

    await onSave(tags);
    const saved = tags.map((tag, index) => ({
      id: `saved-${index}-${tag.key}`,
      key: tag.key,
      value: tag.value,
      source: "Custom",
      deleted: false,
      isNew: false,
    }));
    setRows(saved);
    setOriginal(saved);
  };

  return (
    <div
      className="fixed inset-0 z-[80] flex items-center justify-center bg-black/20 px-6"
      role="dialog"
      aria-modal="true"
      aria-label="Extra Tags"
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) requestClose();
      }}
    >
      <div className="w-full max-w-[860px] max-h-[78vh] bg-white border border-border shadow-xl rounded-xl overflow-hidden flex flex-col">
        <div className="flex items-center gap-3 px-5 py-3.5 bg-surface-alt/70 border-b border-border">
          <div className="w-8 h-8 rounded-lg border border-border bg-white flex items-center justify-center text-text-muted">
            <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.7" strokeLinecap="round" strokeLinejoin="round">
              <path d="M9 18V5l12-2v13" />
              <circle cx="6" cy="18" r="3" />
              <circle cx="18" cy="16" r="3" />
            </svg>
          </div>
          <div className="min-w-0 flex-1">
            <h2 className="text-[13px] font-semibold text-text-primary truncate">
              Extra Tags
            </h2>
            <p className="text-[11px] text-text-muted truncate">{filename}</p>
          </div>
          <label className="relative w-[240px]">
            <span className="sr-only">Filter tags</span>
            <input
              value={filter}
              onChange={(event) => setFilter(event.target.value)}
              className="w-full rounded-lg border border-border bg-white px-3 py-1.5 text-[12px] outline-none focus:border-accent/60 focus:shadow-[0_0_0_3px_rgba(0,122,255,0.16)]"
              placeholder="Search tags"
            />
          </label>
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
          ) : visibleRows.length === 0 ? (
            <div className="h-32 flex items-center justify-center text-[12px] text-text-muted">
              No extra tags
            </div>
          ) : (
            visibleRows.map((row) => (
              <div
                key={row.id}
                className={`group grid grid-cols-[220px_1fr_44px] items-center gap-0 px-5 min-h-[42px] border-b border-border/40 ${
                  row.deleted ? "bg-red-50/80 text-[#c8271d]" : "bg-white"
                }`}
              >
                <input
                  ref={row.isNew ? newKeyRef : undefined}
                  value={row.key}
                  onChange={(event) => updateRow(row.id, { key: event.target.value })}
                  className={`h-8 bg-transparent border border-transparent rounded-md px-2 text-[12px] font-medium outline-none focus:border-accent/60 focus:bg-white focus:shadow-[0_0_0_3px_rgba(0,122,255,0.14)] ${
                    row.deleted ? "line-through" : ""
                  }`}
                  placeholder="Tag key"
                />
                <input
                  value={row.value}
                  onChange={(event) => updateRow(row.id, { value: event.target.value })}
                  className={`h-8 bg-transparent border border-transparent rounded-md px-2 text-[12px] outline-none focus:border-accent/60 focus:bg-white focus:shadow-[0_0_0_3px_rgba(0,122,255,0.14)] ${
                    row.deleted ? "line-through" : ""
                  }`}
                  placeholder="Value"
                />
                <button
                  type="button"
                  onClick={() => markDeleted(row.id)}
                  className="w-8 h-8 rounded-md flex items-center justify-center text-text-muted opacity-0 group-hover:opacity-100 hover:bg-red-50 hover:text-[#ff3b30] transition-all focus:opacity-100"
                  aria-label={row.deleted ? "Restore tag" : "Delete tag"}
                  title={row.deleted ? "Restore tag" : "Delete tag"}
                >
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round">
                    {row.deleted ? (
                      <path d="M3 7v6h6" />
                    ) : (
                      <>
                        <polyline points="3 6 5 6 21 6" />
                        <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6" />
                        <path d="M10 11v6" />
                        <path d="M14 11v6" />
                        <path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
                      </>
                    )}
                  </svg>
                </button>
              </div>
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
            Add Custom Tag
          </button>
          <div className="flex items-center gap-2">
            {dirty && (
              <span className="text-[11px] text-[#ff9f0a] font-medium mr-2">
                Unsaved changes
              </span>
            )}
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
              disabled={!dirty || saving}
              className="px-3 py-1.5 rounded-lg text-[12px] font-medium bg-accent text-white hover:bg-accent/90 disabled:opacity-40 disabled:cursor-not-allowed"
            >
              {saving ? "Saving..." : "Save Changes"}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}

function extraTagToRow(tag: ExtraTag, index: number): DraftRow {
  return {
    id: `${tag.source}-${index}-${tag.key}`,
    key: tag.key,
    value: tag.value,
    source: tag.source,
    deleted: false,
    isNew: false,
  };
}

function serializeRows(rows: DraftRow[]): string {
  return JSON.stringify(
    rows.map((row) => ({
      key: row.key.trim(),
      value: row.value.trim(),
      deleted: row.deleted,
    })),
  );
}
