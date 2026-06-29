import React from "react";

type CorrectedAuditFields = {
  title?: string | null;
  artist?: string | null;
  artists?: string[] | null;
  album?: string | null;
  albumArtist?: string | null;
  album_artist?: string | null;
  albumArtists?: string[] | null;
  year?: string | null;
  genre?: string | null;
  trackNumber?: number | null;
  trackTotal?: number | null;
  discNumber?: number | null;
  discTotal?: number | null;
};

interface AuditTrackResult {
  trackIndex: number;
  field: string;
  status: "correct" | "warning" | "error";
  message: string | null;
  suggestion: string | null;
  source?: "deterministic" | "llm";
  confidence?: number;
  autoFixEligible?: boolean;
  autoFixed?: boolean;
  corrected?: CorrectedAuditFields | null;
}

interface AuditPanelProps {
  results: AuditTrackResult[];
  albumName: string;
  onApplyFixes?: () => void;
  applying?: boolean;
}

const STATUS_COLORS: Record<string, { bg: string; text: string; dot: string; label: string }> = {
  error: { bg: "bg-red-500/10", text: "text-red-600", dot: "bg-red-500", label: "Error" },
  warning: { bg: "bg-[#ff9f0a]/10", text: "text-[#ff9f0a]", dot: "bg-[#ff9f0a]", label: "Warning" },
  correct: { bg: "bg-green-500/10", text: "text-green-600", dot: "bg-green-500", label: "OK" },
};

interface TrackAuditGroup {
  trackIndex: number;
  results: AuditTrackResult[];
  fixable: AuditTrackResult[];
  manual: AuditTrackResult[];
}

function groupByTrack(results: AuditTrackResult[]): TrackAuditGroup[] {
  const groups = new Map<number, AuditTrackResult[]>();
  for (const result of results) {
    const key = result.trackIndex ?? 0;
    groups.set(key, [...(groups.get(key) ?? []), result]);
  }
  return Array.from(groups.entries())
    .sort(([a], [b]) => a - b)
    .map(([trackIndex, trackResults]) => ({
      trackIndex,
      results: trackResults,
      fixable: trackResults.filter((result) => result.autoFixEligible && !result.autoFixed),
      manual: trackResults.filter((result) => (result.status === "error" || result.status === "warning") && !result.autoFixEligible && !result.autoFixed),
    }));
}

function fixableCount(results: AuditTrackResult[]): number {
  return results.filter((result) => result.autoFixEligible && !result.autoFixed).length;
}

function plannedValue(result: AuditTrackResult): string | null {
  const corrected = result.corrected as Record<string, string | number | string[] | null | undefined> | null | undefined;
  const value = result.field === "album_artist"
    ? corrected?.albumArtist ?? corrected?.album_artist
    : corrected?.[result.field];
  if (value === undefined) return result.autoFixEligible ? result.suggestion : null;
  if (value === null) return "(clear)";
  if (Array.isArray(value)) return value.join(", ");
  return String(value);
}

export function SelectedTrackAuditFindings({
  results,
  onApplyFixes,
  applying = false,
}: {
  results: AuditTrackResult[];
  onApplyFixes?: () => void;
  applying?: boolean;
}) {
  if (results.length === 0) return null;
  const fixable = fixableCount(results);

  return (
    <div className="border-b border-border bg-[#ff9f0a]/5 px-4 py-3">
      <div className="flex items-center gap-2">
        <div className="text-[12px] font-medium text-text-primary">
          Audit Findings
        </div>
        <div className="flex-1" />
        {onApplyFixes && fixable > 0 && (
          <button
            type="button"
            onClick={onApplyFixes}
            disabled={applying}
            className="rounded border border-[#ff9f0a]/30 bg-[#ff9f0a]/10 px-2 py-1 text-[10.5px] font-medium text-[#b36200] disabled:opacity-50"
          >
            {applying ? "Applying..." : "Apply Audit Fixes"}
          </button>
        )}
      </div>
      {fixable > 0 && (
        <div className="mt-1 text-[10.5px] text-text-muted">
          Fix plan: {fixable} field(s) can be applied after approval.
        </div>
      )}
      <div className="mt-2 flex flex-col gap-2">
        {results.map((result, index) => {
          const colors = STATUS_COLORS[result.autoFixed ? "correct" : result.status] ?? STATUS_COLORS.warning;
          const stateLabel = result.autoFixed
            ? "Fixed"
            : result.autoFixEligible
              ? "Will fix"
              : colors.label;
          const planned = plannedValue(result);
          return (
            <div key={`${result.field}-${index}`} className="rounded border border-border/60 bg-white/70 px-2.5 py-2">
              <div className="flex items-center gap-2">
                <span className={`w-2 h-2 rounded-full shrink-0 ${colors.dot}`} />
                <span className={`text-[10.5px] font-medium ${colors.text}`}>
                  {stateLabel}
                </span>
                <span className="text-[10.5px] text-text-muted">{result.field}</span>
              </div>
              <div className="mt-1 text-[11px] leading-relaxed text-text-primary">
                {result.message ?? `"${result.field}" field issue`}
              </div>
              {result.suggestion && (
                <div className="mt-0.5 text-[10.5px] italic leading-relaxed text-text-muted">
                  Suggestion: {result.suggestion}
                </div>
              )}
              {result.autoFixEligible && !result.autoFixed && planned && (
                <div className="mt-0.5 text-[10.5px] leading-relaxed text-text-muted">
                  Will write: {planned}
                </div>
              )}
            </div>
          );
        })}
      </div>
    </div>
  );
}

export function AuditPanel({ results, albumName, onApplyFixes, applying = false }: AuditPanelProps) {
  if (results.length === 0) return null;
  const groups = groupByTrack(results);
  const totalFixable = fixableCount(results);
  const manualCount = results.filter((result) => (result.status === "error" || result.status === "warning") && !result.autoFixEligible && !result.autoFixed).length;

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
          {results.length} issue(s) across {groups.length} track(s) in {albumName}
        </div>
        <div className="mt-2 rounded border border-[#ff9f0a]/20 bg-[#ff9f0a]/5 px-2.5 py-2">
          <div className="text-[11px] font-medium text-text-primary">Fix Plan</div>
          <div className="mt-1 text-[10.5px] text-text-muted">
            {totalFixable} fixable field(s), {manualCount} manual-review field(s)
          </div>
          {onApplyFixes && totalFixable > 0 && (
            <button
              type="button"
              onClick={onApplyFixes}
              disabled={applying}
              className="mt-2 rounded border border-[#ff9f0a]/30 bg-[#ff9f0a]/10 px-2 py-1 text-[10.5px] font-medium text-[#b36200] disabled:opacity-50"
            >
              {applying ? "Applying..." : "Apply Audit Fixes"}
            </button>
          )}
        </div>
      </div>

      <div className="flex-1 overflow-y-auto">
        {groups.map((group) => (
          <div key={group.trackIndex} className="border-b border-border/50">
            <div className="px-4 py-2 bg-surface-alt/60">
              <div className="text-[11px] font-medium text-text-primary">
                Track {group.trackIndex + 1}
              </div>
              <div className="mt-0.5 text-[10.5px] text-text-muted">
                {group.fixable.length} will fix, {group.manual.length} needs manual review
              </div>
            </div>
            {group.results.map((r, i) => {
              const colors = STATUS_COLORS[r.autoFixed ? "correct" : r.status] ?? STATUS_COLORS.warning;
              const stateLabel = r.autoFixed
                ? "Fixed"
                : r.autoFixEligible
                  ? "Will fix"
                  : colors.label;
              const planned = plannedValue(r);
              return (
                <div
                  key={`${r.field}-${i}`}
                  className={`px-4 py-2.5 border-b border-border/50 ${colors.bg}`}
                >
                  <div className="flex items-center gap-2">
                    <span className={`w-2 h-2 rounded-full shrink-0 ${colors.dot}`} />
                    <span className={`text-[11px] font-medium ${colors.text}`}>{stateLabel}</span>
                    <span className="text-[10.5px] text-text-muted">{r.field}</span>
                    {r.source && (
                      <span className="text-[10.5px] text-text-muted">
                        {r.source}
                        {typeof r.confidence === "number" ? ` ${Math.round(r.confidence * 100)}%` : ""}
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
                  {r.autoFixEligible && !r.autoFixed && planned && (
                    <div className="mt-0.5 text-[10.5px] text-text-muted leading-relaxed">
                      Will write: {planned}
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        ))}
      </div>
    </div>
  );
}
