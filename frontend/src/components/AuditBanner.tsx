import React from "react";

interface AuditTrackResult {
  trackIndex: number;
  field: string;
  status: "correct" | "warning" | "error";
  message: string | null;
  suggestion: string | null;
}

interface AuditBannerProps {
  results: Record<string, AuditTrackResult[]>;
  onDismiss: () => void;
}

export function AuditBanner({ results, onDismiss }: AuditBannerProps) {
  const entries = Object.entries(results);
  if (entries.length === 0) return null;

  const totalIssues = entries.reduce((sum, [, r]) => sum + r.length, 0);
  const albumCount = entries.length;

  return (
    <div className="flex items-center gap-3 px-4 py-2 bg-[#ff9f0a]/10 border-b border-[#ff9f0a]/20 text-[12px]">
      <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="#ff9f0a" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round" className="shrink-0">
        <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" />
      </svg>
      <span className="text-text-primary font-medium">
        Audit complete
      </span>
      <span className="text-text-muted">
        {totalIssues > 0
          ? `${totalIssues} issue(s) across ${albumCount} album(s)`
          : "No issues found — all metadata matches file paths"}
      </span>
      <div className="flex-1" />
      <button
        onClick={onDismiss}
        className="text-text-muted hover:text-text-primary transition-colors"
        title="Dismiss"
      >
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
          <path d="M18 6 6 18" /><path d="m6 6 12 12" />
        </svg>
      </button>
    </div>
  );
}
