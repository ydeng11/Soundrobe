import React, { useState, useEffect, useRef, useMemo } from "react";

// ── Types ─────────────────────────────────────────────────────────

export type ConvertDirection = "filename-to-tags" | "tags-to-filename";

export interface ConvertPreset {
  label: string;
  direction: ConvertDirection;
  /** Simple pattern with %field% placeholders, e.g. "%track% - %artist% - %title%" */
  pattern: string;
  /** Human-readable description */
  description: string;
}

export interface ConvertFieldMap {
  title?: string;
  artist?: string;
  album?: string;
  year?: string;
  track?: string;
  genre?: string;
  albumArtist?: string;
  composer?: string;
  comment?: string;
  discNumber?: string;
}

export interface ConvertResult {
  direction: ConvertDirection;
  /** The pattern that was applied (%field% or raw regex) */
  pattern: string;
  /** The preset label if one was selected, or "Custom" / "Regex" */
  presetLabel: string;
  /** Whether this is a raw regex pattern (user typed regex directly) */
  isRawRegex: boolean;
  /** For filename→tags: the fields extracted from the filename */
  writeFields: ConvertFieldMap;
  /** For filename→tags: the full filename that was parsed */
  sourceFilename?: string;
  /** For tags→filename: the new filename (basename only, no dir) */
  newFilename?: string;
  /** For tags→filename: the template used */
  filenameTemplate?: string;
}

export interface TrackPreviewData {
  /** Basename of the file (used as input for filename→tags) */
  filename: string;
  /** Current tag values (used as input for tags→filename) */
  title: string | null;
  artist: string | null;
  album: string | null;
  year: string | null;
  track: number | null;
  genre: string | null;
  albumArtist: string | null;
  composer: string | null;
  comment: string | null;
  discNumber: number | null;
}

interface ConvertDialogProps {
  open: boolean;
  onClose: () => void;
  onConvert: (result: ConvertResult) => void;
  track: TrackPreviewData | null;
}

// ── Default Presets (simple %field% syntax) ──────────────────────

const DEFAULT_PRESETS: ConvertPreset[] = [
  {
    label: "Track \u2014 Title",
    direction: "filename-to-tags",
    pattern: "%track% - %title%",
    description: "01 - Song Title \u2192 track=01, title=Song Title",
  },
  {
    label: "Artist \u2014 Title",
    direction: "filename-to-tags",
    pattern: "%artist% - %title%",
    description: "Artist - Song Title \u2192 artist=Artist, title=Song Title",
  },
  {
    label: "Track \u2014 Artist \u2014 Title",
    direction: "filename-to-tags",
    pattern: "%track% - %artist% - %title%",
    description: "2 - ABC - XYZ \u2192 track=2, artist=ABC, title=XYZ",
  },
  {
    label: "Title \u2192 Filename",
    direction: "tags-to-filename",
    pattern: "%track% - %title%%ext%",
    description: "Build filename from tags: %track% - %title%%ext%",
  },
  {
    label: "Artist \u2014 Title filename",
    direction: "tags-to-filename",
    pattern: "%artist% - %title%%ext%",
    description: "Build filename from tags: %artist% - %title%%ext%",
  },
];

// ── Helpers ───────────────────────────────────────────────────────

const FIELD_LABELS: Record<string, string> = {
  title: "Title",
  artist: "Artist",
  album: "Album",
  year: "Year",
  track: "Track",
  genre: "Genre",
  albumArtist: "Album Artist",
  composer: "Composer",
  comment: "Comment",
  discNumber: "Disc",
};

function padTrack(n: number | null): string {
  if (n == null) return "01";
  return String(n).padStart(2, "0");
}

/**
 * Convert a simple %field% pattern into a regex with named capture groups.
 *
 * Examples:
 *   "%track% - %artist% - %title%"  =>  /^(?<track>\d+)\s+-\s+(?<artist>.+?)\s+-\s+(?<title>.+)$/
 *   "%artist% - %track%.%title%%ext%"  =>  /^(?<artist>.+?)\s+-\s+(?<track>\d+)\.(?<title>.+)$/
 *
 * Rules:
 *  - %track%   => matches digits only          => (?<track>\d+)
 *  - %year%    => matches 4 digits             => (?<year>\d{4})
 *  - %ext%     => matches file extension       => (?<ext>\.[^.]+)
 *  - %filename% => matches any text            => (?<filename>.+)
 *  - all other %field% => matches any text     => (?<name>.+?)  (non-greedy)
 *  - the last text group uses greedy matching
 *  - whitespace in literal parts becomes \s+ for flexible matching
 *  - regex special chars in literals are escaped
 */
function patternToRegex(pattern: string): string {
  const tokens: Array<{ type: "literal" | "field"; value: string }> = [];
  let remaining = pattern;
  while (remaining.length > 0) {
    const fieldMatch = remaining.match(/^%(\w+)%/);
    if (fieldMatch) {
      tokens.push({ type: "field", value: fieldMatch[1] });
      remaining = remaining.slice(fieldMatch[0].length);
    } else {
      const nextPct = remaining.indexOf("%");
      const literalEnd = nextPct === -1 ? remaining.length : nextPct;
      tokens.push({ type: "literal", value: remaining.slice(0, literalEnd) });
      remaining = remaining.slice(literalEnd);
    }
  }

  // Count ALL capture groups to know which is the very last one
  const allFieldTokens = tokens.filter((t) => t.type === "field");
  const totalGroups = allFieldTokens.length;

  let groupIndex = 0;
  const parts: string[] = ["^"];

  for (const token of tokens) {
    if (token.type === "literal") {
      for (const seg of token.value.split(/(\s+)/)) {
        if (/^\s+$/.test(seg)) {
          parts.push("\\s+");
        } else if (seg.length > 0) {
          parts.push(seg.replace(/[.+*?^${}()|[\]\\-]/g, "\\$&"));
        }
      }
    } else {
      const name = token.value;
      const isLastGroup = groupIndex >= totalGroups - 1;

      if (name === "track") {
        parts.push("(?<track>\\d+)");
      } else if (name === "year") {
        parts.push("(?<year>\\d{4})");
      } else if (name === "ext") {
        parts.push("(?<ext>\\.[^.]+)");
      } else if (name === "filename") {
        parts.push("(?<filename>.+)");
      } else {
        // Text field: non-greedy unless it's the very last group
        const quantifier = isLastGroup ? "+" : "+?";
        parts.push("(?<" + name + ">." + quantifier + ")");
      }
      groupIndex++;
    }
  }

  parts.push("$");
  return parts.join("");
}

/**
 * Check if a pattern uses the simple %field% syntax (vs raw regex).
 */
function isSimplePattern(pattern: string): boolean {
  return /%\w+%/.test(pattern);
}

/**
 * Parse a filename against a pattern and extract fields.
 * Supports both %field% syntax and raw regex.
 */
function parseFilenameWithPattern(
  pattern: string,
  filename: string
): { fields: ConvertFieldMap; displayResult: string } | { error: string } {
  const isSimple = isSimplePattern(pattern);
  const regexStr = isSimple ? patternToRegex(pattern) : pattern;

  try {
    const regex = new RegExp(regexStr);
    const matchName = pattern.includes("%ext%")
      ? filename
      : filename.replace(/\.[^.]+$/, "");
    const m = matchName.match(regex);
    if (!m) {
      return { error: "No match \u2014 pattern does not fit this filename" };
    }

    const fields: ConvertFieldMap = {};
    if (m.groups) {
      for (const [name, val] of Object.entries(m.groups)) {
        if (val !== undefined && typeof val === "string" && val.trim()) {
          fields[name as keyof ConvertFieldMap] = val.trim();
        }
      }
    }

    const parts = Object.entries(fields).map(
      ([k, v]) => (FIELD_LABELS[k] ?? k) + "=" + v
    );
    const displayResult =
      parts.length > 0 ? parts.join(", ") : "(no captures)";

    return { fields, displayResult };
  } catch (e) {
    const prefix = isSimple ? "Invalid pattern" : "Invalid regex";
    return { error: prefix + ": " + (e as Error).message };
  }
}

/**
 * Build a filename from a %field% template and current track data.
 * %track% -> padded track number, %title% -> title, %ext% -> .ext, etc.
 */
function buildFilenameFromPattern(
  pattern: string,
  data: TrackPreviewData | null
): string {
  if (!data) return pattern;
  return pattern
    .replace(/%title%/g, data.title ?? "Unknown")
    .replace(/%artist%/g, data.artist ?? "Unknown Artist")
    .replace(/%album%/g, data.album ?? "Unknown Album")
    .replace(/%year%/g, data.year ?? "0000")
    .replace(/%track%/g, padTrack(data.track))
    .replace(/%genre%/g, data.genre ?? "Unknown")
    .replace(/%disc%/g, padTrack(data.discNumber))
    .replace(
      /%ext%/g,
      data.filename.includes(".")
        ? "." + data.filename.split(".").pop()!
        : ".mp3"
    )
    .replace(/%filename%/g, data.filename.replace(/\.[^.]+$/, ""));
}

// ── Component ─────────────────────────────────────────────────────

export function ConvertDialog({
  open,
  onClose,
  onConvert,
  track,
}: ConvertDialogProps) {
  const [direction, setDirection] = useState<ConvertDirection>(
    "filename-to-tags"
  );
  const [selectedPresetLabel, setSelectedPresetLabel] = useState<string>(
    DEFAULT_PRESETS[0].label
  );
  const [customPattern, setCustomPattern] = useState("");
  const [useCustom, setUseCustom] = useState(false);
  const [useRegex, setUseRegex] = useState(false);
  const inputRef = useRef<HTMLInputElement>(null);

  // Reset state when dialog opens
  useEffect(() => {
    if (open) {
      setDirection("filename-to-tags");
      setSelectedPresetLabel(DEFAULT_PRESETS[0].label);
      setCustomPattern("");
      setUseCustom(false);
      setUseRegex(false);
      setTimeout(() => inputRef.current?.focus(), 50);
    }
  }, [open]);

  // Filter presets by current direction
  const presetsForDirection = useMemo(
    () => DEFAULT_PRESETS.filter((p) => p.direction === direction),
    [direction]
  );

  // Get current pattern
  const currentPattern = useMemo(() => {
    if (useCustom) return customPattern;
    const preset = DEFAULT_PRESETS.find((p) => p.label === selectedPresetLabel);
    return preset?.pattern ?? "";
  }, [useCustom, customPattern, selectedPresetLabel]);

  // Switch to first preset when direction changes
  useEffect(() => {
    const first = DEFAULT_PRESETS.find((p) => p.direction === direction);
    if (first) {
      setSelectedPresetLabel(first.label);
      setUseCustom(false);
      setUseRegex(false);
    }
  }, [direction]);

  // Compute preview
  const preview = useMemo<{
    input: string | null;
    output: string | null;
    fields: ConvertFieldMap | null;
    error: string | null;
    isNoChange: boolean;
  }>(() => {
    if (!track) {
      return {
        input: null,
        output: null,
        fields: null,
        error: null,
        isNoChange: false,
      };
    }

    if (!currentPattern) {
      return {
        input: track.filename,
        output: null,
        fields: null,
        error:
          direction === "filename-to-tags"
            ? "Enter a pattern like %track% - %title%"
            : "Enter a template like %track% - %title%%ext%",
        isNoChange: false,
      };
    }

    if (direction === "filename-to-tags") {
      const result = parseFilenameWithPattern(currentPattern, track.filename);
      if ("error" in result) {
        return {
          input: track.filename,
          output: null,
          fields: null,
          error: result.error,
          isNoChange: false,
        };
      }
      const titleField = result.fields.title?.trim();
      const noChange =
        titleField !== undefined &&
        titleField === (track.title ?? "").trim();
      return {
        input: track.filename,
        output: result.displayResult,
        fields: result.fields,
        error: null,
        isNoChange: noChange,
      };
    } else {
      const newFilename = buildFilenameFromPattern(currentPattern, track);
      return {
        input: track.filename,
        output: newFilename,
        fields: null,
        error: null,
        isNoChange: newFilename === track.filename,
      };
    }
  }, [direction, currentPattern, track]);

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (!track || !preview.output || preview.error || preview.isNoChange)
      return;

    const preset = DEFAULT_PRESETS.find(
      (p) => p.label === selectedPresetLabel
    );
    const wasRawRegex = useRegex || (useCustom && !isSimplePattern(customPattern));

    if (direction === "filename-to-tags") {
      onConvert({
        direction: "filename-to-tags",
        pattern: currentPattern,
        presetLabel: useCustom
          ? wasRawRegex
            ? "Regex"
            : "Custom"
          : preset?.label ?? "Custom",
        isRawRegex: wasRawRegex,
        writeFields: preview.fields ?? {},
        sourceFilename: track.filename,
      });
    } else {
      onConvert({
        direction: "tags-to-filename",
        pattern: currentPattern,
        presetLabel: useCustom ? "Custom" : preset?.label ?? "Custom",
        isRawRegex: false,
        writeFields: {},
        filenameTemplate: currentPattern,
        newFilename: preview.output,
      });
    }
    onClose();
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") {
      onClose();
    }
  };

  if (!open) return null;

  const canConvert =
    track && preview.output !== null && !preview.error && !preview.isNoChange;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/20 backdrop-blur-sm"
      onClick={onClose}
      onKeyDown={handleKeyDown}
    >
      <div
        className="bg-white rounded-xl shadow-xl border border-border/60 w-[480px] overflow-hidden"
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div className="flex items-center justify-between px-4 py-3 border-b border-border/50">
          <h2 className="text-[13px] font-semibold text-text-primary">
            Convert
          </h2>
          <button
            onClick={onClose}
            className="w-6 h-6 flex items-center justify-center rounded-md text-text-muted hover:text-text-primary hover:bg-surface-hover transition-colors"
          >
            <svg
              width="14"
              height="14"
              viewBox="0 0 24 24"
              fill="none"
              stroke="currentColor"
              strokeWidth="2"
              strokeLinecap="round"
              strokeLinejoin="round"
            >
              <path d="M18 6 6 18" />
              <path d="m6 6 12 12" />
            </svg>
          </button>
        </div>

        <form onSubmit={handleSubmit}>
          <div className="p-4 space-y-3.5">
            {/* Direction toggle */}
            <div>
              <label className="block text-[11px] font-medium text-text-muted uppercase tracking-wider mb-2">
                Direction
              </label>
              <div className="flex gap-1.5">
                <button
                  type="button"
                  onClick={() => setDirection("filename-to-tags")}
                  className={
                    "flex-1 px-3 py-2 text-[12px] font-medium rounded-lg transition-colors " +
                    (direction === "filename-to-tags"
                      ? "bg-accent/10 border border-accent/20 text-accent"
                      : "bg-surface-alt border border-border text-text-secondary hover:bg-surface-hover")
                  }
                >
                  Filename \u2192 Tags
                </button>
                <button
                  type="button"
                  onClick={() => setDirection("tags-to-filename")}
                  className={
                    "flex-1 px-3 py-2 text-[12px] font-medium rounded-lg transition-colors " +
                    (direction === "tags-to-filename"
                      ? "bg-accent/10 border border-accent/20 text-accent"
                      : "bg-surface-alt border border-border text-text-secondary hover:bg-surface-hover")
                  }
                >
                  Tags \u2192 Filename
                </button>
              </div>
            </div>

            {/* Presets */}
            <div>
              <label className="block text-[11px] font-medium text-text-muted uppercase tracking-wider mb-1.5">
                Preset
              </label>
              <div className="flex flex-wrap gap-1.5">
                {presetsForDirection.map((preset) => (
                  <button
                    key={preset.label}
                    type="button"
                    onClick={() => {
                      setSelectedPresetLabel(preset.label);
                      setUseCustom(false);
                      setUseRegex(false);
                    }}
                    title={preset.description}
                    className={
                      "px-2.5 py-1 text-[11px] font-medium rounded-md transition-colors border " +
                      (!useCustom && selectedPresetLabel === preset.label
                        ? "bg-accent/10 border-accent/30 text-accent"
                        : "bg-surface-alt border-border text-text-secondary hover:bg-surface-hover")
                    }
                  >
                    {preset.label}
                  </button>
                ))}
                <button
                  type="button"
                  onClick={() => {
                    setUseCustom(true);
                    setUseRegex(false);
                    setCustomPattern("");
                  }}
                  className={
                    "px-2.5 py-1 text-[11px] font-medium rounded-md transition-colors border " +
                    (useCustom && !useRegex
                      ? "bg-accent/10 border-accent/30 text-accent"
                      : "bg-surface-alt border-border text-text-secondary hover:bg-surface-hover")
                  }
                >
                  Custom
                </button>
                <button
                  type="button"
                  onClick={() => {
                    setUseCustom(true);
                    setUseRegex(true);
                    setCustomPattern("");
                  }}
                  className={
                    "px-2.5 py-1 text-[11px] font-medium rounded-md transition-colors border " +
                    (useRegex
                      ? "bg-accent/10 border-accent/30 text-accent"
                      : "bg-surface-alt border-border text-text-secondary hover:bg-surface-hover")
                  }
                >
                  Regex
                </button>
              </div>
            </div>

            {/* Pattern input */}
            <div>
              <label className="block text-[11px] font-medium text-text-muted uppercase tracking-wider mb-1.5">
                {useRegex
                  ? "Regex pattern"
                  : direction === "filename-to-tags"
                    ? "Filename pattern"
                    : "Filename template"}
              </label>
              <div className="relative">
                <input
                  ref={inputRef}
                  type="text"
                  value={useCustom ? customPattern : currentPattern}
                  onChange={(e) => {
                    if (useCustom) setCustomPattern(e.target.value);
                  }}
                  readOnly={!useCustom}
                  placeholder={
                    useRegex
                      ? "^(?<title>.+)$"
                      : direction === "filename-to-tags"
                        ? "%track% - %artist% - %title%"
                        : "%track% - %title%%ext%"
                  }
                  className={
                    "w-full h-[32px] px-3 text-[12px] bg-surface-alt border border-border rounded-lg text-text-primary placeholder-text-muted/50 outline-none focus:border-accent/60 focus:bg-white transition-colors font-mono " +
                    (!useCustom ? "opacity-60 cursor-not-allowed" : "")
                  }
                  spellCheck={false}
                />
              </div>
              <p className="text-[10.5px] text-text-muted mt-1 leading-relaxed">
                {useRegex
                  ? "Use named groups like (?<title>.+), (?<artist>.+), (?<track>\\d+)"
                  : direction === "filename-to-tags"
                    ? "Use %track%, %title%, %artist%, %year% to extract fields"
                    : "Use %track%, %title%, %artist%, %ext% as placeholders"}
              </p>
            </div>

            {/* Preview */}
            <div>
              <label className="block text-[11px] font-medium text-text-muted uppercase tracking-wider mb-1.5">
                Preview
              </label>
              <div className="bg-surface-alt rounded-lg border border-border p-3 space-y-2 min-h-[60px]">
                {!track ? (
                  <div className="text-[11px] text-text-muted italic">
                    Select a track to preview
                  </div>
                ) : preview.error ? (
                  <div className="text-[11px] text-[#ff3b30]">
                    {preview.error}
                  </div>
                ) : (
                  <>
                    <div className="flex items-start gap-2">
                      <span className="text-[10px] font-medium text-text-muted uppercase tracking-wider mt-0.5 w-14 shrink-0">
                        From
                      </span>
                      <span className="text-[11.5px] text-text-primary font-mono break-all">
                        {preview.input}
                      </span>
                    </div>

                    <div className="flex items-center gap-2">
                      <span className="w-14 shrink-0" />
                      <svg
                        width="14"
                        height="14"
                        viewBox="0 0 24 24"
                        fill="none"
                        stroke={preview.isNoChange ? "#aeaeb2" : "#007aff"}
                        strokeWidth="2"
                        strokeLinecap="round"
                        strokeLinejoin="round"
                      >
                        <path d="M5 12h14" />
                        <path d="m12 5 7 7-7 7" />
                      </svg>
                    </div>

                    <div className="flex items-start gap-2">
                      <span className="text-[10px] font-medium text-text-muted uppercase tracking-wider mt-0.5 w-14 shrink-0">
                        To
                      </span>
                      <span className="text-[11.5px] text-text-primary font-mono break-all">
                        {direction === "filename-to-tags" ? (
                          <span className="text-accent">
                            {preview.output}
                          </span>
                        ) : (
                          <span className="text-[#34c759]">
                            {preview.output}
                          </span>
                        )}
                      </span>
                    </div>

                    {direction === "filename-to-tags" &&
                      preview.fields &&
                      Object.keys(preview.fields).length > 0 && (
                        <div className="mt-1.5 pt-1.5 border-t border-border/50">
                          <div className="text-[10px] font-medium text-text-muted uppercase tracking-wider mb-1">
                            Fields to write:
                          </div>
                          <div className="flex flex-wrap gap-1.5">
                            {Object.entries(preview.fields).map(
                              ([key, val]) => (
                                <span
                                  key={key}
                                  className="inline-flex items-center gap-1 px-1.5 py-0.5 bg-accent/8 border border-accent/15 rounded text-[10.5px] text-accent font-medium"
                                >
                                  <span className="uppercase opacity-70">
                                    {key}
                                  </span>
                                  <span>=</span>
                                  <span className="font-mono">
                                    {String(val)}
                                  </span>
                                </span>
                              )
                            )}
                          </div>
                        </div>
                      )}

                    {preview.isNoChange && (
                      <div className="mt-1 pt-1.5 border-t border-border/50">
                        <div className="text-[11px] text-text-muted italic flex items-center gap-1">
                          <svg
                            width="12"
                            height="12"
                            viewBox="0 0 24 24"
                            fill="none"
                            stroke="currentColor"
                            strokeWidth="2"
                            strokeLinecap="round"
                            strokeLinejoin="round"
                          >
                            <circle cx="12" cy="12" r="10" />
                            <line x1="12" y1="8" x2="12" y2="12" />
                            <line x1="12" y1="16" x2="12.01" y2="16" />
                          </svg>
                          No change \u2014 result is identical to current value
                        </div>
                      </div>
                    )}
                  </>
                )}
              </div>
            </div>
          </div>

          {/* Footer */}
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
            >
              Convert
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
