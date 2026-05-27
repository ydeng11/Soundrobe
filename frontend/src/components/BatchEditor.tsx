import React, { useCallback, useId, useMemo, useState } from "react";
import type { TrackData } from "../../electron/preload";

interface BatchEditorProps {
  tracks: TrackData[];
  coverDataUrl: string | null;
  saving: boolean;
  onSave: (fields: Record<string, string>) => void;
}

const BATCH_FIELDS: { key: string; label: string; placeholder: string }[] = [
  { key: "artist", label: "Artist", placeholder: "Common artist…" },
  { key: "album", label: "Album", placeholder: "Common album…" },
  { key: "albumArtist", label: "Album Artist", placeholder: "Common album artist…" },
  { key: "genre", label: "Genre", placeholder: "Common genre…" },
  { key: "year", label: "Year", placeholder: "2024" },
  { key: "comment", label: "Comment", placeholder: "Common comment…" },
];

export function BatchEditor({
  tracks,
  coverDataUrl,
  saving,
  onSave,
}: BatchEditorProps) {
  // Build suggestion lists from the selected tracks
  const suggestions = useMemo(() => {
    const map: Record<string, Set<string>> = {};
    for (const f of BATCH_FIELDS) {
      map[f.key] = new Set<string>();
    }
    for (const t of tracks) {
      if (t.artist) map.artist.add(t.artist);
      if (t.album) map.album.add(t.album);
      if (t.albumArtist) map.albumArtist.add(t.albumArtist);
      if (t.genre) map.genre.add(t.genre);
      if (t.year) map.year.add(t.year);
      if (t.comment) map.comment.add(t.comment);
    }
    const sorted: Record<string, string[]> = {};
    for (const f of BATCH_FIELDS) {
      sorted[f.key] = [...map[f.key]].sort();
    }
    return sorted;
  }, [tracks]);

  const [values, setValues] = useState<Record<string, string>>({});

  const setField = useCallback((key: string, value: string) => {
    setValues((prev) => ({ ...prev, [key]: value }));
  }, []);

  const handleApply = useCallback(() => {
    // Only send non-empty values
    const fields: Record<string, string> = {};
    for (const [key, value] of Object.entries(values)) {
      if (value.trim()) {
        fields[key] = value.trim();
      }
    }
    if (Object.keys(fields).length === 0) return;
    onSave(fields);
    setValues({}); // clear after apply
  }, [values, onSave]);

  const trackCount = tracks.length;
  const primary = tracks[0];

  // Collect which fields differ across selection for informative display
  const differing = useMemo(() => {
    const map: Record<string, Set<string>> = {};
    for (const f of BATCH_FIELDS) {
      map[f.key] = new Set<string>();
    }
    for (const t of tracks) {
      for (const f of BATCH_FIELDS) {
        const val = (t as unknown as Record<string, unknown>)[f.key];
        if (typeof val === "string") map[f.key].add(val);
      }
    }
    const diff = new Set<string>();
    for (const f of BATCH_FIELDS) {
      if (map[f.key].size > 1) diff.add(f.key);
    }
    return diff;
  }, [tracks]);

  return (
    <div className="flex flex-col h-full overflow-y-auto bg-white border-l border-border">
      {/* Header */}
      <div className="px-5 py-3.5 bg-surface-alt/40 border-b border-border/60">
        <div className="flex items-center gap-2.5">
          <div className="flex-1 min-w-0">
            <div className="text-[12px] font-medium text-text-primary truncate">
              Batch Edit
            </div>
            <div className="text-[10px] text-text-muted mt-0.5 tabular-nums">
              {trackCount} files selected
            </div>
          </div>
          {saving && (
            <span className="flex items-center gap-1.5 text-[10px] text-accent font-medium">
              <span className="w-1.5 h-1.5 rounded-full bg-accent animate-pulse" />
              Saving
            </span>
          )}
        </div>
      </div>

      <div className="flex-1 px-5 py-4 space-y-5">
        {/* Cover art preview */}
        <div>
          <div className="w-full aspect-square max-w-[220px] mx-auto rounded-xl overflow-hidden bg-surface-alt border border-border shadow-sm">
            {coverDataUrl ? (
              <img
                src={coverDataUrl}
                alt="Cover art"
                className="w-full h-full object-cover"
              />
            ) : (
              <div className="w-full h-full flex items-center justify-center">
                <div className="flex flex-col items-center gap-2 text-text-muted">
                  <svg width="40" height="40" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1" strokeLinecap="round" strokeLinejoin="round" className="opacity-40">
                    <path d="M9 18V5l12-2v13" />
                    <circle cx="6" cy="18" r="3" />
                    <circle cx="18" cy="16" r="3" />
                  </svg>
                  <span className="text-[11px]">No cover art</span>
                </div>
              </div>
            )}
          </div>
        </div>

        {/* Selection summary */}
        <div className="text-[11px] text-text-muted leading-relaxed px-1">
          Set common values for all {trackCount} selected files.
          <br />
          Fields left empty won't be changed.
        </div>

        {/* Batch fields with suggestions */}
        <div className="space-y-3">
          {BATCH_FIELDS.map((f) => (
            <BatchField
              key={f.key}
              label={f.label}
              placeholder={f.placeholder}
              value={values[f.key] ?? ""}
              suggestions={suggestions[f.key]}
              hasDiffering={differing.has(f.key)}
              onChange={(v) => setField(f.key, v)}
            />
          ))}
        </div>

        {/* Apply button */}
        <button
          onClick={handleApply}
          disabled={saving || Object.values(values).every((v) => !v.trim())}
          className="w-full inline-flex items-center justify-center gap-2 px-4 py-2.5 text-[12px] font-semibold rounded-xl bg-accent text-white hover:bg-accent/90 disabled:opacity-40 disabled:cursor-not-allowed transition-all shadow-sm active:scale-[0.97]"
        >
          {saving ? (
            <>
              <span className="w-3.5 h-3.5 rounded-full border-2 border-white/30 border-t-white animate-spin" />
              Saving…
            </>
          ) : (
            <>
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <polyline points="20 6 9 17 4 12" />
              </svg>
              Apply to {trackCount} file{trackCount !== 1 ? "s" : ""}
            </>
          )}
        </button>
      </div>
    </div>
  );
}

// ── Batch Field sub-component ──────────────────────────────────

function BatchField({
  label,
  placeholder,
  value,
  suggestions,
  hasDiffering,
  onChange,
}: {
  label: string;
  placeholder: string;
  value: string;
  suggestions: string[];
  hasDiffering: boolean;
  onChange: (v: string) => void;
}) {
  const id = useId();
  const listId = useId();

  return (
    <div>
      <div className="flex items-center gap-2 mb-1.5">
        <label
          htmlFor={id}
          className="text-[10px] font-semibold uppercase tracking-widest text-text-muted"
        >
          {label}
        </label>
        {hasDiffering && (
          <span className="text-[9px] text-amber-600/70 bg-amber-50 px-1.5 py-0.5 rounded-full font-medium">
            mixed
          </span>
        )}
      </div>
      <div className="relative">
        <input
          id={id}
          type="text"
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder={hasDiffering ? `${placeholder} (mixed values)` : placeholder}
          list={listId}
          className="w-full bg-white border border-border rounded-lg px-3 py-1.5 text-[12px] text-text-primary placeholder-text-muted/40 outline-none transition-all focus:border-accent/60 focus:shadow-[0_0_0_3px_rgba(0,122,255,0.2)]"
        />
        {suggestions.length > 0 && (
          <datalist id={listId}>
            {suggestions.map((s) => (
              <option key={s} value={s} />
            ))}
          </datalist>
        )}
      </div>
    </div>
  );
}
