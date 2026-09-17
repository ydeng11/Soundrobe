import React, { useEffect, useState } from "react";

type Task = {
  taskId: string;
  status: "running" | "completed" | "needs_review" | "failed" | "cancelled";
  message: string;
  result: unknown;
};

const statusTone: Record<Task["status"], string> = {
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
  const [tasks, setTasks] = useState<Task[]>([]);

  useEffect(() => {
    if (!open) return;
    let active = true;
    const refresh = async () => {
      const response = await fetch("/api/v1/tasks", { credentials: "same-origin" });
      if (response.ok && active) setTasks(await response.json());
    };
    void refresh().catch(() => undefined);
    const interval = window.setInterval(() => void refresh().catch(() => undefined), 5_000);
    return () => { active = false; window.clearInterval(interval); };
  }, [open]);

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
          <ol className="mt-3 max-h-72 space-y-2 overflow-auto">
            {tasks.length === 0 && (
              <li className="text-sm text-text-muted">No background work has been recorded.</li>
            )}
            {tasks.slice().reverse().map((task) => (
              <li key={task.taskId} className="rounded-lg border border-border px-3 py-2">
                <div className="flex items-center justify-between gap-2">
                  <span className={`rounded-full border px-2 py-0.5 text-xs ${statusTone[task.status]}`}>
                    {task.status.replace("_", " ")}
                  </span>
                  <span className="truncate text-xs text-text-muted">{task.taskId}</span>
                </div>
                <p className="mt-1 text-sm text-text-primary">{task.message}</p>
                {formatTaskResult(task.result) && (
                  <details className="mt-2">
                    <summary className="cursor-pointer text-xs text-text-muted">Details</summary>
                    <pre className="mt-1 overflow-auto rounded bg-surface-alt p-2 text-[11px] text-text-secondary">
                      {formatTaskResult(task.result)}
                    </pre>
                  </details>
                )}
              </li>
            ))}
          </ol>
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
