import React, { useEffect, useRef, useState } from "react";
import { listWebTasks, type WebTask, type WebTaskStatus } from "../shared/web-adapter";

const statusTone: Record<WebTaskStatus, string> = {
  running: "text-blue-700 bg-blue-50 border-blue-200",
  completed: "text-emerald-700 bg-emerald-50 border-emerald-200",
  needs_review: "text-amber-800 bg-amber-50 border-amber-200",
  failed: "text-red-700 bg-red-50 border-red-200",
  cancelled: "text-slate-600 bg-slate-50 border-slate-200",
};

function formatTaskResult(result: unknown): string | null {
  if (result === null || result === undefined) return null;
  return typeof result === "string" ? result : JSON.stringify(result, null, 2);
}

/** Kept out of the primary editor so automatic tagging stays quiet. */
export function WebTaggingDebugPortal() {
  const [open, setOpen] = useState(false);
  const [tasks, setTasks] = useState<WebTask[]>([]);
  const [loadState, setLoadState] = useState<"idle" | "loading" | "ready" | "error">("idle");
  const [hasLoaded, setHasLoaded] = useState(false);
  const activeRef = useRef(false);

  useEffect(() => {
    if (!open) return;
    let active = true;
    activeRef.current = true;
    const refresh = async () => {
      try {
        const nextTasks = await listWebTasks();
        if (!active) return;
        setTasks(nextTasks);
        setHasLoaded(true);
        setLoadState("ready");
      } catch {
        if (!active) return;
        setLoadState("error");
      }
    };
    setLoadState("loading");
    void refresh();
    const interval = window.setInterval(() => void refresh(), 5_000);
    return () => {
      active = false;
      activeRef.current = false;
      window.clearInterval(interval);
    };
  }, [open]);

  const showFirstLoadError = loadState === "error" && !hasLoaded;
  const showStaleError = loadState === "error" && hasLoaded;
  const retry = () => {
    if (!activeRef.current) return;
    setLoadState("loading");
    void listWebTasks()
      .then((nextTasks) => {
        if (!activeRef.current) return;
        setTasks(nextTasks);
        setHasLoaded(true);
        setLoadState("ready");
      })
      .catch(() => {
        if (activeRef.current) setLoadState("error");
      });
  };

  return (
    <div className="fixed bottom-4 right-4 z-50">
      {open && (
        <section
          className="mb-2 w-[min(36rem,calc(100vw-2rem))] rounded-xl border border-border bg-white p-4 shadow-xl"
          aria-label="Background tagging diagnostics"
        >
          <div className="flex items-start justify-between gap-3">
            <div>
              <h2 className="text-sm font-semibold">Background tagging diagnostics</h2>
              <p className="mt-1 text-xs text-text-muted">
                Automatic activity and review outcomes for mounted libraries.
              </p>
            </div>
            <button
              type="button"
              onClick={() => setOpen(false)}
              className="text-xs text-text-muted hover:text-text-primary"
            >
              Close
            </button>
          </div>
          <div className="mt-3" aria-live="polite" aria-busy={loadState === "loading"}>
            {loadState === "loading" && !hasLoaded && (
              <p className="text-sm text-text-muted">Loading activity…</p>
            )}
            {loadState === "loading" && hasLoaded && (
              <p className="text-sm text-text-muted">Refreshing activity…</p>
            )}
            {showFirstLoadError && (
              <div className="flex items-center justify-between gap-3 text-sm text-red-700">
                <p>Unable to load activity.</p>
                <button
                  type="button"
                  onClick={retry}
                  className="rounded border border-red-200 px-2 py-1 text-xs font-medium hover:bg-red-50"
                >
                  Retry
                </button>
              </div>
            )}
            {showStaleError && (
              <div className="flex items-center justify-between gap-3 text-sm text-amber-800">
                <p>Refresh failed; showing last known activity.</p>
                <button
                  type="button"
                  onClick={retry}
                  className="rounded border border-amber-200 px-2 py-1 text-xs font-medium hover:bg-amber-50"
                >
                  Retry
                </button>
              </div>
            )}
            {loadState === "ready" && tasks.length === 0 && (
              <p className="text-sm text-text-muted">No background activity.</p>
            )}
            {tasks.length > 0 && (
              <ol className="max-h-72 space-y-2 overflow-auto">
                {tasks.map((task) => {
                  const formattedResult = formatTaskResult(task.result);
                  return (
                    <li key={task.taskId} className="rounded-lg border border-border px-3 py-2">
                      <div className="flex items-center justify-between gap-2">
                        <span className={`rounded-full border px-2 py-0.5 text-xs ${statusTone[task.status]}`}>
                          {task.status.replace("_", " ")}
                        </span>
                        <span className="truncate text-xs text-text-muted">{task.taskId}</span>
                      </div>
                      <p className="mt-1 text-sm text-text-primary">{task.message}</p>
                      {task.total > 0 && (
                        <p className="mt-1 text-xs text-text-muted">
                          Progress: {task.progress}/{task.total}
                        </p>
                      )}
                      {formattedResult && (
                        <details className="mt-2">
                          <summary className="cursor-pointer text-xs text-text-muted">Details</summary>
                          <pre className="mt-1 overflow-auto rounded bg-surface-alt p-2 text-[11px] text-text-secondary">
                            {formattedResult}
                          </pre>
                        </details>
                      )}
                    </li>
                  );
                })}
              </ol>
            )}
          </div>
        </section>
      )}
      <button
        type="button"
        onClick={() => setOpen((value) => !value)}
        className="rounded-full border border-border bg-white px-3 py-2 text-xs font-medium text-text-secondary shadow-lg hover:bg-surface-hover"
        aria-expanded={open}
      >
        Activity
      </button>
    </div>
  );
}
