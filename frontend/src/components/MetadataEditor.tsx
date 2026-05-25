import React from "react";
import type { TrackData } from "../../electron/preload";

interface MetadataEditorProps {
  track: TrackData;
  dirPath: string;
  coverDataUrl: string | null;
  saving: boolean;
  onFieldChange: (field: string, value: string) => void;
  onChangeCover: () => void;
  onRemoveCover: () => void;
}

export function MetadataEditor({
  track,
  dirPath,
  coverDataUrl,
  saving,
  onFieldChange,
  onChangeCover,
  onRemoveCover,
}: MetadataEditorProps) {
  const filename = filenameFromPath(track.path);

  return (
    <div className="flex flex-col h-full overflow-y-auto">
      {/* File header */}
      <div className="px-4 py-3 bg-surface-alt/60 border-b border-gray-700/30">
        <div className="text-xs font-medium text-text-primary truncate flex items-center gap-2">
          <span className="truncate">{filename}</span>
          {saving && (
            <span className="text-accent-light text-[10px] animate-pulse">
              ● saving
            </span>
          )}
        </div>
      </div>

      <div className="flex-1 px-4 py-3 space-y-4">
        {/* Album art */}
        <div>
          <div className="w-full aspect-square max-w-[220px] mx-auto rounded-lg overflow-hidden bg-surface-card border border-gray-700/30">
            {coverDataUrl ? (
              <img
                src={coverDataUrl}
                alt="Cover art"
                className="w-full h-full object-cover"
              />
            ) : (
              <div className="w-full h-full flex items-center justify-center text-text-muted text-[11px]">
                <span className="text-center">
                  ♪
                  <br />
                  No cover
                </span>
              </div>
            )}
          </div>
          {/* Cover buttons */}
          <div className="flex gap-2 mt-2 justify-center">
            <button
              onClick={onChangeCover}
              className="px-2.5 py-1 text-[10px] font-medium rounded bg-accent/20 text-accent-light hover:bg-accent/30 transition-colors"
            >
              🖼 Change
            </button>
            <button
              onClick={onRemoveCover}
              className="px-2.5 py-1 text-[10px] font-medium rounded bg-red-500/10 text-red-400 hover:bg-red-500/20 transition-colors"
            >
              ✕ Remove
            </button>
          </div>
        </div>

        {/* Metadata fields */}
        <div className="space-y-2">
          <FieldRow
            label="Title"
            value={track.title ?? ""}
            onChange={(v) => onFieldChange("title", v)}
          />
          <FieldRow
            label="Artist"
            value={track.artist ?? ""}
            onChange={(v) => onFieldChange("artist", v)}
          />
          <FieldRow
            label="Album"
            value={track.album ?? ""}
            onChange={(v) => onFieldChange("album", v)}
          />
          <div className="flex gap-2">
            <FieldRow
              label="Year"
              value={track.year ?? ""}
              onChange={(v) => onFieldChange("year", v)}
              className="flex-1"
            />
            <FieldRow
              label="Track"
              value={
                track.trackNumber != null
                  ? track.trackTotal != null
                    ? `${track.trackNumber}/${track.trackTotal}`
                    : String(track.trackNumber)
                  : ""
              }
              onChange={(v) => onFieldChange("track", v)}
              className="w-20"
            />
          </div>
          <FieldRow
            label="Genre"
            value={track.genre ?? ""}
            onChange={(v) => onFieldChange("genre", v)}
          />
          <FieldRow
            label="Composer"
            value={track.composer ?? ""}
            onChange={(v) => onFieldChange("composer", v)}
            multiline
          />
          <FieldRow
            label="Comment"
            value=""
            onChange={(v) => onFieldChange("comment", v)}
            multiline
          />
        </div>

        {/* File format details */}
        <div className="pt-3 border-t border-gray-700/30">
          <div className="text-[10px] font-semibold uppercase tracking-wider text-text-muted mb-2">
            Format Details
          </div>
          <div className="space-y-1 text-[10px] text-text-muted">
            <DetailRow label="Codec" value={track.codec || "—"} />
            <DetailRow
              label="Sample Rate"
              value={
                track.sampleRate
                  ? `${Math.round(track.sampleRate / 1000)} kHz`
                  : "—"
              }
            />
            <DetailRow
              label="Bitrate"
              value={
                track.bitrate
                  ? `${Math.round(track.bitrate / 1000)} kbps`
                  : "—"
              }
            />
            <DetailRow label="Size" value={formatSize(track.sizeBytes)} />
          </div>
        </div>

        {/* Detailed tags (read-only) */}
        <div className="pt-3 border-t border-gray-700/30">
          <div className="text-[10px] font-semibold uppercase tracking-wider text-text-muted mb-2">
            Detailed Tags
          </div>
          <pre className="text-[10px] text-text-muted/70 font-mono whitespace-pre-wrap break-all leading-relaxed">
            {formatDetailedTags(track)}
          </pre>
        </div>
      </div>
    </div>
  );
}

function FieldRow({
  label,
  value,
  onChange,
  multiline,
  className,
}: {
  label: string;
  value: string;
  onChange: (v: string) => void;
  multiline?: boolean;
  className?: string;
}) {
  return (
    <div className={className ?? ""}>
      <label className="block text-[10px] font-semibold uppercase tracking-wider text-text-muted mb-1">
        {label}
      </label>
      {multiline ? (
        <textarea
          value={value}
          onChange={(e) => onChange(e.target.value)}
          rows={2}
          className="w-full bg-surface-card/60 border border-gray-700/40 rounded px-2.5 py-1.5 text-xs text-text-primary outline-none focus:border-accent/50 focus:bg-surface-card transition-colors resize-none"
        />
      ) : (
        <input
          type="text"
          value={value}
          onChange={(e) => onChange(e.target.value)}
          className="w-full bg-surface-card/60 border border-gray-700/40 rounded px-2.5 py-1.5 text-xs text-text-primary outline-none focus:border-accent/50 focus:bg-surface-card transition-colors"
        />
      )}
    </div>
  );
}

function DetailRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-center gap-2">
      <span className="w-20 text-text-muted/60">{label}</span>
      <span className="text-text-secondary">{value}</span>
    </div>
  );
}

function filenameFromPath(p: string): string {
  const sep = p.replace(/\\/g, "/");
  return sep.split("/").pop() ?? p;
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

function formatDetailedTags(track: TrackData): string {
  const tags: string[] = [];
  if (track.musicbrainzTrackId)
    tags.push(`MusicBrainz Track ID: ${track.musicbrainzTrackId}`);
  if (track.musicbrainzAlbumId)
    tags.push(`MusicBrainz Album ID: ${track.musicbrainzAlbumId}`);
  if (track.musicbrainzArtistId)
    tags.push(`MusicBrainz Artist ID: ${track.musicbrainzArtistId}`);
  if (track.compilation != null)
    tags.push(`Compilation: ${track.compilation}`);
  if (track.lyrics)
    tags.push(
      `Lyrics: ${track.lyrics.slice(0, 100)}${track.lyrics.length > 100 ? "…" : ""}`
    );
  if (track.discNumber != null) {
    tags.push(
      `Disc: ${track.discNumber}${track.discTotal != null ? `/${track.discTotal}` : ""}`
    );
  }
  tags.push(`Path: ${track.path}`);
  return tags.join("\n");
}
