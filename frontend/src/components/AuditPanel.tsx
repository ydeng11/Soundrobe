import React from "react";

interface AuditTrackResult {
  trackIndex: number;
  field: string;
  status: "correct" | "warning" | "error";
  message: string | null;
  suggestion: string | null;
}

interface AuditPanelProps {
  results: AuditTrackResult[];
  albumName: string;
}

const STATUS_COLORS: Record<string, { bg: string; text: string; dot: string; label: string }> = {
  error: { bg: "bg-red-500/10", text: "text-red-600", dot: "bg-red-500", label: "Error" },
  warning: { bg: "bg-[#ff9f0a]/10", text: "text-[#ff9f0a]", dot: "bg-[#ff9f0a]", label: "Warning" },
  correct: { bg: "bg-green-500/10", text: "text-green-600", dot: "bg-green-500", label: "OK" },
};

export function AuditPanel({ results, albumName }: AuditPanelProps) {
  if (results.length === 0) return null;

  return (
    <div className="flex flex-col h-full">
      <div className="px-4 py-3 border-b border-border">
        <div className="text-[13px] font-medium text-text-primary flex items-center gap-2">
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="#ff9f0a" strokeWidth="1.5" strokeLinecap="round" strokeLinejoin="round">
            <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z" />
          </svg>
          Audit Results
        </div>
        <div className="text-[11px] text-text-muted mt-0.5">
          {results.length} issue(s) in {albumName}
        </div>
      </div>

      <div className="flex-1 overflow-y-auto">
        {results.map((r, i) => {
          const colors = STATUS_COLORS[r.status] ?? STATUS_COLORS.warning;
          return (
            <div
              key={i}
              className={`px-4 py-2.5 border-b border-border/50 ${colors.bg}`}
            >
              <div className="flex items-center gap-2">
                <span className={`w-2 h-2 rounded-full shrink-0 ${colors.dot}`} />
                <span className={`text-[11px] font-medium ${colors.text}`}>{colors.label}</span>
                {r.trackIndex !== undefined && (
                  <span className="text-[11px] text-text-muted">
                    Track {r.trackIndex + 1}
                  </span>
                )}
              </div>
              <div className="mt-1 text-[11.5px] text-text-primary leading-relaxed">
                {r.message ?? `"${r.field}" field issue`}
              </div>
              {r.suggestion && (
                <div className="mt-0.5 text-[10.5px] text-text-muted italic leading-relaxed">
                  Suggestion: {r.suggestion}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}
