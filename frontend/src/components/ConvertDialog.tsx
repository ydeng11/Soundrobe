import React, { useState, useEffect, useRef } from "react";

export type ConvertDirection = "filename-to-title" | "title-to-filename" | "custom-regex";

interface ConvertDialogProps {
  open: boolean;
  onClose: () => void;
  onConvert: (direction: ConvertDirection, pattern: string) => void;
}

const DIRECTION_LABELS: Record<ConvertDirection, string> = {
  "filename-to-title": "Extract title from filename",
  "title-to-filename": "Rename file from title",
  "custom-regex": "Custom regex",
};

const DIRECTION_PLACEHOLDERS: Record<ConvertDirection, string> = {
  "filename-to-title": "Defaults: strip extension + leading track number",
  "title-to-filename": "Defaults: {title}{ext}",
  "custom-regex": "e.g. ^(\\d+)[\\s.-]+(.+)$ to capture track + title",
};

const DEFAULT_PATTERNS: Record<ConvertDirection, string> = {
  "filename-to-title": "strip-number",
  "title-to-filename": "title-as-name",
  "custom-regex": "",
};

export function ConvertDialog({ open, onClose, onConvert }: ConvertDialogProps) {
  const [direction, setDirection] = useState<ConvertDirection>("filename-to-title");
  const [pattern, setPattern] = useState(DEFAULT_PATTERNS["filename-to-title"]);
  const inputRef = useRef<HTMLInputElement>(null);

  // Reset state when dialog opens
  useEffect(() => {
    if (open) {
      setDirection("filename-to-title");
      setPattern(DEFAULT_PATTERNS["filename-to-title"]);
      setTimeout(() => inputRef.current?.focus(), 50);
    }
  }, [open]);

  useEffect(() => {
    setPattern(DEFAULT_PATTERNS[direction]);
  }, [direction]);

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    onConvert(direction, pattern);
    onClose();
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") {
      onClose();
    }
  };

  if (!open) return null;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/20 backdrop-blur-sm"
      onClick={onClose}
      onKeyDown={handleKeyDown}
    >
      <div
        className="bg-white rounded-xl shadow-xl border border-border/60 w-[420px] overflow-hidden"
        onClick={(e) => e.stopPropagation()}
      >
        {/* Header */}
        <div className="flex items-center justify-between px-4 py-3 border-b border-border/50">
          <h2 className="text-[13px] font-semibold text-text-primary">Convert</h2>
          <button
            onClick={onClose}
            className="w-6 h-6 flex items-center justify-center rounded-md text-text-muted hover:text-text-primary hover:bg-surface-hover transition-colors"
          >
            <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <path d="M18 6 6 18" /><path d="m6 6 12 12" />
            </svg>
          </button>
        </div>

        <form onSubmit={handleSubmit}>
          <div className="p-4 space-y-3.5">
            {/* Direction */}
            <div>
              <label className="block text-[11px] font-medium text-text-muted uppercase tracking-wider mb-2">
                Direction
              </label>
              <div className="space-y-1.5">
                {(Object.keys(DIRECTION_LABELS) as ConvertDirection[]).map((d) => (
                  <label
                    key={d}
                    className={`flex items-center gap-2.5 px-3 py-2 rounded-lg cursor-pointer transition-colors ${
                      direction === d
                        ? "bg-accent/10 border border-accent/20"
                        : "hover:bg-surface-hover border border-transparent"
                    }`}
                  >
                    <input
                      type="radio"
                      name="direction"
                      value={d}
                      checked={direction === d}
                      onChange={() => setDirection(d)}
                      className="accent-accent"
                    />
                    <div>
                      <div className="text-[12.5px] font-medium text-text-primary">
                        {DIRECTION_LABELS[d]}
                      </div>
                      <div className="text-[10.5px] text-text-muted mt-0.5">
                        {d === "filename-to-title"
                          ? "Extract metadata from the filename and write to tags"
                          : d === "title-to-filename"
                            ? "Rename the audio file based on its title tag"
                            : "Apply a custom regex to transform filenames"}
                      </div>
                    </div>
                  </label>
                ))}
              </div>
            </div>

            {/* Regex / Pattern */}
            <div>
              <label className="block text-[11px] font-medium text-text-muted uppercase tracking-wider mb-1.5">
                {direction === "custom-regex" ? "Regex pattern" : "Pattern"}
              </label>
              <div className="relative">
                <input
                  ref={inputRef}
                  type="text"
                  value={pattern}
                  onChange={(e) => setPattern(e.target.value)}
                  placeholder={DIRECTION_PLACEHOLDERS[direction]}
                  className="w-full h-[32px] px-3 text-[12px] bg-surface-alt border border-border rounded-lg text-text-primary placeholder-text-muted/50 outline-none focus:border-accent/60 focus:bg-white transition-colors font-mono"
                  spellCheck={false}
                />
              </div>
              <p className="text-[10.5px] text-text-muted mt-1 leading-relaxed">
                {direction === "filename-to-title"
                  ? "Uses: strip-extension → strip leading number → trim. Leave the default for this."
                  : direction === "title-to-filename"
                    ? 'Uses: {title} for title tag, {ext} for original extension, {track} for track number. Default: "{title}{ext}"'
                    : "Enter a JavaScript regex with capture groups. The first capture becomes the new title."}
              </p>
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
              className="px-3.5 py-1.5 text-[11.5px] font-medium text-white bg-accent hover:bg-accent-dim rounded-lg transition-colors"
            >
              Convert
            </button>
          </div>
        </form>
      </div>
    </div>
  );
}
