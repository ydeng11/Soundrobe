import React, { useCallback, useId, useMemo, useState, useRef, useEffect } from "react";
import type { TrackData } from "../shared/desktop-api";

interface BatchEditorProps {
  tracks: TrackData[];
  coverDataUrl: string | null;
  saving: boolean;
  onSave: (fields: Record<string, string>) => void;
  onChangeCover?: () => void;
  onRemoveCover?: () => void;
  onDownloadCover?: () => Promise<void>;
  onDownloadArtistArt?: () => Promise<void>;
}

const BATCH_FIELDS: { key: string; label: string; placeholder: string }[] = [
  { key: "artist", label: "Artist", placeholder: "Common artist…" },
  { key: "album", label: "Album", placeholder: "Common album…" },
  { key: "albumArtist", label: "Album Artist", placeholder: "Common album artist…" },
  { key: "genre", label: "Genre", placeholder: "Common genre…" },
  { key: "year", label: "Year", placeholder: "2024" },
  { key: "disc", label: "Disc", placeholder: "Disc number (e.g. 1 or 1/2)" },
];

export function BatchEditor({
  tracks,
  coverDataUrl,
  saving,
  onSave,
  onChangeCover,
  onRemoveCover,
  onDownloadCover,
  onDownloadArtistArt,
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
    }
    // Special handling for disc: combine discNumber + discTotal
    for (const t of tracks) {
      if (t.discNumber != null) {
        const discValue = t.discTotal != null
          ? `${t.discNumber}/${t.discTotal}`
          : String(t.discNumber);
        map.disc.add(discValue);
      }
    }

    const sorted: Record<string, string[]> = {};
    for (const f of BATCH_FIELDS) {
      sorted[f.key] = [...map[f.key]].sort();
    }
    return sorted;
  }, [tracks]);

  const [values, setValues] = useState<Record<string, string>>({});
  const [dirty, setDirty] = useState(false);
  const panelRef = useRef<HTMLDivElement>(null);
  const valuesRef = useRef<Record<string, string>>({});

  const selectionKey = tracks.map((track) => track.path).sort().join("\n");

  // Reset only when the selection changes. Readback updates should not erase a draft.
  useEffect(() => {
    setValues({});
    valuesRef.current = {};
    setDirty(false);
  }, [selectionKey]);

  // Flush pending changes to disk
  const flushChanges = useCallback(() => {
    const currentValues = valuesRef.current;
    const fields: Record<string, string> = {};
    for (const [key, value] of Object.entries(currentValues)) {
      fields[key] = value.trim();
    }
    if (Object.keys(fields).length > 0) {
      onSave(fields);
    }
    setValues({});
    valuesRef.current = {};
    setDirty(false);
  }, [onSave]);

  // Save when focus leaves the panel
  const handleBlur = useCallback(
    (e: React.FocusEvent) => {
      if (
        panelRef.current &&
        !panelRef.current.contains(e.relatedTarget as Node)
      ) {
        flushChanges();
      }
    },
    [flushChanges],
  );

  const setField = useCallback((key: string, value: string) => {
    setValues((prev) => ({ ...prev, [key]: value }));
    valuesRef.current = { ...valuesRef.current, [key]: value };
    setDirty(true);
  }, []);

  const trackCount = tracks.length;

  // Collect which fields differ across selection for informative display
  const differing = useMemo(() => {
    const map: Record<string, Set<string>> = {};
    for (const f of BATCH_FIELDS) {
      map[f.key] = new Set<string>();
    }
    for (const t of tracks) {
      for (const f of BATCH_FIELDS) {
        // Special handling for disc: derived from discNumber + discTotal
        if (f.key === "disc") {
          if (t.discNumber != null) {
            const discValue = t.discTotal != null
              ? `${t.discNumber}/${t.discTotal}`
              : String(t.discNumber);
            map[f.key].add(discValue);
          }
        } else {
          const val = (t as unknown as Record<string, unknown>)[f.key];
          if (typeof val === "string") map[f.key].add(val);
        }
      }
    }
    const diff = new Set<string>();
    for (const f of BATCH_FIELDS) {
      if (map[f.key].size > 1) diff.add(f.key);
    }
    return diff;
  }, [tracks]);

  return (
    <div
      ref={panelRef}
      className="flex flex-col h-full overflow-y-auto bg-white border-l border-border"
      onBlur={handleBlur}
    >
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
          {(saving || dirty) && (
            <span className="flex items-center gap-1.5 text-[10px] font-medium">
              {saving ? (
                <>
                  <span className="w-1.5 h-1.5 rounded-full bg-accent animate-pulse" />
                  <span className="text-accent">Saving</span>
                </>
              ) : (
                <>
                  <span className="w-1.5 h-1.5 rounded-full bg-[#ff9f0a]" />
                  <span className="text-[#ff9f0a]">Unsaved</span>
                </>
              )}
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
          {(onChangeCover || onDownloadCover || onDownloadArtistArt || (coverDataUrl && onRemoveCover)) && (
            <div className="flex gap-2 mt-2.5 justify-center flex-wrap">
              {onChangeCover && (
                <button
                  aria-label="Change cover"
                  onClick={onChangeCover}
                  className="px-3 py-1.5 text-[11px] font-medium rounded-lg bg-accent text-white hover:bg-accent/90"
                >
                  Change
                </button>
              )}
              {onDownloadCover && (
                <button
                  aria-label="Download cover"
                  onClick={onDownloadCover}
                  className="px-3 py-1.5 text-[11px] font-medium rounded-lg bg-[#34c759] text-white hover:bg-[#30b753]"
                >
                  Download
                </button>
              )}
              {onDownloadArtistArt && (
                <button
                  aria-label="Download artist image"
                  onClick={onDownloadArtistArt}
                  className="px-3 py-1.5 text-[11px] font-medium rounded-lg bg-[#34c759] text-white hover:bg-[#30b753]"
                >
                  Artist
                </button>
              )}
              {coverDataUrl && onRemoveCover && (
                <button
                  aria-label="Remove cover"
                  onClick={onRemoveCover}
                  className="px-3 py-1.5 text-[11px] font-medium rounded-lg text-[#ff3b30] hover:bg-red-50"
                >
                  Remove
                </button>
              )}
            </div>
          )}
        </div>

        {/* Selection summary */}
        <div className="text-[11px] text-text-muted leading-relaxed px-1">
          Set common values for all {trackCount} selected files.
          <br />
          Changes save when you click outside this panel.
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

        <button
          type="button"
          onClick={flushChanges}
          disabled={!dirty || saving}
          className="w-full rounded-lg bg-accent px-3 py-2 text-[12px] font-medium text-white transition-opacity disabled:cursor-not-allowed disabled:opacity-40"
        >
          Apply changes
        </button>
      </div>
    </div>
  );
}

// ── Batch Field sub-component (custom listbox) ──────────────────

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
  const listboxId = useId();
  const [open, setOpen] = useState(false);
  const [activeIndex, setActiveIndex] = useState(-1);
  const inputRef = useRef<HTMLInputElement>(null);
  const listboxRef = useRef<HTMLDivElement>(null);

  const filtered = useMemo(() => {
    if (!value) return suggestions;
    const lower = value.toLowerCase();
    return suggestions.filter((s) => s.toLowerCase().includes(lower));
  }, [suggestions, value]);

  // Close on click outside
  useEffect(() => {
    if (!open) return;
    const handleClick = (e: MouseEvent) => {
      if (
        inputRef.current &&
        !inputRef.current.contains(e.target as Node) &&
        listboxRef.current &&
        !listboxRef.current.contains(e.target as Node)
      ) {
        setOpen(false);
      }
    };
    document.addEventListener("mousedown", handleClick);
    return () => document.removeEventListener("mousedown", handleClick);
  }, [open]);

  const selectSuggestion = useCallback(
    (suggestion: string) => {
      onChange(suggestion);
      setOpen(false);
      setActiveIndex(-1);
    },
    [onChange],
  );

  const handleKeyDown = useCallback(
    (e: React.KeyboardEvent) => {
      if (!open) {
        if (e.key === "ArrowDown" || e.key === "ArrowUp") {
          e.preventDefault();
          setOpen(true);
          setActiveIndex(e.key === "ArrowDown" ? 0 : filtered.length - 1);
        }
        return;
      }

      switch (e.key) {
        case "ArrowDown":
          e.preventDefault();
          setActiveIndex((prev) =>
            prev < filtered.length - 1 ? prev + 1 : 0,
          );
          break;
        case "ArrowUp":
          e.preventDefault();
          setActiveIndex((prev) =>
            prev > 0 ? prev - 1 : filtered.length - 1,
          );
          break;
        case "Enter":
          e.preventDefault();
          if (activeIndex >= 0 && activeIndex < filtered.length) {
            selectSuggestion(filtered[activeIndex]);
          }
          break;
        case "Escape":
          e.preventDefault();
          setOpen(false);
          setActiveIndex(-1);
          break;
      }
    },
    [open, filtered, activeIndex, selectSuggestion],
  );

  const handleChange = useCallback(
    (e: React.ChangeEvent<HTMLInputElement>) => {
      onChange(e.target.value);
      setOpen(true);
      setActiveIndex(-1);
    },
    [onChange],
  );

  const handleFocus = useCallback(() => {
    if (filtered.length > 0) {
      setOpen(true);
    }
  }, [filtered.length]);

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
          ref={inputRef}
          id={id}
          type="text"
          role="combobox"
          aria-expanded={open}
          aria-controls={listboxId}
          aria-autocomplete="list"
          aria-activedescendant={
            activeIndex >= 0 ? `${listboxId}-${activeIndex}` : undefined
          }
          value={value}
          onChange={handleChange}
          onFocus={handleFocus}
          onKeyDown={handleKeyDown}
          placeholder={hasDiffering ? `${placeholder} (mixed values)` : placeholder}
          className="w-full bg-white border border-border rounded-lg px-3 py-1.5 text-[12px] text-text-primary placeholder-text-muted/40 outline-none transition-all focus:border-accent/60 focus:shadow-[0_0_0_3px_rgba(0,122,255,0.2)]"
        />
        {open && filtered.length > 0 && (
          <div
            ref={listboxRef}
            id={listboxId}
            role="listbox"
            className="absolute top-full left-0 right-0 z-[100] max-h-40 overflow-y-auto rounded-lg border border-border shadow-lg bg-[#000]"
          >
            {filtered.map((s, i) => (
              <div
                key={i}
                id={`${listboxId}-${i}`}
                role="option"
                aria-selected={i === activeIndex}
                className={`px-3 py-1.5 text-[12px] cursor-pointer ${
                  i === activeIndex
                    ? "bg-[#2c2c2e] text-white"
                    : "text-white"
                }`}
                onMouseDown={(e) => {
                  e.preventDefault();
                  selectSuggestion(s);
                }}
                onMouseEnter={() => setActiveIndex(i)}
              >
                {s}
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
