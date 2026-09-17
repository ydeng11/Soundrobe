import React from "react";
import type { LibraryRoot } from "../shared/desktop-api";

interface WebLibraryPickerProps {
  roots: LibraryRoot[];
  loading?: boolean;
  error?: string | null;
  onSelect: (path: string) => void;
  onLogout?: () => void;
}

export function WebLibraryPicker({
  roots,
  loading = false,
  error = null,
  onSelect,
  onLogout,
}: WebLibraryPickerProps) {
  return (
    <section
      aria-labelledby="web-library-picker-title"
      className="w-full rounded-xl border border-border bg-white p-5 shadow-sm sm:p-6"
    >
      <div className="flex items-start justify-between gap-4">
        <div>
          <h2 id="web-library-picker-title" className="text-lg font-semibold">
            Choose a library
          </h2>
          <p className="mt-1 text-sm text-text-muted">
            Select one of the mounted music libraries to open.
          </p>
        </div>
        {onLogout && (
          <button
            type="button"
            onClick={onLogout}
            className="text-sm text-text-muted hover:text-text-primary"
          >
            Sign out
          </button>
        )}
      </div>
      {loading && (
        <p className="mt-5 text-sm text-text-muted" role="status" aria-live="polite">
          Loading libraries…
        </p>
      )}
      {!loading && error && (
        <p className="mt-5 text-sm text-red-600" role="alert">
          {error}
        </p>
      )}
      {!loading && !error && roots.length === 0 && (
        <p className="mt-5 text-sm text-text-muted">
          No libraries are mounted. Add a library under the service&apos;s configured root.
        </p>
      )}
      {!loading && !error && roots.length > 0 && (
        <div className="mt-5 flex flex-col gap-2">
          {roots.map((root) => (
            <button
              key={root.id}
              type="button"
              onClick={() => onSelect(root.path)}
              className="flex min-h-12 items-center justify-between rounded-lg border border-border px-3 py-3 text-left hover:border-accent hover:bg-surface-hover focus:outline-none focus:ring-2 focus:ring-accent/40"
            >
              <span className="font-medium">{root.name}</span>
              <span className="ml-4 truncate text-sm text-text-muted">{root.path}</span>
            </button>
          ))}
        </div>
      )}
    </section>
  );
}
