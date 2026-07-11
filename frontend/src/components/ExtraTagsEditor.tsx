import React, { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { ExtraTag, TrackData } from "../../electron/preload";
import { basename } from "../utils/path";

// Tag keys that are shared across Vorbis and ID3v2 formats
const COMMON_EXTRA_TAGS = [
  "COMMENT",
  "DESCRIPTION",
  "LYRICIST",
  "ARRANGER",
  "CONDUCTOR",
  "DISCSUBTITLE",
  "DISCTOTAL",
  "GROUPING",
  "ISRC",
  "LABEL",
  "LICENSE",
  "OPUS",
  "REPLAYGAIN_TRACK_GAIN",
  "REPLAYGAIN_TRACK_PEAK",
  "REPLAYGAIN_ALBUM_GAIN",
  "REPLAYGAIN_ALBUM_PEAK",
  "SCRIPT",
  "SUBTITLE",
  "TOTALDISCS",
  "TOTALTRACKS",
  "ARTISTS",
  "ALBUMARTISTS",
  "COMPILATION",
  "MUSICBRAINZ_ALBUMID",
  "MUSICBRAINZ_ARTISTID",
  "MUSICBRAINZ_TRACKID",
  "DISCOGS_ARTIST_ID",
  "DISCOGS_RELEASE_ID",
];

// Vorbis-style uppercase tags (FLAC, OGG, OPUS, APE)
const VORBIS_SPECIFIC_TAGS = [
  // MusicBrainz
  "MUSICBRAINZ_DISCID",
  "MUSICBRAINZ_ORIGINALALBUMID",
  "MUSICBRAINZ_RELEASEGROUPID",
  "MUSICBRAINZ_RELEASEID",
  "MUSICBRAINZ_WORKID",
  // Discogs
  "DISCOGS_ALBUM_ARTISTS",
  "DISCOGS_CATALOG",
  "DISCOGS_COUNTRY",
  "DISCOGS_LABEL",
  "DISCOGS_RELEASED",
  "DISCOGS_STYLE",
  "DISCOGS_VOTES",
];

// ID3v2 native frame IDs
const ID3V2_FRAME_TAGS = [
  "TCOM",
  "TIT3",
  "TSRC",
  "TPUB",
  "TCOP",
  "TOLY",
  "TPE3",
  "TPE4",
  "TSST",
  "TSOA",
  "TSOP",
  "TSOT",
];

const ID3V2_SUGGESTED_KEYS = [...COMMON_EXTRA_TAGS, ...ID3V2_FRAME_TAGS];
const VORBIS_SUGGESTED_KEYS = [...COMMON_EXTRA_TAGS, ...VORBIS_SPECIFIC_TAGS];

/** Return file-type-specific suggested tag keys. */
function getSuggestedTagKeys(filePath: string): string[] {
  const ext = filePath.split(".").pop()?.toLowerCase() ?? "";
  return ext === "mp3" || ext === "wav" ? ID3V2_SUGGESTED_KEYS : VORBIS_SUGGESTED_KEYS;
}

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
  const [activeKeyId, setActiveKeyId] = useState<string | null>(null);
  const [keyFilter, setKeyFilter] = useState("");
  const [dropdownPos, setDropdownPos] = useState<{ top: number; left: number; width: number } | null>(null);
  const newKeyRef = useRef<HTMLInputElement | null>(null);
  const keyInputRefs = useRef<Map<string, HTMLInputElement>>(new Map());
  const suggestionsRef = useRef<HTMLDivElement | null>(null);
  const scrollRef = useRef<HTMLDivElement | null>(null);

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

  // Suggestions for the tag key autocomplete
  const keySuggestions = useMemo(() => {
    if (!activeKeyId) return [];
    const row = rows.find((r) => r.id === activeKeyId);
    if (!row) return [];
    const usedKeys = new Set(rows.map((r) => r.key.toUpperCase()));
    const q = keyFilter.trim().toUpperCase();
    const suggestedKeys = getSuggestedTagKeys(track.path);
    return suggestedKeys.filter(
      (k) => !usedKeys.has(k.toUpperCase()) && (!q || k.toUpperCase().includes(q)),
    ).slice(0, 8);
  }, [activeKeyId, rows, keyFilter, track.path]);

  // Close suggestions on outside click
  useEffect(() => {
    if (!activeKeyId) return;
    const handler = (e: MouseEvent) => {
      const target = e.target as Node;
      const inputEl = keyInputRefs.current.get(activeKeyId);
      const dropEl = suggestionsRef.current;
      if (inputEl?.contains(target) || dropEl?.contains(target)) return;
      setActiveKeyId(null);
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [activeKeyId]);

  // Update dropdown position on scroll
  useEffect(() => {
    if (!activeKeyId) return;
    const updatePos = () => {
      const el = keyInputRefs.current.get(activeKeyId);
      if (!el) return;
      const rect = el.getBoundingClientRect();
      setDropdownPos({ top: rect.bottom + 2, left: rect.left, width: rect.width });
    };
    updatePos();
    const scrollEl = scrollRef.current;
    scrollEl?.addEventListener("scroll", updatePos, { passive: true });
    return () => scrollEl?.removeEventListener("scroll", updatePos);
  }, [activeKeyId]);

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

        <div ref={scrollRef} className="flex-1 overflow-y-auto">
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
                <div className="relative">
                  <input
                    ref={(el) => {
                      if (row.isNew) newKeyRef.current = el;
                      if (el) keyInputRefs.current.set(row.id, el);
                    }}
                    value={row.key}
                    onChange={(event) => {
                      updateRow(row.id, { key: event.target.value });
                      setKeyFilter(event.target.value);
                    }}
                    onFocus={() => {
                      setActiveKeyId(row.id);
                      setKeyFilter(row.key);
                    }}
                    onKeyDown={(event) => {
                      if (event.key === "Escape" && activeKeyId === row.id) {
                        setActiveKeyId(null);
                      }
                    }}
                    className={`h-8 w-full bg-transparent border border-transparent rounded-md px-2 text-[12px] font-medium outline-none focus:border-accent/60 focus:bg-white focus:shadow-[0_0_0_3px_rgba(0,122,255,0.14)] ${
                      row.deleted ? "line-through" : ""
                    }`}
                    placeholder="Tag key"
                  />
                </div>
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

      {/* Autocomplete dropdown — rendered outside overflow containers */}
      {activeKeyId && keySuggestions.length > 0 && dropdownPos && (
        <div
          ref={suggestionsRef}
          style={{ position: "fixed", top: dropdownPos.top, left: dropdownPos.left, width: dropdownPos.width }}
          className="z-[90] max-h-[160px] overflow-y-auto bg-white border border-border rounded-lg shadow-lg"
        >
          {keySuggestions.map((key) => (
            <button
              key={key}
              type="button"
              onMouseDown={(e) => {
                e.preventDefault();
                updateRow(activeKeyId, { key });
                setActiveKeyId(null);
              }}
              className="w-full text-left px-3 py-1.5 text-[11px] hover:bg-accent/10 text-text-primary truncate"
            >
              {key}
            </button>
          ))}
        </div>
      )}
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
