import React from "react";
import type {
  AppUpdateInfo,
  AppUpdateProgress,
} from "../shared/desktop-api";

interface UpdateDialogProps {
  update: AppUpdateInfo | null;
  busy: boolean;
  installing: boolean;
  progress: AppUpdateProgress | null;
  error: string | null;
  onLater: () => void;
  onInstall: () => void;
}

function releaseDate(value: string | null): string | null {
  if (!value) return null;
  const date = new Date(value);
  return Number.isNaN(date.getTime())
    ? value
    : new Intl.DateTimeFormat(undefined, { dateStyle: "medium" }).format(date);
}

export function UpdateDialog({
  update,
  busy,
  installing,
  progress,
  error,
  onLater,
  onInstall,
}: UpdateDialogProps) {
  if (!update) return null;

  const date = releaseDate(update.date);
  const determinate = progress?.total != null && progress.total > 0;
  const percent = determinate
    ? Math.min(100, Math.round((progress.downloaded / progress.total!) * 100))
    : null;

  return (
    <div className="fixed inset-0 z-[60] flex items-center justify-center bg-black/40 backdrop-blur-sm">
      <div
        role="dialog"
        aria-modal="true"
        aria-labelledby="update-title"
        className="w-[440px] max-w-[calc(100vw-32px)] rounded-xl border border-border bg-white p-5 text-text-primary shadow-xl shadow-black/20"
      >
        <h2 id="update-title" className="text-[15px] font-semibold">
          Soundrobe {update.availableVersion} is available
        </h2>
        <p className="mt-1 text-[11px] text-text-muted">
          You have {update.currentVersion}{date ? ` · Released ${date}` : ""}
        </p>

        <div className="mt-4 max-h-48 overflow-y-auto whitespace-pre-wrap rounded-lg bg-surface-alt/50 p-3 text-[12px] leading-relaxed text-text-secondary">
          {update.notes || "No release notes were provided."}
        </div>

        {installing && progress && (
          <div className="mt-4">
            <div className="mb-1.5 flex justify-between text-[11px] text-text-muted">
              <span>
                {progress.phase === "installing"
                  ? "Installing update…"
                  : "Downloading update…"}
              </span>
              {percent != null && <span>{percent}%</span>}
            </div>
            <progress
              className="h-2 w-full accent-accent"
              value={determinate ? progress.downloaded : undefined}
              max={determinate ? progress.total! : undefined}
            />
          </div>
        )}

        {busy && !installing && (
          <p className="mt-4 text-[11px] text-amber-700">
            Finish the current disk operation before downloading and installing
            the update.
          </p>
        )}
        {error && (
          <p className="mt-4 rounded-lg bg-red-50 px-3 py-2 text-[11px] text-red-700">
            {error}
          </p>
        )}

        <div className="mt-5 flex justify-end gap-2">
          {!installing && (
            <button
              type="button"
              onClick={onLater}
              className="rounded-lg px-4 py-2 text-[11.5px] font-medium text-text-secondary hover:bg-surface-hover"
            >
              Later
            </button>
          )}
          <button
            type="button"
            disabled={busy || installing}
            onClick={onInstall}
            className={`rounded-lg px-4 py-2 text-[11.5px] font-medium ${
              busy || installing
                ? "cursor-not-allowed bg-accent/20 text-accent/60"
                : "bg-accent text-white hover:bg-accent/90"
            }`}
          >
            {installing
              ? "Installing…"
              : error
                ? "Retry download and restart"
                : "Download and restart"}
          </button>
        </div>
      </div>
    </div>
  );
}
