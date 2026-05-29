import React, { useState, useEffect, useId, useRef, useCallback } from "react";
import type { TrackData } from "../../electron/preload";

interface MetadataEditorProps {
  track: TrackData;
  dirPath: string;
  coverDataUrl: string | null;
  saving: boolean;
  onChangeCover: () => void;
  onRemoveCover: () => void;
  /** Called to write changed fields to disk. Fires when focus leaves the panel. */
  onSave: (fields: Record<string, string>) => void;
}

export function MetadataEditor({
  track,
  dirPath,
  coverDataUrl,
  saving,
  onChangeCover,
  onRemoveCover,
  onSave,
}: MetadataEditorProps) {
  const filename = track.path.replace(/\\/g, "/").split("/").pop() ?? track.path;

  // Local draft fields — reset whenever the user selects a different track
  const [draft, setDraft] = useState<Record<string, string>>({});
  const [dirty, setDirty] = useState(false);
  const panelRef = useRef<HTMLDivElement>(null);
  const draftRef = useRef<Record<string, string>>({});

  const orig = useCallback(
    (field: string): string => {
      switch (field) {
        case "title":
          return track.title ?? "";
        case "artist":
          return track.artist ?? "";
        case "album":
          return track.album ?? "";
        case "albumArtist":
          return track.albumArtist ?? "";
        case "year":
          return track.year ?? "";
        case "track":
          return formatRange(track.trackNumber, track.trackTotal) ?? "";
        case "disc":
          return formatRange(track.discNumber, track.discTotal) ?? "";
        case "genre":
          return track.genre ?? "";
        case "composer":
          return track.composer ?? "";
        case "comment":
          return track.comment ?? "";
        default:
          return "";
      }
    },
    [track],
  );

  // Flush pending changes to disk
  const flushChanges = useCallback(() => {
    const currentDraft = draftRef.current;
    const changed: Record<string, string> = {};
    for (const field of Object.keys(currentDraft)) {
      if (currentDraft[field] !== orig(field)) {
        changed[field] = currentDraft[field];
      }
    }
    if (Object.keys(changed).length > 0) {
      onSave(changed);
    }
    setDraft({});
    draftRef.current = {};
    setDirty(false);
  }, [onSave, orig]);

  // Reset on track change and flush any pending changes from previous track
  useEffect(() => {
    return () => {
      // Flush if there are pending changes when navigating away
      if (dirty && Object.keys(draftRef.current).length > 0) {
        flushChanges();
      }
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [track.path]);

  useEffect(() => {
    setDraft({});
    draftRef.current = {};
    setDirty(false);
  }, [track.path]);

  // Save when focus leaves the panel
  const handleBlur = useCallback(
    (e: React.FocusEvent) => {
      // Only save if focus moved outside the entire panel
      if (
        panelRef.current &&
        !panelRef.current.contains(e.relatedTarget as Node)
      ) {
        flushChanges();
      }
    },
    [flushChanges],
  );

  const setField = useCallback((field: string, value: string) => {
    setDraft((prev) => ({ ...prev, [field]: value }));
    draftRef.current = { ...draftRef.current, [field]: value };
    setDirty(true);
  }, []);

  const value = (field: string): string =>
    field in draft ? draft[field] : orig(field);

  const isDirty = (field: string): boolean =>
    field in draft && draft[field] !== orig(field);

  const hasChanges = dirty && Object.keys(draft).some((f) => isDirty(f));

  return (
    <div
      ref={panelRef}
      className="flex flex-col h-full overflow-y-auto bg-white border-l border-border"
      onBlur={handleBlur}
    >
      {/* Inspector header */}
      <div className="px-5 py-3.5 bg-surface-alt/40 border-b border-border/60">
        <div className="flex items-center gap-2.5">
          <div className="flex-1 min-w-0">
            <div className="text-[12px] font-medium text-text-primary truncate">
              {filename}
            </div>
            <div className="text-[10px] text-text-muted mt-0.5 truncate">
              Inspector
            </div>
          </div>
          {saving && (
            <span className="flex items-center gap-1.5 text-[10px] text-accent font-medium">
              <span className="w-1.5 h-1.5 rounded-full bg-accent animate-pulse" />
              Saving
            </span>
          )}
          {!saving && hasChanges && (
            <span className="flex items-center gap-1.5 text-[10px] text-[#ff9f0a] font-medium">
              <span className="w-1.5 h-1.5 rounded-full bg-[#ff9f0a]" />
              Unsaved
            </span>
          )}
        </div>
      </div>

      <div className="flex-1 px-5 py-4 space-y-5">
        {/* Album Art */}
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
          <div className="flex gap-2 mt-2.5 justify-center">
            <button
              onClick={onChangeCover}
              className="inline-flex items-center gap-1 px-3 py-1.5 text-[11px] font-medium rounded-lg bg-accent text-white hover:bg-accent/90 transition-all shadow-sm active:scale-[0.97]"
            >
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <path d="M23 19a2 2 0 0 1-2 2H3a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h4l2-3h6l2 3h4a2 2 0 0 1 2 2z" />
                <circle cx="12" cy="13" r="4" />
              </svg>
              Change
            </button>
            {coverDataUrl && (
              <button
                onClick={onRemoveCover}
                className="inline-flex items-center gap-1 px-3 py-1.5 text-[11px] font-medium rounded-lg text-[#ff3b30] hover:bg-red-50 transition-all active:scale-[0.97]"
              >
                <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                  <polyline points="3 6 5 6 21 6" />
                  <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
                </svg>
                Remove
              </button>
            )}
          </div>
        </div>

        {/* Metadata Fields */}
        <div className="space-y-3">
          <InspectorField
            label="Title"
            value={value("title")}
            onChange={(v) => setField("title", v)}
            placeholder="Track title"
            dirty={isDirty("title")}
          />
          <InspectorField
            label="Artist"
            value={value("artist")}
            onChange={(v) => setField("artist", v)}
            placeholder="Artist name"
            dirty={isDirty("artist")}
          />
          <InspectorField
            label="Album"
            value={value("album")}
            onChange={(v) => setField("album", v)}
            placeholder="Album name"
            dirty={isDirty("album")}
          />
          <div className="grid grid-cols-2 gap-2.5">
            <InspectorField
              label="Year"
              value={value("year")}
              onChange={(v) => setField("year", v)}
              placeholder="2024"
              dirty={isDirty("year")}
            />
            <InspectorField
              label="Track"
              value={value("track")}
              onChange={(v) => setField("track", v)}
              placeholder="1/10"
              dirty={isDirty("track")}
            />
          </div>
          <InspectorField
            label="Genre"
            value={value("genre")}
            onChange={(v) => setField("genre", v)}
            placeholder="Genre"
            dirty={isDirty("genre")}
          />
          <InspectorField
            label="Album Artist"
            value={value("albumArtist")}
            onChange={(v) => setField("albumArtist", v)}
            placeholder="Album artist"
            dirty={isDirty("albumArtist")}
          />
          <div className="grid grid-cols-2 gap-2.5">
            <InspectorField
              label="Disc"
              value={value("disc")}
              onChange={(v) => setField("disc", v)}
              placeholder="1/1"
              dirty={isDirty("disc")}
            />
            <InspectorField
              label="Comment"
              value={value("comment")}
              onChange={(v) => setField("comment", v)}
              placeholder="Comment"
              dirty={isDirty("comment")}
            />
          </div>
          <InspectorField
            label="Composer"
            value={value("composer")}
            onChange={(v) => setField("composer", v)}
            placeholder="Composer"
            dirty={isDirty("composer")}
            multiline
          />
        </div>

        {/* Format Details */}
        <div className="pt-1">
          <SectionHeader title="Format Details" />
          <div className="space-y-1.5 mt-2">
            <DetailRow label="Codec" value={track.codec || "—"} />
            <DetailRow
              label="Sample Rate"
              value={
                track.sampleRate ? `${Math.round(track.sampleRate / 1000)} kHz` : "—"
              }
            />
            <DetailRow
              label="Bitrate"
              value={track.bitrate ? `${Math.round(track.bitrate / 1000)} kbps` : "—"}
            />
            <DetailRow label="Size" value={formatSize(track.sizeBytes)} />
            <DetailRow label="Duration" value={formatDuration(track.duration)} />
          </div>
        </div>

        {/* Detailed Tags */}
        {hasDetailedTags(track) && (
          <div className="pt-1">
            <SectionHeader title="Tags" />
            <pre className="mt-2 text-[10px] text-text-muted/70 font-mono whitespace-pre-wrap break-all leading-relaxed">
              {formatDetailedTags(track)}
            </pre>
          </div>
        )}
      </div>
    </div>
  );
}

// ── Sub-components ──────────────────────────────────────────────

function SectionHeader({ title }: { title: string }) {
  return (
    <div className="flex items-center gap-2">
      <span className="text-[10px] font-semibold uppercase tracking-widest text-text-muted">
        {title}
      </span>
      <div className="flex-1 h-px bg-border/50" />
    </div>
  );
}

function InspectorField({
  label,
  value,
  onChange,
  placeholder,
  multiline,
  dirty,
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
  multiline?: boolean;
  dirty?: boolean;
}) {
  const id = useId();

  return (
    <div className="relative">
      <label
        htmlFor={id}
        className="block text-[10px] font-semibold uppercase tracking-widest text-text-muted mb-1.5"
      >
        {label}
      </label>
      <div className="relative">
        {multiline ? (
          <textarea
            id={id}
            value={value}
            onChange={(e) => onChange(e.target.value)}
            rows={2}
            placeholder={placeholder}
            className={`w-full bg-white border rounded-lg px-3 py-1.5 text-[12px] text-text-primary placeholder-text-muted/40 outline-none transition-all resize-none ${
              dirty
                ? "border-[#ff9f0a]/50 focus:border-[#ff9f0a] focus:shadow-[0_0_0_3px_rgba(255,159,10,0.15)]"
                : "border-border focus:border-accent/60 focus:shadow-[0_0_0_3px_rgba(0,122,255,0.2)]"
            }`}
          />
        ) : (
          <input
            id={id}
            type="text"
            value={value}
            onChange={(e) => onChange(e.target.value)}
            placeholder={placeholder}
            className={`w-full bg-white border rounded-lg px-3 py-1.5 text-[12px] text-text-primary placeholder-text-muted/40 outline-none transition-all ${
              dirty
                ? "border-[#ff9f0a]/50 focus:border-[#ff9f0a] focus:shadow-[0_0_0_3px_rgba(255,159,10,0.15)]"
                : "border-border focus:border-accent/60 focus:shadow-[0_0_0_3px_rgba(0,122,255,0.2)]"
            }`}
          />
        )}
        {dirty && (
          <span className="absolute right-2.5 top-1/2 -translate-y-1/2 w-1.5 h-1.5 rounded-full bg-[#ff9f0a]" />
        )}
      </div>
    </div>
  );
}

function DetailRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-center gap-2.5">
      <span className="text-[11px] text-text-muted w-[76px] shrink-0">{label}</span>
      <span className="text-[11px] text-text-secondary">{value}</span>
    </div>
  );
}

// ── Helpers ──────────────────────────────────────────────────────

/** Format seconds as M:SS, or "—" when duration is zero. */
function formatDuration(sec: number): string {
  if (!sec) return "—";
  const m = Math.floor(sec / 60);
  const s = Math.floor(sec % 60)
    .toString()
    .padStart(2, "0");
  return `${m}:${s}`;
}

/** Format number/total as "N/M", just the number when total is absent, or null when both are absent. */
function formatRange(num: number | null, total: number | null): string | null {
  if (num == null) return null;
  return total != null ? `${num}/${total}` : String(num);
}

function formatSize(bytes: number): string {
  if (bytes === 0) return "—";
  const units = ["B", "KB", "MB", "GB"];
  let i = 0;
  let size = bytes;
  while (size >= 1024 && i < units.length - 1) {
    size /= 1024;
    i++;
  }
  return `${size.toFixed(i > 0 ? 1 : 0)} ${units[i]}`;
}

function hasDetailedTags(track: TrackData): boolean {
  return !!(
    track.musicbrainzTrackId ||
    track.musicbrainzAlbumId ||
    track.musicbrainzArtistId ||
    track.compilation != null ||
    track.lyrics ||
    track.discNumber != null
  );
}

function formatDetailedTags(track: TrackData): string {
  const tags: string[] = [];
  if (track.musicbrainzTrackId) tags.push(`MusicBrainz Track ID: ${track.musicbrainzTrackId}`);
  if (track.musicbrainzAlbumId) tags.push(`MusicBrainz Album ID: ${track.musicbrainzAlbumId}`);
  if (track.musicbrainzArtistId) tags.push(`MusicBrainz Artist ID: ${track.musicbrainzArtistId}`);
  if (track.compilation != null) tags.push(`Compilation: ${track.compilation}`);
  if (track.lyrics) {
    tags.push(
      `Lyrics: ${track.lyrics.slice(0, 100)}${track.lyrics.length > 100 ? "…" : ""}`,
    );
  }
  if (track.discNumber != null) {
    tags.push(`Disc: ${track.discNumber}${track.discTotal != null ? `/${track.discTotal}` : ""}`);
  }
  return tags.join("\n");
}
