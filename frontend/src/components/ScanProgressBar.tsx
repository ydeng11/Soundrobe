import React from "react";

interface ScanProgressBarProps {
  scanning: boolean;
  progress: { current: number; total: number } | null;
  label?: string | null;
}

export function ScanProgressBar({ scanning, progress, label }: ScanProgressBarProps) {
  if (!scanning) return null;

  const determinate = progress !== null && progress.total > 0;
  const pct = determinate
    ? Math.round((progress.current / progress.total) * 100)
    : 0;

  return (
    <div className="flex flex-col">
      <div className="h-[3px] w-full bg-surface-alt overflow-hidden relative">
        <div
          className={`h-full bg-accent ${
            determinate
              ? "transition-all duration-300 ease-out"
              : "animate-progress-indeterminate"
          }`}
          style={
            determinate
              ? { width: `${pct}%`, minWidth: "4px" }
              : { width: "40%" }
          }
        />
        {/* Shimmer overlay */}
        <div
          className={`absolute inset-0 ${
            determinate
              ? "bg-gradient-to-r from-transparent via-white/20 to-transparent animate-shimmer"
              : "bg-gradient-to-r from-transparent via-white/20 to-transparent animate-shimmer-fast"
          }`}
          style={{ backgroundSize: "200% 100%" }}
        />
      </div>
      {label && (
        <div className="px-3 py-0.5 text-[10px] text-text-muted italic">
          {label}
        </div>
      )}
    </div>
  );
}
