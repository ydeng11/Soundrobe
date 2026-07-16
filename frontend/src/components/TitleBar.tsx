import React, { useState, useRef, useEffect, useCallback } from "react";
import type { OrderingRule } from "../shared/track-numbering";
import { ORDERING_RULE_LABELS } from "../shared/track-numbering";

/** All ordering rules, derived from the canonical label map. */
const NUMBERING_RULES = Object.keys(ORDERING_RULE_LABELS) as OrderingRule[];

interface TitleBarProps {
  libraryPath: string | null;
  trackCount: number;
  filterText: string;
  onFilterChange: (text: string) => void;
  selectedFilePath: string | null;
  saving: boolean;
  autoTagging: boolean;
  lyricsGetting: boolean;
  auditing: boolean;
  darkMode: boolean;
  error: string | null;
  onOpenLibrary: () => void;
  onRefresh: () => void;
  onConvert: () => void;
  onAutoTag: () => void;
  onGetLyrics: () => void;
  onAudit: () => void;
  onNumberTracks: (rule: OrderingRule) => void;
  activeAlbumPath: string | null;
  onToggleDarkMode: () => void;
  onOpenSettings: () => void;
  onToggleAssistant: () => void;
  onErrorDismiss: () => void;
}



export function TitleBar({
  libraryPath,
  trackCount,
  filterText,
  onFilterChange,
  selectedFilePath,
  saving,
  autoTagging,
  lyricsGetting,
  auditing,
  darkMode,
  error,
  onOpenLibrary,
  onRefresh,
  onConvert,
  onAutoTag,
  onGetLyrics,
  onAudit,
  onNumberTracks,
  activeAlbumPath,
  onToggleDarkMode,
  onOpenSettings,
  onToggleAssistant,
  onErrorDismiss,
}: TitleBarProps) {
  const [numberMenuOpen, setNumberMenuOpen] = useState(false);
  const numberBtnRef = useRef<HTMLButtonElement>(null);
  const numberMenuRef = useRef<HTMLDivElement>(null);

  // Close the numbering dropdown when clicking outside
  useEffect(() => {
    if (!numberMenuOpen) return;
    const handler = (e: MouseEvent) => {
      if (
        numberBtnRef.current &&
        !numberBtnRef.current.contains(e.target as Node) &&
        numberMenuRef.current &&
        !numberMenuRef.current.contains(e.target as Node)
      ) {
        setNumberMenuOpen(false);
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [numberMenuOpen]);

  const handleNumberClick = useCallback(() => {
    setNumberMenuOpen((prev) => !prev);
  }, []);

  const handleRuleSelect = useCallback(
    (rule: OrderingRule) => {
      setNumberMenuOpen(false);
      onNumberTracks(rule);
    },
    [onNumberTracks],
  );

  const numberDisabled = !libraryPath || !activeAlbumPath;

  return (
    <div className="flex items-center h-[38px] px-3 bg-white/95 backdrop-blur-md border-b border-border drag-region select-none gap-2">
      {/* Spacer for traffic light controls (70px accounts for native red/yellow/green) */}
      <div className="w-[70px] shrink-0" />

      {/* Open Library */}
      <button
        onClick={onOpenLibrary}
        className="no-drag inline-flex items-center gap-1.5 px-3 py-1 text-[11.5px] font-medium text-text-secondary hover:text-text-primary bg-transparent hover:bg-surface-hover rounded-md transition-all active:scale-[0.97] whitespace-nowrap"
        title="Open music library (⌘O)"
      >
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
          <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
        </svg>
        <span>Open Library</span>
      </button>

      {/* Library path */}
      {libraryPath && (
        <div className="flex items-center gap-1.5 text-[11px] text-text-muted truncate max-w-[180px] no-drag">
          <span className="truncate">{libraryPath}</span>
          <span className="text-text-muted/50 tabular-nums">({trackCount})</span>
          <button
            onClick={onRefresh}
            className="inline-flex items-center justify-center w-5 h-5 text-text-muted hover:text-text-primary rounded hover:bg-surface-hover transition-all shrink-0"
            title="Refresh library (⌘R)"
          >
            <svg width="13" height="13" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
              <polyline points="23 4 23 10 17 10" />
              <polyline points="1 20 1 14 7 14" />
              <path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15" />
            </svg>
          </button>
        </div>
      )}

      <div className="w-px h-4 bg-border no-drag" />

      {/* Capsule Search Bar */}
      <div className="flex items-center flex-1 max-w-[260px] no-drag">
        <div className="relative w-full group">
          <div className="absolute inset-y-0 left-0 flex items-center pl-2.5 pointer-events-none">
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="#aeaeb2" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="transition-colors duration-200 group-focus-within:stroke-accent">
              <circle cx="11" cy="11" r="8" />
              <path d="m21 21-4.3-4.3" />
            </svg>
          </div>
          <input
            type="text"
            value={filterText}
            onChange={(e) => onFilterChange(e.target.value)}
            placeholder="Filter files..."
            className="w-full h-[26px] bg-surface-alt/80 border border-border/80 rounded-full pl-8 pr-3 text-[12px] text-text-primary placeholder-text-muted/60 outline-none transition-all duration-200 focus:border-accent/60 focus:bg-white focus:shadow-[0_0_0_3px_rgba(0,122,255,0.12)]"
          />
          {filterText && (
            <button
              onClick={() => onFilterChange("")}
              className="absolute inset-y-0 right-0 flex items-center pr-2.5 text-text-muted hover:text-text-secondary transition-colors"
            >
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                <path d="M18 6 6 18" /><path d="m6 6 12 12" />
              </svg>
            </button>
          )}
        </div>
      </div>

      <div className="flex-1" />

      {/* Auto-Tag button */}
      <button
        onClick={onAutoTag}
        disabled={!libraryPath || autoTagging}
        className={`no-drag inline-flex items-center gap-1.5 px-3 py-1 text-[11.5px] font-medium rounded-md transition-all duration-200 active:scale-[0.95] hover:scale-[1.03] ${
          autoTagging
            ? "text-accent/60 cursor-wait"
            : "text-accent hover:bg-accent/10"
        }`}
        title="Auto-tag selected album or all (⌘T)"
      >
        {autoTagging ? (
          <SpinnerIcon />
        ) : (
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
            <polygon points="5 3 19 12 5 21 5 3" />
          </svg>
        )}
        <span>{autoTagging ? "Tagging…" : "Auto-Tag"}</span>
      </button>

      {/* Get Lyrics button */}
      <button
        onClick={onGetLyrics}
        disabled={!libraryPath || lyricsGetting}
        className={`no-drag inline-flex items-center gap-1.5 px-3 py-1 text-[11.5px] font-medium rounded-md transition-all duration-200 active:scale-[0.95] hover:scale-[1.03] ${
          lyricsGetting
            ? "text-[#34c759]/60 cursor-wait"
            : "text-[#34c759] hover:bg-[#34c759]/10"
        }`}
        title="Download missing lyrics and fix encoding of existing .lrc/.txt files"
      >
        {lyricsGetting ? (
          <SpinnerIcon />
        ) : (
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
            <path d="M9 18V5l12-2v13" />
            <circle cx="6" cy="18" r="3" />
            <circle cx="18" cy="16" r="3" />
          </svg>
        )}
        <span>{lyricsGetting ? "Getting…" : "Get Lyrics"}</span>
      </button>

      <div className="w-px h-4 bg-border no-drag" />

      {/* Audit button */}
      <button
        onClick={onAudit}
        disabled={!libraryPath || auditing}
        className={`no-drag inline-flex items-center gap-1.5 px-3 py-1 text-[11.5px] font-medium rounded-md transition-all duration-200 active:scale-[0.95] hover:scale-[1.03] ${
          auditing
            ? "text-[#ff9f0a]/60 cursor-wait"
            : "text-[#ff9f0a] hover:bg-[#ff9f0a]/10"
        }`}
        title="Run LLM audit to verify metadata matches file paths"
      >
        {auditing ? (
          <SpinnerIcon />
        ) : (
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
            <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" />
          </svg>
        )}
        <span>{auditing ? "Auditing…" : "Audit"}</span>
      </button>

      <div className="w-px h-4 bg-border no-drag" />

      {/* Number button with dropdown */}
      <div className="relative no-drag">
        <button
          ref={numberBtnRef}
          onClick={handleNumberClick}
          disabled={numberDisabled}
          className={`inline-flex items-center gap-1.5 px-3 py-1 text-[11.5px] font-medium rounded-md transition-all duration-200 active:scale-[0.95] hover:scale-[1.03] ${
            numberDisabled
              ? "text-text-muted/40 cursor-not-allowed"
              : "text-[#5e5ce6] hover:bg-[#5e5ce6]/10"
          }`}
          title="Auto-number tracks within the selected album"
        >
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
            <line x1="4" y1="9" x2="20" y2="9" />
            <line x1="4" y1="15" x2="20" y2="15" />
            <line x1="4" y1="12" x2="20" y2="12" />
            <line x1="4" y1="6" x2="12" y2="6" />
            <line x1="4" y1="18" x2="12" y2="18" />
          </svg>
          <span>Number</span>
        </button>

        {numberMenuOpen && !numberDisabled && (
          <div
            ref={numberMenuRef}
            className="absolute right-0 top-full mt-1 z-50 min-w-[200px] bg-white border border-border rounded-lg shadow-lg py-1"
          >
            <div className="px-3 py-1.5 text-[10px] font-semibold uppercase tracking-widest text-text-muted">
              Number tracks by…
            </div>
            <div className="h-px bg-border/50 mx-2 my-0.5" />
            {NUMBERING_RULES.map((rule) => (
              <button
                key={rule}
                onClick={() => handleRuleSelect(rule)}
                className="w-full flex items-center gap-2 px-3 py-1.5 text-[12px] text-left text-text-secondary hover:bg-surface-hover hover:text-text-primary transition-colors"
              >
                {ORDERING_RULE_LABELS[rule]}
              </button>
            ))}
          </div>
        )}
      </div>

      <div className="w-px h-4 bg-border no-drag" />

      <ToolbarButton
        icon={
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
            <polyline points="23 4 23 10 17 10" />
            <polyline points="1 20 1 14 7 14" />
            <path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15" />
          </svg>
        }
        label="Convert"
        onClick={onConvert}
      />

      <div className="w-px h-4 bg-border no-drag" />

      {/* Assistant toggle */}
      <button
        onClick={onToggleAssistant}
        className="no-drag inline-flex items-center justify-center w-7 h-7 text-[#b4befe] hover:text-white rounded-lg hover:bg-[#b4befe]/10 transition-all"
        title="Assistant"
      >
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
          <path d="M12 8V4m0 0L9 7m3-3l3 3" />
          <path d="M4 17L2 19v2h4v-2l-2-2z" />
          <path d="M20 17l2 2v2h-4v-2l2-2z" />
          <path d="M7 14l2.5 4h5L17 14" />
          <path d="M12 14l-.5-.5" />
          <circle cx="12" cy="12" r="3" />
          <circle cx="12" cy="12" r="9" strokeWidth="1.5" />
        </svg>
      </button>

      {/* Dark mode toggle */}
      <button
        onClick={onToggleDarkMode}
        className="no-drag inline-flex items-center justify-center w-7 h-7 text-text-muted hover:text-text-primary rounded-lg hover:bg-surface-hover transition-all"
        title={darkMode ? "Switch to light mode" : "Switch to dark mode"}
      >
        {darkMode ? (
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
            <circle cx="12" cy="12" r="5" />
            <line x1="12" y1="1" x2="12" y2="3" />
            <line x1="12" y1="21" x2="12" y2="23" />
            <line x1="4.22" y1="4.22" x2="5.64" y2="5.64" />
            <line x1="18.36" y1="18.36" x2="19.78" y2="19.78" />
            <line x1="1" y1="12" x2="3" y2="12" />
            <line x1="21" y1="12" x2="23" y2="12" />
            <line x1="4.22" y1="19.78" x2="5.64" y2="18.36" />
            <line x1="18.36" y1="5.64" x2="19.78" y2="4.22" />
          </svg>
        ) : (
          <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
            <path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z" />
          </svg>
        )}
      </button>

      {/* Settings gear */}
      <button
        onClick={onOpenSettings}
        className="no-drag inline-flex items-center justify-center w-7 h-7 text-text-muted hover:text-text-primary rounded-lg hover:bg-surface-hover transition-all"
        title="Settings"
      >
        <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
          <circle cx="12" cy="12" r="3" />
          <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z" />
        </svg>
      </button>

      {/* Status indicators */}
      <div className="flex items-center gap-2 text-[11px] text-text-muted no-drag">
        {error ? (
          <button
            onClick={onErrorDismiss}
            className="text-[#ff3b30] flex items-center gap-1 cursor-pointer hover:opacity-70 transition-opacity"
            title="Click to dismiss"
          >
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
              <circle cx="12" cy="12" r="10" /><line x1="12" y1="8" x2="12" y2="12" /><line x1="12" y1="16" x2="12.01" y2="16" />
            </svg>
            {error}
            <svg width="10" height="10" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" className="ml-1 opacity-50">
              <path d="M18 6 6 18" /><path d="m6 6 12 12" />
            </svg>
          </button>
        ) : (
          <>
            {saving && (
              <span className="text-accent flex items-center gap-1">
                <span className="w-1.5 h-1.5 rounded-full bg-accent animate-pulse" />
                Saving
              </span>
            )}
            {selectedFilePath && !saving ? (
              <span className="text-accent tabular-nums">1 selected</span>
            ) : (
              !saving && <span className="tabular-nums">{trackCount} file{trackCount !== 1 ? "s" : ""}</span>
            )}
          </>
        )}
      </div>
    </div>
  );
}

// ── Shared sub-components ───────────────────────────────────────────

function SpinnerIcon() {
  return (
    <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" className="animate-spin">
      <line x1="12" y1="2" x2="12" y2="6" />
      <line x1="12" y1="18" x2="12" y2="22" />
      <line x1="4.93" y1="4.93" x2="7.76" y2="7.76" />
      <line x1="16.24" y1="16.24" x2="19.07" y2="19.07" />
      <line x1="2" y1="12" x2="6" y2="12" />
      <line x1="18" y1="12" x2="22" y2="12" />
      <line x1="4.93" y1="19.07" x2="7.76" y2="16.24" />
      <line x1="16.24" y1="7.76" x2="19.07" y2="4.93" />
    </svg>
  );
}

function ToolbarButton({
  icon,
  label,
  disabled,
  onClick,
}: {
  icon: React.ReactNode;
  label: string;
  disabled?: boolean;
  onClick?: () => void;
}) {
  return (
    <button
      disabled={disabled}
      onClick={onClick}
      className={`no-drag inline-flex items-center gap-1 px-2 py-1 text-[11.5px] font-medium rounded-md transition-all active:scale-[0.97] ${
        disabled
          ? "text-text-muted/40 cursor-not-allowed"
          : "text-text-secondary hover:text-text-primary hover:bg-surface-hover"
      }`}
    >
      <span className="w-3.5 h-3.5 flex items-center justify-center">{icon}</span>
      <span>{label}</span>
    </button>
  );
}
