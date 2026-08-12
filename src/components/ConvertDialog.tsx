import React, { useState, useEffect, useRef, useMemo } from "react";
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

export type { ConvertDirection, ConvertFieldMap, ConvertTrackData as TrackPreviewData };

export interface ConvertResult {
  direction: ConvertDirection;
  pattern: string;
  presetLabel: string;
  writeFields: ConvertFieldMap;
  sourceFilename?: string;
  sourceTag?: ConvertSourceTag;
  sourceValue?: string;
  newFilename?: string;
  filenameTemplate?: string;
}

interface ConvertDialogProps {
  open: boolean;
  onClose: () => void;
  onConvert: (result: ConvertResult) => void;
  tracks: ConvertTrackData[];
}

export function ConvertDialog({
  open,
  onClose,
  onConvert,
  tracks,
}: ConvertDialogProps) {
  const track = tracks.length > 0 ? tracks[0] : null;
  const trackCount = tracks.length;
  const [direction, setDirection] = useState<ConvertDirection>(
    "filename-to-tags",
  );
  const [selectedPresetLabel, setSelectedPresetLabel] = useState<string>(
    DEFAULT_CONVERT_PRESETS[0].label,
  );
  const [pattern, setPattern] = useState(DEFAULT_CONVERT_PRESETS[0].pattern);
  const [sourceTag, setSourceTag] = useState<ConvertSourceTag>("title");
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (!open) return;
    const first = DEFAULT_CONVERT_PRESETS[0];
    setDirection(first.direction);
    setSelectedPresetLabel(first.label);
    setPattern(first.pattern);
    setSourceTag(first.sourceTag ?? "title");
    setTimeout(() => inputRef.current?.focus(), 50);
  }, [open]);

  const presetsForDirection = useMemo(
    () => DEFAULT_CONVERT_PRESETS.filter((p) => p.direction === direction),
    [direction],
  );

  useEffect(() => {
    const selected = DEFAULT_CONVERT_PRESETS.find(
      (p) => p.label === selectedPresetLabel && p.direction === direction,
    );
    if (selected) return;
    const first = DEFAULT_CONVERT_PRESETS.find((p) => p.direction === direction);
    if (!first) return;
    setSelectedPresetLabel(first.label);
    setPattern(first.pattern);
    setSourceTag(first.sourceTag ?? "title");
  }, [direction, selectedPresetLabel]);

  const preview = useMemo<{
    input: string | null;
    output: string | null;
    fields: ConvertFieldMap | null;
    error: string | null;
    isNoChange: boolean;
    sourceValue: string | null;
  }>(() => {
    if (!track) {
      return {
        input: null,
        output: null,
        fields: null,
        error: null,
        isNoChange: false,
        sourceValue: null,
      };
    }

    const trimmedPattern = pattern.trim();
    if (!trimmedPattern) {
      return {
        input: direction === "tag-to-tags" ? getConvertSourceValue(track, sourceTag) : track.filename,
        output: null,
        fields: null,
        error:
          direction === "tags-to-filename"
            ? "Enter a template like %{track}% - %{title}%%{ext}%"
            : "Enter a pattern like %{track}% - %{title}%",
        isNoChange: false,
        sourceValue: null,
      };
    }

    if (direction === "tags-to-filename") {
      const newFilename = buildFilenameFromConvertPattern(trimmedPattern, track);
      return {
        input: track.filename,
        output: newFilename,
        fields: null,
        error: null,
        isNoChange: newFilename === track.filename,
        sourceValue: null,
      };
    }

    const sourceValue =
      direction === "filename-to-tags"
        ? track.filename
        : getConvertSourceValue(track, sourceTag);
    if (direction === "tag-to-tags" && !sourceValue.trim()) {
      return {
        input: sourceValue,
        output: null,
        fields: null,
        error: `${CONVERT_FIELD_LABELS[sourceTag] ?? sourceTag} is empty`,
        isNoChange: false,
        sourceValue,
      };
    }

    const parsed =
      direction === "filename-to-tags"
        ? parseFilenameWithConvertPattern(trimmedPattern, track.filename)
        : parseTextWithConvertPattern(trimmedPattern, sourceValue);
    if ("error" in parsed) {
      return {
        input: sourceValue,
        output: null,
        fields: null,
        error: parsed.error,
        isNoChange: false,
        sourceValue,
      };
    }

    const currentValues: Record<string, string> = {
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
    const isNoChange =
      Object.keys(parsed.fields).length > 0 &&
      Object.entries(parsed.fields).every(
        ([key, value]) => currentValues[key] === value,
      );

    return {
      input: sourceValue,
      output: parsed.displayResult,
      fields: parsed.fields,
      error: null,
      isNoChange,
      sourceValue,
    };
  }, [direction, pattern, sourceTag, track]);

  const handlePresetSelect = (presetLabel: string) => {
    const preset = DEFAULT_CONVERT_PRESETS.find((p) => p.label === presetLabel);
    if (!preset) return;
    setSelectedPresetLabel(preset.label);
    setPattern(preset.pattern);
    setSourceTag(preset.sourceTag ?? "title");
  };

  const handleSubmit = (event: React.FormEvent) => {
    event.preventDefault();
    if (!track || !preview.output || preview.error || preview.isNoChange) return;

    const trimmedPattern = pattern.trim();

    switch (direction) {
      case "tags-to-filename":
        onConvert({
          direction,
          pattern: trimmedPattern,
          presetLabel: selectedPresetLabel,
          writeFields: {},
          filenameTemplate: trimmedPattern,
          newFilename: preview.output,
        });
        break;

      case "filename-to-tags":
        onConvert({
          direction,
          pattern: trimmedPattern,
          presetLabel: selectedPresetLabel,
          writeFields: preview.fields ?? {},
          sourceFilename: track.filename,
        });
        break;

      case "tag-to-tags":
        onConvert({
          direction,
          pattern: trimmedPattern,
          presetLabel: selectedPresetLabel,
          writeFields: preview.fields ?? {},
          sourceTag,
          sourceValue: preview.sourceValue ?? "",
        });
        break;
    }
    onClose();
  };

  const canConvert =
    track && preview.output !== null && !preview.error && !preview.isNoChange;

  const convertLabel =
    trackCount > 1 ? `Convert (${trackCount} tracks)` : "Convert";

  if (!open) return null;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/20 backdrop-blur-sm"
      onClick={onClose}
      onKeyDown={(event) => {
        if (event.key === "Escape") onClose();
      }}
    >
      <div
        role="dialog"
        aria-label="Convert"
        className="bg-white rounded-xl shadow-xl border border-border/60 w-[520px] overflow-hidden"
        onClick={(event) => event.stopPropagation()}
      >
        <div className="flex items-center justify-between px-4 py-3 border-b border-border/50">
          <div className="flex items-center gap-2">
            <h2 className="text-[13px] font-semibold text-text-primary">
              Convert
            </h2>
            {trackCount > 1 && (
              <span className="px-1.5 py-0.5 text-[10px] font-medium bg-accent/10 text-accent border border-accent/20 rounded-md">
                {trackCount} tracks
              </span>
            )}
          </div>
          <button
            type="button"
            aria-label="Close"
            onClick={onClose}
            className="w-6 h-6 flex items-center justify-center rounded-md text-text-muted hover:text-text-primary hover:bg-surface-hover transition-colors"
          >
            x
          </button>
        </div>

        <form onSubmit={handleSubmit}>
          <div className="p-4 space-y-3.5">
            <div>
              <label className="block text-[11px] font-medium text-text-muted uppercase tracking-wider mb-2">
                Direction
              </label>
              <div className="grid grid-cols-3 gap-1.5">
                <DirectionButton
                  active={direction === "filename-to-tags"}
                  label="Filename -> Tags"
                  onClick={() => setDirection("filename-to-tags")}
                />
                <DirectionButton
                  active={direction === "tag-to-tags"}
                  label="Tag -> Tags"
                  onClick={() => setDirection("tag-to-tags")}
                />
                <DirectionButton
                  active={direction === "tags-to-filename"}
                  label="Tags -> Filename"
                  onClick={() => setDirection("tags-to-filename")}
                />
              </div>
            </div>

            <div>
              <label className="block text-[11px] font-medium text-text-muted uppercase tracking-wider mb-1.5">
                Preset
              </label>
              <div className="flex flex-wrap gap-1.5">
                {presetsForDirection.map((preset) => (
                  <button
                    key={preset.label}
                    type="button"
                    onClick={() => handlePresetSelect(preset.label)}
                    title={preset.description}
                    className={
                      "px-2.5 py-1 text-[11px] font-medium rounded-md transition-colors border " +
                      (selectedPresetLabel === preset.label
                        ? "bg-accent/10 border-accent/30 text-accent"
                        : "bg-surface-alt border-border text-text-secondary hover:bg-surface-hover")
                    }
                  >
                    {preset.label}
                  </button>
                ))}
              </div>
            </div>

            {direction === "tag-to-tags" && (
              <div>
                <label className="block text-[11px] font-medium text-text-muted uppercase tracking-wider mb-1.5">
                  Source Tag
                </label>
                <select
                  value={sourceTag}
                  onChange={(event) =>
                    setSourceTag(event.target.value as ConvertSourceTag)
                  }
                  className="w-full h-[32px] px-3 text-[12px] bg-surface-alt border border-border rounded-lg text-text-primary outline-none focus:border-accent/60 focus:bg-white transition-colors"
                >
                  {CONVERT_SOURCE_TAGS.map((tag) => (
                    <option key={tag.key} value={tag.key}>
                      {tag.label}
                    </option>
                  ))}
                </select>
              </div>
            )}

            <div>
              <label className="block text-[11px] font-medium text-text-muted uppercase tracking-wider mb-1.5">
                {direction === "tags-to-filename" ? "Filename Template" : "Pattern"}
              </label>
              <input
                ref={inputRef}
                type="text"
                value={pattern}
                onChange={(event) => setPattern(event.target.value)}
                placeholder={
                  direction === "tags-to-filename"
                    ? "%{track}% - %{title}%%{ext}%"
                    : "%{track}% - %{title}%"
                }
                className="w-full h-[32px] px-3 text-[12px] bg-surface-alt border border-border rounded-lg text-text-primary placeholder-text-muted/50 outline-none focus:border-accent/60 focus:bg-white transition-colors font-mono"
                spellCheck={false}
              />
              <p className="text-[10.5px] text-text-muted mt-1 leading-relaxed">
                {"Use placeholders like %{track}% and %{title}% with the separators around them."}
              </p>
            </div>

            <div>
              <label className="block text-[11px] font-medium text-text-muted uppercase tracking-wider mb-1.5">
                Preview
              </label>
              <div className="bg-surface-alt rounded-lg border border-border p-3 space-y-2 min-h-[60px]">
                {!track ? (
                  <div className="text-[11px] text-text-muted italic">
                    {trackCount === 0
                      ? "Select a file to convert"
                      : "Track not available"}
                  </div>
                ) : preview.error ? (
                  <div className="text-[11px] text-[#ff3b30]">
                    {preview.error}
                  </div>
                ) : (
                  <>
                    <PreviewRow label="From" value={preview.input ?? ""} />
                    <div className="flex items-center gap-2">
                      <span className="w-14 shrink-0" />
                      <span className="text-[12px] text-accent">{"->"}</span>
                    </div>
                    <PreviewRow label="To" value={preview.output ?? ""} accent />

                    {preview.fields && Object.keys(preview.fields).length > 0 && (
                      <div className="mt-1.5 pt-1.5 border-t border-border/50">
                        <div className="text-[10px] font-medium text-text-muted uppercase tracking-wider mb-1">
                          Fields to write
                        </div>
                        <div className="flex flex-wrap gap-1.5">
                          {Object.entries(preview.fields).map(([key, value]) => (
                            <span
                              key={key}
                              className="inline-flex items-center gap-1 px-1.5 py-0.5 bg-accent/8 border border-accent/15 rounded text-[10.5px] text-accent font-medium"
                            >
                              <span className="uppercase opacity-70">{key}</span>
                              <span>=</span>
                              <span className="font-mono">{String(value)}</span>
                            </span>
                          ))}
                        </div>
                      </div>
                    )}

                    {preview.isNoChange && (
                      <div className="mt-1 pt-1.5 border-t border-border/50">
                        <div className="text-[11px] text-text-muted italic">
                          No change - result is identical to current value
                        </div>
                      </div>
                    )}
                  </>
                )}
              </div>
            </div>
          </div>

          <div className="flex items-center justify-end gap-2 px-4 py-3 bg-surface-alt/50 border-t border-border/50">
            <button
              type="button"
              onClick={onClose}
              className="px-3.5 py-1.5 text-[11.5px] font-medium text-text-secondary hover:text-text-primary bg-white border border-border rounded-lg hover:bg-surface-hover transition-colors"
            >
              Cancel
            </button>
            <button
              type="submit"
              disabled={!canConvert}
              className={
                "px-3.5 py-1.5 text-[11.5px] font-medium rounded-lg transition-colors " +
                (canConvert
                  ? "text-white bg-accent hover:bg-accent-dim cursor-pointer"
                  : "text-text-muted bg-surface-alt cursor-not-allowed")
              }
              title={!canConvert && track && preview.error ? preview.error : undefined}
            >
              {convertLabel}
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}

function DirectionButton({
  active,
  label,
  onClick,
}: {
  active: boolean;
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={
        "px-3 py-2 text-[12px] font-medium rounded-lg transition-colors " +
        (active
          ? "bg-accent/10 border border-accent/20 text-accent"
          : "bg-surface-alt border border-border text-text-secondary hover:bg-surface-hover")
      }
    >
      {label}
    </button>
  );
}

function PreviewRow({
  label,
  value,
  accent = false,
}: {
  label: string;
  value: string;
  accent?: boolean;
}) {
  return (
    <div className="flex items-start gap-2">
      <span className="text-[10px] font-medium text-text-muted uppercase tracking-wider mt-0.5 w-14 shrink-0">
        {label}
      </span>
      <span
        className={
          "text-[11.5px] font-mono break-all " +
          (accent ? "text-accent" : "text-text-primary")
        }
      >
        {value}
      </span>
    </div>
  );
}
