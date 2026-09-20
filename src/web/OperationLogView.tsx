/**
 * Operation log for the browser runtime.
 *
 * The reference's `操作记录` page lists tagging operations. Soundrobe already
 * tracks browser operations as server-side tasks, so this view renders that
 * real task state rather than a parallel log format.
 *
 * The reference's exact column set (ID / 用户 / 操作 / 等级 / 状态 / 详情摘要 /
 * 时间 / 操作) does not map onto Soundrobe's task model; the mapping used here
 * is recorded as a deliberate difference in the parity matrix.
 */

import React from "react";
import { listWebTasks, type WebTask, type WebTaskStatus } from "../shared/web-adapter";

export interface OperationLogViewProps {
  /** Injected for tests. */
  loadTasks?: () => Promise<WebTask[]>;
  cancelTask?: (taskId: string) => Promise<unknown>;
}

const STATUS_LABEL: Record<WebTaskStatus, string> = {
  running: "Running",
  completed: "Completed",
  needs_review: "Needs review",
  failed: "Failed",
  cancelled: "Cancelled",
};

const STATUS_CLASS: Record<WebTaskStatus, string> = {
  running: "sr-badge--info",
  completed: "sr-badge--success",
  needs_review: "sr-badge--warning",
  failed: "sr-badge--danger",
  cancelled: "sr-badge--muted",
};

export function OperationLogView({
  loadTasks = listWebTasks,
  cancelTask,
}: OperationLogViewProps) {
  const [tasks, setTasks] = React.useState<WebTask[]>([]);
  const [loading, setLoading] = React.useState(true);
  const [error, setError] = React.useState<string | null>(null);

  const refresh = React.useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setTasks(await loadTasks());
    } catch (reason) {
      setError(reason instanceof Error ? reason.message : String(reason));
    } finally {
      setLoading(false);
    }
  }, [loadTasks]);

  React.useEffect(() => {
    void refresh();
  }, [refresh]);

  const running = tasks.filter((task) => task.status === "running").length;
  const needsReview = tasks.filter((task) => task.status === "needs_review").length;
  const failed = tasks.filter((task) => task.status === "failed").length;

  return (
    <section className="sr-section" data-testid="operation-log">
      <div className="sr-toolbar">
        <h3 className="sr-section__title">Operations</h3>
        <div className="sr-header__spacer" />
        <button type="button" className="sr-btn" onClick={() => void refresh()}>
          Refresh
        </button>
      </div>

      <div className="sr-summary" data-testid="log-summary">
        <Summary label="Total" value={tasks.length} />
        <Summary label="Running" value={running} />
        <Summary label="Needs review" value={needsReview} />
        <Summary label="Failed" value={failed} />
      </div>

      {error && (
        <p className="sr-banner sr-banner--error" role="alert">
          {error}
        </p>
      )}

      {loading && tasks.length === 0 ? (
        <div className="sr-empty">
          <p className="sr-empty__body">Loading operations…</p>
        </div>
      ) : tasks.length === 0 ? (
        <div className="sr-empty" data-testid="log-empty">
          <h3 className="sr-empty__title">No operations yet</h3>
          <p className="sr-empty__body">
            Tagging runs started from the workspace appear here.
          </p>
        </div>
      ) : (
        <div className="sr-table-wrap">
          <table className="sr-table sr-table--log" data-testid="log-table">
            <thead>
              <tr>
                <th scope="col">Task</th>
                <th scope="col">Status</th>
                <th scope="col">Progress</th>
                <th scope="col">Details</th>
                <th scope="col">Actions</th>
              </tr>
            </thead>
            <tbody>
              {tasks.map((task) => (
                <tr key={task.taskId}>
                  <td>{task.taskId}</td>
                  <td>
                    <span className={`sr-badge ${STATUS_CLASS[task.status]}`}>
                      {STATUS_LABEL[task.status]}
                    </span>
                  </td>
                  <td>
                    {task.total > 0 ? `${task.progress} / ${task.total}` : "—"}
                  </td>
                  <td className="sr-cell--muted">
                    <div>{task.message || "—"}</div>
                    <TagCleanupSummary result={task.result} />
                  </td>
                  <td>
                    {task.status === "running" && cancelTask ? (
                      <button
                        type="button"
                        className="sr-btn"
                        onClick={() => {
                          void cancelTask(task.taskId).then(refresh);
                        }}
                      >
                        Cancel
                      </button>
                    ) : (
                      "—"
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}
    </section>
  );
}

function Summary({ label, value }: { label: string; value: number }) {
  return (
    <span className="sr-summary__card">
      <span className="sr-summary__value">{value}</span>
      <span className="sr-summary__label">{label}</span>
    </span>
  );
}

function TagCleanupSummary({ result }: { result: unknown }) {
  if (!result || typeof result !== "object") return null;
  const value = result as { keptTags?: unknown; clearedTags?: unknown };
  const keptTags = Array.isArray(value.keptTags)
    ? value.keptTags.filter((tag): tag is string => typeof tag === "string")
    : [];
  const clearedTags = Array.isArray(value.clearedTags)
    ? value.clearedTags.filter((tag): tag is string => typeof tag === "string")
    : [];
  if (keptTags.length === 0 && clearedTags.length === 0) return null;
  return (
    <div data-testid="operation-tag-summary" className="sr-field__static">
      <div>Tags kept: {keptTags.length > 0 ? keptTags.join(", ") : "none"}</div>
      {clearedTags.length > 0 && <div>Tags cleared: {clearedTags.join(", ")}</div>}
    </div>
  );
}
