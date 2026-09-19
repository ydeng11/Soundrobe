/**
 * Shared chrome for the workspace tool drawers.
 *
 * The three tools present identically and share their busy/error/outcome
 * bookkeeping, so that lives here rather than in one of them: a tool importing
 * the others' module would close an import cycle.
 */

import React from "react";

/** Shared drawer chrome, so the three tools present identically. */
export function ToolDrawer({
  title,
  busy,
  error,
  errors,
  onClose,
  children,
  testId,
}: {
  title: string;
  busy: boolean;
  error: string | null;
  /** Per-file failures, which the writers report alongside successes. */
  errors?: string[];
  onClose: () => void;
  children: React.ReactNode;
  testId: string;
}) {
  return (
    // The backdrop is the positioning container and the drawer is its child,
    // matching WorkspaceSettingsDrawer: a sibling drawer would sit under the
    // fixed backdrop and could not be clicked.
    <div
      className="sr-drawer__backdrop"
      data-testid={`${testId}-backdrop`}
      onMouseDown={(event) => {
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <div
        role="dialog"
        aria-modal="true"
        aria-label={title}
        aria-busy={busy}
        data-testid={testId}
        tabIndex={-1}
        className="sr-drawer"
        onKeyDown={(event) => {
          if (event.key === "Escape") onClose();
        }}
      >
        <header className="sr-toolbar">
          <h2 className="sr-section__title">{title}</h2>
          <div className="sr-header__spacer" />
          <button type="button" className="sr-btn" onClick={onClose}>
            Close
          </button>
        </header>
        {error && (
          <p className="sr-banner sr-banner--error" role="alert" data-testid={`${testId}-error`}>
            {error}
          </p>
        )}
        {errors && errors.length > 0 && (
          <div className="sr-banner sr-banner--error" role="alert" data-testid={`${testId}-failures`}>
            <ul className="sr-failure-list">
              {errors.map((message) => (
                <li key={message}>{message}</li>
              ))}
            </ul>
          </div>
        )}
        {children}
      </div>
    </div>
  );
}

/** Busy / error / outcome bookkeeping shared by the three tools. */
export function useToolRun() {
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [failures, setFailures] = React.useState<string[]>([]);
  const [message, setMessage] = React.useState<string | null>(null);

  /**
   * Run a tool step. A step returns its outcome line, optionally with the
   * per-file failures the writers report beside their successes, or nothing at
   * all when it only opens the next step.
   */
  const run = React.useCallback(
    async (
      work: () => Promise<
        string | { message: string; failures?: string[] } | null | undefined
      >,
    ) => {
      setBusy(true);
      setError(null);
      setMessage(null);
      setFailures([]);
      try {
        const outcome = await work();
        if (typeof outcome === "string" || outcome === null || outcome === undefined) {
          setMessage(typeof outcome === "string" ? outcome : null);
        } else {
          setMessage(outcome.message);
          setFailures(outcome.failures ?? []);
        }
      } catch (reason) {
        // A write path must surface the backend's reason, not swallow it.
        setError(reason instanceof Error ? reason.message : String(reason));
      } finally {
        setBusy(false);
      }
    },
    [],
  );

  return { busy, error, failures, message, run };
}

/**
 * The desktop reports the first few failures and how many it truncated; the
 * count matters because a silently clipped list reads as a clean run.
 */
export function summarizeFailures(prefix: string, failures: string[]): string[] {
  const shown = failures.slice(0, 3);
  const extra = failures.length - shown.length;
  return shown.map((line) => `${prefix}${line}`).concat(
    extra > 0 ? [`…and ${extra} more`] : [],
  );
}
