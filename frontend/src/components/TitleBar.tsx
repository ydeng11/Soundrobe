import React from "react";

interface TitleBarProps {
  libraryPath: string | null;
  trackCount: number;
  filterText: string;
  onFilterChange: (text: string) => void;
  selectedFilePath: string | null;
  dirtyCount: number;
  canUndo: boolean;
  saving: boolean;
  error: string | null;
  onOpenLibrary: () => void;
  onSave: () => void;
  onRevert: () => void;
  onConvert: () => void;
  onAutonumber: () => void;
  onRename: () => void;
  onOpenSettings: () => void;
}

export function TitleBar({
  libraryPath,
  trackCount,
  filterText,
  onFilterChange,
  selectedFilePath,
  dirtyCount,
  canUndo,
  saving,
  error,
  onOpenLibrary,
  onSave,
  onRevert,
  onConvert,
  onAutonumber,
  onRename,
  onOpenSettings,
}: TitleBarProps) {
  return (
    <div className="flex items-center h-10 pl-20 pr-4 bg-surface border-b border-gray-700/30 drag-region select-none gap-2">
      {/* App icon — inset left to clear macOS window controls */}
      <span className="text-accent-light text-lg leading-none no-drag">♪</span>

      {/* Open Library button */}
      <button
        onClick={onOpenLibrary}
        className="no-drag px-2.5 py-1 text-[10px] font-medium rounded bg-accent/20 text-accent-light hover:bg-accent/30 transition-colors whitespace-nowrap"
        title="Open music library (⌘O)"
      >
        📂 Open Library
      </button>

      {/* Library path */}
      {libraryPath && (
        <div className="flex items-center gap-1.5 text-[10px] text-text-muted truncate max-w-[200px] no-drag">
          <span className="truncate">{libraryPath}</span>
          <span className="text-text-muted/40">({trackCount})</span>
        </div>
      )}

      <div className="w-px h-5 bg-gray-700/30 no-drag" />

      {/* Filter */}
      <div className="flex items-center gap-1 flex-1 max-w-[200px] no-drag">
        <span className="text-text-muted text-[10px]">🔍</span>
        <input
          type="text"
          value={filterText}
          onChange={(e) => onFilterChange(e.target.value)}
          placeholder="Filter files..."
          className="flex-1 bg-surface/60 border border-gray-700/40 rounded px-2 py-1 text-[10px] text-text-primary placeholder-text-muted/50 outline-none focus:border-accent/40 transition-colors"
        />
      </div>

      {/* Action buttons */}
      <div className="flex items-center gap-1 no-drag">
        <ActionButton
          icon="💾"
          label={saving ? "Saving…" : "Save"}
          accent
          disabled={dirtyCount === 0 || saving}
          onClick={onSave}
        />
        <ActionButton
          icon="↩"
          label="Revert"
          disabled={!canUndo}
          onClick={onRevert}
        />
        <ActionButton icon="🔄" label="Convert" onClick={onConvert} />
        <ActionButton icon="#" label="Num" onClick={onAutonumber} />
        <ActionButton icon="✏" label="Rename" onClick={onRename} />
      </div>

      <div className="w-px h-5 bg-gray-700/30 no-drag" />

      {/* Settings */}
      <ActionButton icon="⚙️" label="" onClick={onOpenSettings} />

      {/* Stats */}
      <div className="flex items-center gap-2 text-[10px] text-text-muted ml-auto no-drag">
        {error ? (
          <span className="text-red-400">⚠ {error}</span>
        ) : (
          <>
            {dirtyCount > 0 && (
              <span className="text-yellow-400">{dirtyCount} unsaved</span>
            )}
            {selectedFilePath && dirtyCount === 0 ? (
              <span className="text-accent-light">1 selected</span>
            ) : (
              <span>{trackCount} file{trackCount !== 1 ? "s" : ""}</span>
            )}
          </>
        )}
      </div>
    </div>
  );
}

function ActionButton({
  icon,
  label,
  accent,
  disabled,
  onClick,
}: {
  icon: string;
  label: string;
  accent?: boolean;
  disabled?: boolean;
  onClick?: () => void;
}) {
  return (
    <button
      disabled={disabled}
      onClick={onClick}
      className={`flex items-center gap-1 px-1.5 py-1 text-[10px] font-medium rounded transition-colors ${
        disabled
          ? "text-text-muted/30 cursor-not-allowed"
          : accent
            ? "bg-accent/25 text-accent-light hover:bg-accent/35"
            : "text-text-muted hover:bg-surface-hover hover:text-text-secondary"
      }`}
    >
      <span>{icon}</span>
      {label && <span>{label}</span>}
    </button>
  );
}
