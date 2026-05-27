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
      <div className="h-[3px] w-full bg-surface-alt overflow-hidden">
        <div
          className={`h-full bg-accent ${
            determinate
              ? "transition-all duration-200 ease-out"
              : "animate-pulse"
          }`}
          style={
            determinate
              ? { width: `${pct}%`, minWidth: "4px" }
              : { width: "30%" }
          }
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
