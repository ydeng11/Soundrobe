import type {
  AutoTagBatchItem,
  AutoTagBatchItemStatus,
  AutoTagBatchSummary,
} from "../state/auto-tag-batch";

interface AutoTagSummaryDialogProps {
  summary: AutoTagBatchSummary | null;
  onClose: () => void;
}

const statusLabel: Record<AutoTagBatchItemStatus, string> = {
  applied: "Applied",
  recovered: "Recovered on retry",
  needs_review: "Needs review",
  failed: "Failed",
  cancelled: "Cancelled",
};

const statusClass: Record<AutoTagBatchItemStatus, string> = {
  applied: "text-green-700 bg-green-50 border-green-200",
  recovered: "text-blue-700 bg-blue-50 border-blue-200",
  needs_review: "text-amber-700 bg-amber-50 border-amber-200",
  failed: "text-red-700 bg-red-50 border-red-200",
  cancelled: "text-text-muted bg-surface-hover border-border/60",
};

function countStatus(summary: AutoTagBatchSummary, status: AutoTagBatchItemStatus) {
  return summary.items.filter((item) => item.status === status).length;
}

function itemDetail(item: AutoTagBatchItem): string {
  const providerDiagnostic = item.providerAttempts
    .map((attempt) => attempt.diagnostic)
    .find((diagnostic): diagnostic is string => Boolean(diagnostic));
  const retryCount = item.providerAttempts.reduce(
    (total, attempt) => total + (attempt.retryCount ?? 0),
    0,
  );
  const retryAfterSeconds = item.providerAttempts.reduce(
    (maximum, attempt) =>
      Math.max(maximum, attempt.retryAfterSeconds ?? 0),
    0,
  );
  const details = [providerDiagnostic ?? item.message];
  if (retryCount > 0) {
    details.push(`${retryCount} provider retr${retryCount === 1 ? "y" : "ies"}`);
  }
  if (retryAfterSeconds > 0) {
    details.push(`max Retry-After ${retryAfterSeconds}s`);
  }
  if (item.attempts > 1) details.push(`${item.attempts} batch attempts`);
  return details.join(" · ");
}

export function AutoTagSummaryDialog({
  summary,
  onClose,
}: AutoTagSummaryDialogProps) {
  if (!summary) return null;

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
        aria-modal="true"
        aria-labelledby="auto-tag-summary-title"
        className="bg-white rounded-xl shadow-xl border border-border/60 w-[min(680px,calc(100vw-2rem))] max-h-[80vh] overflow-hidden"
        onClick={(event) => event.stopPropagation()}
      >
        <div className="flex items-center justify-between px-4 py-3 border-b border-border/50">
          <div>
            <h2
              id="auto-tag-summary-title"
              className="text-[13px] font-semibold text-text-primary"
            >
              Auto-tag summary
            </h2>
            <p className="mt-0.5 text-[11px] text-text-muted">
              {summary.items.length} album{summary.items.length === 1 ? "" : "s"} processed
            </p>
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

        <div className="flex flex-wrap gap-1.5 px-4 py-3 border-b border-border/50 text-[10px]">
          {(Object.keys(statusLabel) as AutoTagBatchItemStatus[]).map((status) => (
            <span
              key={status}
              className={`px-1.5 py-0.5 rounded-md border ${statusClass[status]}`}
            >
              {statusLabel[status]}: {countStatus(summary, status)}
            </span>
          ))}
        </div>

        <div className="overflow-y-auto max-h-[55vh] p-3 space-y-1.5">
          {summary.items.map((item) => (
            <div
              key={item.albumPath}
              className="flex items-start gap-3 rounded-lg border border-border/50 px-3 py-2"
            >
              <div className="min-w-0 flex-1">
                <div className="truncate text-[12px] font-medium text-text-primary">
                  {item.albumPath}
                </div>
                <div className="mt-0.5 text-[11px] text-text-muted">
                  {itemDetail(item)}
                </div>
              </div>
              <span
                className={`shrink-0 px-1.5 py-0.5 rounded-md border text-[10px] ${statusClass[item.status]}`}
              >
                {statusLabel[item.status]}
              </span>
            </div>
          ))}
        </div>

        <div className="flex justify-end px-4 py-3 border-t border-border/50">
          <button
            type="button"
            onClick={onClose}
            className="px-3 py-1.5 rounded-md bg-accent text-white text-[11px] font-medium hover:bg-accent/90"
          >
            Done
          </button>
        </div>
      </div>
    </div>
  );
}
