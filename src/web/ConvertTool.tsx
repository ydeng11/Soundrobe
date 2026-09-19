/**
 * Convert: rewrite tags from file names, one tag from another, or file names
 * from tags.
 *
 * This mirrors the desktop `ConvertDialog` and its apply path in `App.tsx`,
 * including the parts that make it safe to point at a library: the pattern is
 * previewed and validated before the button enables, a result identical to the
 * current value is reported as no change rather than written, a rename refuses
 * to clobber an existing target, and per-file failures are reported instead of
 * letting a partial run look like a clean one.
 *
 * `shared/convert` owns the pattern language, so both apps agree on it.
 */

import React from "react";
import type { TrackData } from "../shared/desktop-api";
import {
  CONVERT_FIELD_LABELS,
  CONVERT_SOURCE_TAGS,
  DEFAULT_CONVERT_PRESETS,
  buildFilenameFromConvertPattern,
  getConvertSourceValue,
  parseFilenameWithConvertPattern,
  parseTextWithConvertPattern,
  type ConvertDirection,
  type ConvertFieldMap,
  type ConvertSourceTag,
  type ConvertTrackData,
} from "../shared/convert";
import { fileNameOf } from "./columns";
import { ToolDrawer, summarizeFailures, useToolRun } from "./ToolDrawer";

export interface ConvertToolProps {
  /** The tracks this run covers: the selection, or the album when none. */
  tracks: TrackData[];
  /** How many of those came from an explicit selection. */
  selectedCount: number;
  api: Window["api"];
  onApplied: () => Promise<void> | void;
  onClose: () => void;
}

const DIRECTIONS: Array<{ key: ConvertDirection; label: string }> = [
  { key: "filename-to-tags", label: "Filename → Tags" },
  { key: "tag-to-tags", label: "Tag → Tags" },
  { key: "tags-to-filename", label: "Tags → Filename" },
];

function toConvertTrack(track: TrackData): ConvertTrackData {
  return {
    filename: fileNameOf(track.path),
    title: track.title,
    artist: track.artist,
    album: track.album,
    year: track.year,
    track: track.trackNumber,
    genre: track.genre,
    albumArtist: track.albumArtist,
    composer: track.composer,
    comment: track.comment,
    discNumber: track.discNumber,
  };
}

/** The tag values as they stand, so a preview can say "no change". */
function currentValues(track: ConvertTrackData): Record<string, string> {
  return {
    title: track.title ?? "",
    artist: track.artist ?? "",
    album: track.album ?? "",
    year: track.year ?? "",
    genre: track.genre ?? "",
    albumArtist: track.albumArtist ?? "",
    composer: track.composer ?? "",
    comment: track.comment ?? "",
    track: track.track == null ? "" : String(track.track),
    disc: track.discNumber == null ? "" : String(track.discNumber),
  };
}

interface Preview {
  input: string | null;
  output: string | null;
  fields: ConvertFieldMap;
  error: string | null;
  isNoChange: boolean;
}

/** The desktop's preview rules, so the button state means the same thing. */
function buildPreview(
  track: ConvertTrackData | null,
  direction: ConvertDirection,
  pattern: string,
  sourceTag: ConvertSourceTag,
): Preview {
  const empty: Preview = { input: null, output: null, fields: {}, error: null, isNoChange: false };
  if (!track) return empty;

  const trimmed = pattern.trim();
  if (!trimmed) {
    return {
      ...empty,
      input: direction === "tag-to-tags" ? getConvertSourceValue(track, sourceTag) : track.filename,
      error:
        direction === "tags-to-filename"
          ? "Enter a template like %{track}% - %{title}%%{ext}%"
          : "Enter a pattern like %{track}% - %{title}%",
    };
  }

  if (direction === "tags-to-filename") {
    const newFilename = buildFilenameFromConvertPattern(trimmed, track);
    return {
      ...empty,
      input: track.filename,
      output: newFilename,
      isNoChange: newFilename === track.filename,
    };
  }

  const sourceValue =
    direction === "filename-to-tags"
      ? track.filename
      : getConvertSourceValue(track, sourceTag);
  if (direction === "tag-to-tags" && !sourceValue.trim()) {
    return {
      ...empty,
      input: sourceValue,
      error: `${CONVERT_FIELD_LABELS[sourceTag] ?? sourceTag} is empty`,
    };
  }

  const parsed =
    direction === "filename-to-tags"
      ? parseFilenameWithConvertPattern(trimmed, track.filename)
      : parseTextWithConvertPattern(trimmed, sourceValue);
  if ("error" in parsed) return { ...empty, input: sourceValue, error: parsed.error };

  const current = currentValues(track);
  const keys = Object.keys(parsed.fields);
  const isNoChange =
    keys.length > 0 && keys.every((key) => current[key] === parsed.fields[key as keyof ConvertFieldMap]);

  return {
    ...empty,
    input: sourceValue,
    output: parsed.displayResult,
    fields: parsed.fields,
    isNoChange,
  };
}

/** Tags the writer stores as numbers, so a text capture still lands correctly. */
function toWriteFields(fields: ConvertFieldMap): Record<string, unknown> {
  const patch: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(fields)) {
    if (value === undefined) continue;
    if (key === "track") {
      const track = Number.parseInt(value, 10);
      if (Number.isFinite(track)) patch.trackNumber = track;
    } else if (key === "disc") {
      const disc = Number.parseInt(value, 10);
      if (Number.isFinite(disc)) patch.discNumber = disc;
    } else {
      patch[key] = value;
    }
  }
  return patch;
}

export function ConvertTool({
  tracks,
  selectedCount,
  api,
  onApplied,
  onClose,
}: ConvertToolProps) {
  const [direction, setDirection] = React.useState<ConvertDirection>(
    DEFAULT_CONVERT_PRESETS[0].direction,
  );
  const [presetLabel, setPresetLabel] = React.useState(DEFAULT_CONVERT_PRESETS[0].label);
  const [pattern, setPattern] = React.useState(DEFAULT_CONVERT_PRESETS[0].pattern);
  const [sourceTag, setSourceTag] = React.useState<ConvertSourceTag>(
    DEFAULT_CONVERT_PRESETS[0].sourceTag ?? "title",
  );
  const { busy, error, failures, message, run } = useToolRun();

  const presetsForDirection = React.useMemo(
    () => DEFAULT_CONVERT_PRESETS.filter((preset) => preset.direction === direction),
    [direction],
  );

  // Switching direction must not leave a pattern from the other direction.
  React.useEffect(() => {
    const selected = DEFAULT_CONVERT_PRESETS.find(
      (preset) => preset.label === presetLabel && preset.direction === direction,
    );
    if (selected) return;
    const first = DEFAULT_CONVERT_PRESETS.find((preset) => preset.direction === direction);
    if (!first) return;
    setPresetLabel(first.label);
    setPattern(first.pattern);
    setSourceTag(first.sourceTag ?? "title");
  }, [direction, presetLabel]);

  // The desktop previews the first track; the count in the button says the rest.
  const preview = React.useMemo(
    () => buildPreview(tracks[0] ? toConvertTrack(tracks[0]) : null, direction, pattern, sourceTag),
    [direction, pattern, sourceTag, tracks],
  );

  const canConvert =
    tracks.length > 0 && preview.output !== null && !preview.error && !preview.isNoChange;
  const convertLabel =
    tracks.length > 1 ? `Convert (${tracks.length} tracks)` : "Convert";

  const selectPreset = (label: string) => {
    const preset = DEFAULT_CONVERT_PRESETS.find((candidate) => candidate.label === label);
    if (!preset) return;
    setPresetLabel(preset.label);
    setPattern(preset.pattern);
    setSourceTag(preset.sourceTag ?? "title");
  };

  const apply = () =>
    run(async () => {
      if (tracks.length === 0) throw new Error("no tracks found to convert");
      const trimmed = pattern.trim();
      const problems: string[] = [];
      let converted = 0;

      if (direction === "tags-to-filename") {
        for (const track of tracks) {
          const filename = fileNameOf(track.path);
          const newFilename = buildFilenameFromConvertPattern(trimmed, toConvertTrack(track));
          if (!newFilename || newFilename === filename) {
            problems.push(`${filename}: no change needed`);
            continue;
          }
          const separator = track.path.lastIndexOf("/");
          const nextPath = `${track.path.slice(0, separator + 1)}${newFilename}`;
          // A rename that overwrites another track is unrecoverable by undo.
          const exists = await api.checkFileExists(nextPath).catch(() => false);
          if (exists) {
            problems.push(`${filename}: target already exists (${newFilename})`);
            continue;
          }
          try {
            await api.renameTrack(track.path, nextPath);
            converted += 1;
          } catch (reason) {
            problems.push(
              `${filename}: ${reason instanceof Error ? reason.message : "rename failed"}`,
            );
          }
        }
        await onApplied();
        return { message: `Renamed ${converted} of ${tracks.length} files.`, failures: problems };
      }

      const updates: Array<{ path: string; fields: Record<string, unknown> }> = [];
      for (const track of tracks) {
        const filename = fileNameOf(track.path);
        const data = toConvertTrack(track);
        const sourceValue =
          direction === "filename-to-tags" ? filename : getConvertSourceValue(data, sourceTag);
        if (direction === "tag-to-tags" && !sourceValue.trim()) {
          problems.push(`${filename}: ${sourceTag} tag is empty`);
          continue;
        }
        const parsed =
          direction === "filename-to-tags"
            ? parseFilenameWithConvertPattern(trimmed, filename)
            : parseTextWithConvertPattern(trimmed, sourceValue);
        if ("error" in parsed) {
          problems.push(`${filename}: ${parsed.error}`);
          continue;
        }
        const fields = toWriteFields(parsed.fields);
        if (Object.keys(fields).length === 0) {
          problems.push(`${filename}: no fields extracted`);
          continue;
        }
        updates.push({ path: track.path, fields });
      }

      if (updates.length === 0) {
        return {
          message: `Wrote nothing; ${problems.length} file(s) could not be converted.`,
          failures: summarizeFailures("", problems),
        };
      }

      const result = await api.writeTracks(updates);
      await onApplied();
      const rejected = (result.failures ?? []).map(
        (failure) => `${failure.path}: ${failure.error}`,
      );
      return {
        message: `Wrote fields onto ${updates.length - rejected.length} of ${tracks.length} tracks.`,
        failures: summarizeFailures("", [...problems, ...rejected]),
      };
    });

  const track = tracks[0] ? toConvertTrack(tracks[0]) : null;

  return (
    <ToolDrawer
      title="Convert names and tags"
      busy={busy}
      error={error}
      errors={failures}
      onClose={onClose}
      testId="convert-drawer"
    >
      <section className="sr-section">
        <h3 className="sr-section__title">Direction</h3>
        <div className="sr-chips" data-testid="convert-directions">
          {DIRECTIONS.map((candidate) => (
            <button
              key={candidate.key}
              type="button"
              className={`sr-btn ${direction === candidate.key ? "sr-btn--primary" : ""}`}
              data-testid={`convert-direction-${candidate.key}`}
              aria-pressed={direction === candidate.key}
              onClick={() => setDirection(candidate.key)}
            >
              {candidate.label}
            </button>
          ))}
        </div>

        <h3 className="sr-section__title">Preset</h3>
        <div className="sr-chips" data-testid="convert-presets">
          {presetsForDirection.map((preset) => (
            <button
              key={preset.label}
              type="button"
              className={`sr-btn ${presetLabel === preset.label ? "sr-btn--primary" : ""}`}
              data-testid={`convert-preset-${preset.label}`}
              title={preset.description}
              aria-pressed={presetLabel === preset.label}
              onClick={() => selectPreset(preset.label)}
            >
              {preset.label}
            </button>
          ))}
        </div>

        {direction === "tag-to-tags" && (
          <div className="sr-field sr-field--stacked">
            <label className="sr-field__label" htmlFor="convert-source">
              Source tag
            </label>
            <select
              id="convert-source"
              className="sr-input"
              data-testid="convert-source"
              value={sourceTag}
              onChange={(event) => setSourceTag(event.target.value as ConvertSourceTag)}
            >
              {CONVERT_SOURCE_TAGS.map((tag) => (
                <option key={tag.key} value={tag.key}>
                  {tag.label}
                </option>
              ))}
            </select>
          </div>
        )}

        <div className="sr-field sr-field--stacked">
          <label className="sr-field__label" htmlFor="convert-pattern">
            {direction === "tags-to-filename" ? "Filename template" : "Pattern"}
          </label>
          <input
            id="convert-pattern"
            className="sr-input"
            data-testid="convert-pattern"
            spellCheck={false}
            placeholder={
              direction === "tags-to-filename"
                ? "%{track}% - %{title}%%{ext}%"
                : "%{track}% - %{title}%"
            }
            value={pattern}
            onChange={(event) => setPattern(event.target.value)}
          />
          <p className="sr-field__static">
            Use placeholders like %&#123;track&#125;% and %&#123;title&#125;% with the
            separators around them. Name templates keep the extension with %&#123;ext&#125;%.
          </p>
        </div>

        <h3 className="sr-section__title">Preview</h3>
        <div className="sr-field" data-testid="convert-preview">
          {!track ? (
            <p className="sr-field__static">Select a file to convert.</p>
          ) : preview.error ? (
            <p className="sr-field__static sr-field__static--error" data-testid="convert-preview-error">
              {preview.error}
            </p>
          ) : (
            <>
              <p className="sr-field__static">
                From <code>{preview.input}</code>
              </p>
              <p className="sr-field__static">
                To <code>{preview.output}</code>
              </p>
              {Object.keys(preview.fields).length > 0 && (
                <ul className="sr-chips" data-testid="convert-preview-fields">
                  {Object.entries(preview.fields).map(([key, value]) => (
                    <li key={key} className="sr-chip">
                      <span className="sr-cell--muted">{key.toUpperCase()}</span>=
                      <code>{String(value)}</code>
                    </li>
                  ))}
                </ul>
              )}
              {preview.isNoChange && (
                <p className="sr-field__static" data-testid="convert-preview-nochange">
                  No change — the result is identical to the current value.
                </p>
              )}
            </>
          )}
        </div>

        <p className="sr-field__static">
          {selectedCount > 0
            ? `${selectedCount} selected track(s).`
            : `No selection, so this covers all ${tracks.length} track(s) in the album.`}
        </p>

        <button
          type="button"
          className="sr-btn sr-btn--primary"
          data-testid="convert-apply"
          disabled={busy || !canConvert}
          title={!canConvert && preview.error ? preview.error : undefined}
          onClick={() => void apply()}
        >
          {busy ? "Converting…" : convertLabel}
        </button>
        {message && (
          <p className="sr-banner sr-banner--notice" data-testid="convert-result">
            {message}
          </p>
        )}
      </section>
    </ToolDrawer>
  );
}
