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
  const dialogRef = React.useRef<HTMLDivElement | null>(null);
  const titleId = `${testId}-title`;

  /**
   * A modal that never takes focus is a modal a keyboard user is outside of.
   *
   * Focus moves to the first control when the drawer opens and returns to
   * whatever opened it when the drawer closes, so a keyboard user ends up where
   * they were rather than at the top of the document.
   */
  React.useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    const dialog = dialogRef.current;
    const first = dialog?.querySelector<HTMLElement>(
      'input:not([type="hidden"]):not([disabled]), select:not([disabled]), textarea:not([disabled]), button:not([disabled])',
    );
    (first ?? dialog)?.focus();
    return () => {
      previous?.focus?.();
    };
  }, []);

  const handleKeyDown = (event: React.KeyboardEvent<HTMLDivElement>) => {
    if (event.key === "Escape") {
      onClose();
      return;
    }
    if (event.key !== "Tab") return;
    // Tab stays inside: the backdrop hides the page, so tabbing past the last
    // control would move focus to something the reader cannot see.
    const items = Array.from(
      dialogRef.current?.querySelectorAll<HTMLElement>(
        'a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
      ) ?? [],
    );
    if (items.length === 0) return;
    const first = items[0];
    const last = items[items.length - 1];
    const active = document.activeElement as HTMLElement | null;
    if (!event.shiftKey && active === last) {
      event.preventDefault();
      first.focus();
    } else if (event.shiftKey && active === first) {
      event.preventDefault();
      last.focus();
    }
  };

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
        ref={dialogRef}
        role="dialog"
        aria-modal="true"
        aria-labelledby={titleId}
        aria-busy={busy}
        data-testid={testId}
        tabIndex={-1}
        className="sr-drawer"
        onKeyDown={handleKeyDown}
      >
        <header className="sr-toolbar">
          <h2 className="sr-section__title" id={titleId}>
            {title}
          </h2>
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
