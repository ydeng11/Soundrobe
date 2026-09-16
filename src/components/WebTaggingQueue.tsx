import React, { useEffect, useState } from "react";

type Task = {
  taskId: string;
  status: "running" | "completed" | "needs_review" | "failed" | "cancelled";
  message: string;
  result: unknown;
};

const label: Record<Task["status"], string> = {
  running: "Tagging",
  completed: "Tagged",
  needs_review: "Needs review",
  failed: "Failed",
  cancelled: "Cancelled",
};

const tone: Record<Task["status"], string> = {
  running: "bg-blue-50 text-blue-700 border-blue-200",
  completed: "bg-emerald-50 text-emerald-700 border-emerald-200",
  needs_review: "bg-amber-50 text-amber-800 border-amber-200",
  failed: "bg-red-50 text-red-700 border-red-200",
  cancelled: "bg-slate-50 text-slate-600 border-slate-200",
};

/** Service-only queue: background work remains visible even when it was not
 * started from this browser tab. */
export function WebTaggingQueue() {
  const [tasks, setTasks] = useState<Task[]>([]);

  useEffect(() => {
    let active = true;
    const refresh = async () => {
      try {
        const response = await fetch("/api/v1/tasks", { credentials: "same-origin" });
        if (response.ok && active) setTasks(await response.json());
      } catch {
        // The primary app remains usable while the service is restarting.
      }
    };
    void refresh();
    const timer = window.setInterval(() => void refresh(), 5_000);
    return () => { active = false; window.clearInterval(timer); };
  }, []);

  const review = tasks.filter((task) => task.status === "needs_review" || task.status === "failed");
  const running = tasks.filter((task) => task.status === "running");
  if (!review.length && !running.length) return null;

  return (
    <section className="border-b border-border bg-white px-5 py-3" aria-label="Background tagging queue">
      <div className="mx-auto flex max-w-screen-2xl items-start gap-3">
        <div className="min-w-0 flex-1">
          <p className="text-sm font-semibold text-text-primary">Library tagging queue</p>
          <p className="mt-0.5 text-xs text-text-muted">
            {running.length ? `${running.length} release${running.length === 1 ? "" : "s"} processing automatically.` : "Automatic scan is complete."}
          </p>
          {review.length > 0 && (
            <ul className="mt-2 flex flex-wrap gap-2">
              {review.map((task) => (
                <li key={task.taskId} className={`rounded-full border px-2.5 py-1 text-xs ${tone[task.status]}`} title={task.message}>
                  {label[task.status]}: {task.message}
                </li>
              ))}
            </ul>
          )}
        </div>
        {running.map((task) => (
          <span key={task.taskId} className={`shrink-0 rounded-full border px-2.5 py-1 text-xs ${tone.running}`}>
            {label.running}
          </span>
        ))}
      </div>
    </section>
  );
}
