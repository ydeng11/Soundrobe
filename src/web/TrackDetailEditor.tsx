/**
 * Track detail editor for the browser workspace.
 *
 * Section order, section names and field set follow the Music Tag Web V2
 * detail editor: Basic Information, Media Information, Lyric Information,
 * Category Information, Metadata Information, Technical Information.
 *
 * Every control writes through the shared Rust pipeline; nothing here writes
 * to disk directly. Edits are staged in a draft and applied through
 * `onApply`, so Cancel always leaves files untouched.
 */

import React from "react";
import type { TrackData } from "../shared/desktop-api";
import { parseDiscField } from "../shared/fields";

export interface TrackDetailEditorProps {
  track: TrackData | null;
  coverDataUrl?: string | null;
  /** True when a lyrics sidecar exists next to the track. */
  hasLrc?: boolean;
  /** Applies the staged patch through the native writer. */
  onApply: (fields: Record<string, unknown>) => void | Promise<void>;
  onSetCover?: () => void;
  onRemoveCover?: () => void;
  saving?: boolean;
}

interface Draft {
  title: string;
  artistsText: string;
  album: string;
  albumArtist: string;
  year: string;
  genre: string;
  composer: string;
  comment: string;
  description: string;
  lyrics: string;
  discNumber: string;
  discTotal: string;
  trackNumber: string;
  trackTotal: string;
}

const EMPTY_DRAFT: Draft = {
  title: "",
  artistsText: "",
  album: "",
  albumArtist: "",
  year: "",
  genre: "",
  composer: "",
  comment: "",
  description: "",
  lyrics: "",
  discNumber: "",
  discTotal: "",
  trackNumber: "",
  trackTotal: "",
};

function draftFromTrack(track: TrackData | null): Draft {
  if (!track) return EMPTY_DRAFT;
  const artists =
    track.artists.length > 0 ? track.artists.join(", ") : track.artist ?? "";
  return {
    title: track.title ?? "",
    artistsText: artists,
    album: track.album ?? "",
    albumArtist:
      track.albumArtist ??
      (track.albumArtists.length > 0 ? track.albumArtists.join(", ") : ""),
    year: track.year ?? "",
    genre: track.genre ?? "",
    composer: track.composer ?? "",
    comment: track.comment ?? "",
    description: track.description ?? "",
    lyrics: track.lyrics?.plainLyrics ?? "",
    discNumber: track.discNumber != null ? String(track.discNumber) : "",
    discTotal: track.discTotal != null ? String(track.discTotal) : "",
    trackNumber: track.trackNumber != null ? String(track.trackNumber) : "",
    trackTotal: track.trackTotal != null ? String(track.trackTotal) : "",
  };
}

/** Split the comma-separated artist text into individual credits. */
export function parseArtistList(value: string): string[] {
  return value
    .split(/[,;]/)
    .map((part) => part.trim())
    .filter(Boolean);
}

/** Build the `TrackPatch` payload for a staged draft. */
export function buildTrackPatch(draft: Draft): Record<string, unknown> {
  const disc = parseDiscField(
    draft.discNumber && draft.discTotal
      ? `${draft.discNumber}/${draft.discTotal}`
      : draft.discNumber,
  );
  const patch: Record<string, unknown> = {
    title: draft.title || null,
    artists: parseArtistList(draft.artistsText),
    album: draft.album || null,
    albumArtist:
      draft.albumArtist || null,
    year: draft.year || null,
    genre: draft.genre || null,
    composer: draft.composer || null,
    comment: draft.comment || null,
    description: draft.description || null,
    lyrics: draft.lyrics
      ? { plainLyrics: draft.lyrics, syncedLyrics: null }
      : null,
    trackNumber: draft.trackNumber ? Number(draft.trackNumber) : null,
    trackTotal: draft.trackTotal ? Number(draft.trackTotal) : null,
    ...disc,
  };
  if (!draft.discNumber) {
    patch.discNumber = null;
    patch.discTotal = draft.discTotal ? Number(draft.discTotal) : null;
  }
  return patch;
}

export function TrackDetailEditor({
  track,
  coverDataUrl,
  hasLrc = false,
  onApply,
  onSetCover,
  onRemoveCover,
  saving = false,
}: TrackDetailEditorProps) {
  const [draft, setDraft] = React.useState<Draft>(() => draftFromTrack(track));

  React.useEffect(() => {
    setDraft(draftFromTrack(track));
  }, [track]);

  if (!track) {
    return (
      <div className="sr-empty" data-testid="editor-empty">
        <h3 className="sr-empty__title">Select a track to start organizing</h3>
        <p className="sr-empty__body">
          Choose a file in the table to edit its tags, artwork and lyrics.
        </p>
      </div>
    );
  }

  const set = <K extends keyof Draft>(key: K, value: Draft[K]) =>
    setDraft((current) => ({ ...current, [key]: value }));

  const dirty = JSON.stringify(draft) !== JSON.stringify(draftFromTrack(track));

  return (
    <form
      className="sr-editor"
      data-testid="track-editor"
      onSubmit={(event) => {
        event.preventDefault();
        void onApply(buildTrackPatch(draft));
      }}
    >
      <h2 className="sr-editor__heading">File details</h2>

      <section className="sr-section">
        <h3 className="sr-section__title">Basic Information</h3>

        <Field label="Title">
          <input
            className="sr-input"
            aria-label="Title"
            value={draft.title}
            onChange={(event) => set("title", event.target.value)}
          />
        </Field>

        <Field label="Artists">
          <input
            className="sr-input"
            aria-label="Artists"
            placeholder="Comma separated"
            value={draft.artistsText}
            onChange={(event) => set("artistsText", event.target.value)}
          />
        </Field>

        <Field label="Albums">
          <input
            className="sr-input"
            aria-label="Albums"
            value={draft.album}
            onChange={(event) => set("album", event.target.value)}
          />
        </Field>

        <Field label="Album Artist">
          <input
            className="sr-input"
            aria-label="Album Artist"
            value={draft.albumArtist}
            onChange={(event) => set("albumArtist", event.target.value)}
          />
        </Field>

        <Field label="Year">
          <input
            className="sr-input"
            aria-label="Year"
            value={draft.year}
            onChange={(event) => set("year", event.target.value)}
          />
        </Field>
      </section>

      <section className="sr-section">
        <h3 className="sr-section__title">Media Information</h3>
        <div className="sr-field">
          <span className="sr-field__label">Cover</span>
          <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
            {coverDataUrl ? (
              <img className="sr-cover" src={coverDataUrl} alt="" />
            ) : (
              <span aria-hidden="true" className="sr-cover" />
            )}
            {onSetCover && (
              <button type="button" className="sr-btn" onClick={onSetCover}>
                Set cover
              </button>
            )}
            {onRemoveCover && (
              <button
                type="button"
                className="sr-btn"
                disabled={!track.hasCover}
                onClick={onRemoveCover}
              >
                Remove cover
              </button>
            )}
          </div>
        </div>
      </section>

      <section className="sr-section">
        <h3 className="sr-section__title">Lyric Information</h3>
        <div className="sr-field sr-field--stacked">
          <label className="sr-field__label" htmlFor="sr-lyrics">
            Lyrics
          </label>
          <textarea
            id="sr-lyrics"
            className="sr-textarea"
            placeholder="One line per lyric. LRC timestamps are preserved."
            value={draft.lyrics}
            onChange={(event) => set("lyrics", event.target.value)}
          />
        </div>
        <Field label="LRC sidecar">
          <span className="sr-field__static" data-testid="editor-lrc-status">
            {hasLrc ? "Present" : "None"}
          </span>
        </Field>
      </section>

      <section className="sr-section">
        <h3 className="sr-section__title">Category Information</h3>

        <Field label="Genres">
          <input
            className="sr-input"
            aria-label="Genres"
            value={draft.genre}
            onChange={(event) => set("genre", event.target.value)}
          />
        </Field>

        <Field label="Composer">
          <input
            className="sr-input"
            aria-label="Composer"
            value={draft.composer}
            onChange={(event) => set("composer", event.target.value)}
          />
        </Field>
      </section>

      <section className="sr-section">
        <h3 className="sr-section__title">Metadata Information</h3>

        <div className="sr-field sr-field--stacked">
          <label className="sr-field__label" htmlFor="sr-comment">
            Summary
          </label>
          <textarea
            id="sr-comment"
            className="sr-textarea"
            value={draft.comment}
            onChange={(event) => set("comment", event.target.value)}
          />
        </div>

        <div className="sr-field sr-field--stacked">
          <label className="sr-field__label" htmlFor="sr-description">
            Description
          </label>
          <textarea
            id="sr-description"
            className="sr-textarea"
            value={draft.description}
            onChange={(event) => set("description", event.target.value)}
          />
        </div>
      </section>

      <section className="sr-section">
        <h3 className="sr-section__title">Technical Information</h3>

        <Field label="Disc Number">
          <input
            className="sr-input"
            aria-label="Disc Number"
            value={draft.discNumber}
            onChange={(event) => set("discNumber", event.target.value)}
          />
        </Field>

        <Field label="Track No.">
          <input
            className="sr-input"
            aria-label="Track No."
            value={draft.trackNumber}
            onChange={(event) => set("trackNumber", event.target.value)}
          />
        </Field>

        <Field label="Total Tracks">
          <input
            className="sr-input"
            aria-label="Total Tracks"
            value={draft.trackTotal}
            onChange={(event) => set("trackTotal", event.target.value)}
          />
        </Field>

        <Field label="Total Discs">
          <input
            className="sr-input"
            aria-label="Total Discs"
            value={draft.discTotal}
            onChange={(event) => set("discTotal", event.target.value)}
          />
        </Field>

        <div className="sr-technical" data-testid="editor-technical">
          <TechnicalRow label="Duration" value={formatDuration(track.duration)} />
          <TechnicalRow label="Bitrate" value={formatBitrate(track.bitrate)} />
          <TechnicalRow label="Sample Rate" value={formatSampleRate(track.sampleRate)} />
          <TechnicalRow label="Format" value={track.codec.toUpperCase()} />
          <TechnicalRow label="File Size" value={formatBytes(track.sizeBytes)} />
        </div>
      </section>

      <div style={{ display: "flex", gap: 8 }}>
        <button type="submit" className="sr-btn sr-btn--primary" disabled={!dirty || saving}>
          {saving ? "Applying…" : "Apply"}
        </button>
        <button
          type="button"
          className="sr-btn"
          disabled={!dirty || saving}
          onClick={() => setDraft(draftFromTrack(track))}
        >
          Cancel
        </button>
      </div>
    </form>
  );
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="sr-field">
      <span className="sr-field__label">{label}</span>
      {children}
    </div>
  );
}

function TechnicalRow({ label, value }: { label: string; value: string }) {
  return (
    <span className="sr-technical__row">
      <span className="sr-technical__label">{label}</span>
      <span>{value}</span>
    </span>
  );
}

export function formatDuration(seconds: number | null | undefined): string {
  if (seconds == null || !Number.isFinite(seconds)) return "—";
  const total = Math.round(seconds);
  const mins = Math.floor(total / 60);
  const secs = total % 60;
  return `${mins}:${String(secs).padStart(2, "0")}`;
}

export function formatSampleRate(hertz: number | null | undefined): string {
  if (hertz == null || !Number.isFinite(hertz) || hertz <= 0) return "—";
  return `${(hertz / 1000).toFixed(1)} kHz`;
}

export function formatBitrate(bitsPerSecond: number | null | undefined): string {
  if (bitsPerSecond == null || !Number.isFinite(bitsPerSecond) || bitsPerSecond <= 0) {
    return "—";
  }
  return `${Math.round(bitsPerSecond / 1000)} kbps`;
}

export function formatBytes(bytes: number | null | undefined): string {
  if (bytes == null || !Number.isFinite(bytes) || bytes <= 0) return "—";
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(0)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
}
