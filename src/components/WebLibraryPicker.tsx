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
    <main className="flex min-h-screen items-center justify-center bg-surface px-4 text-text-primary">
      <section className="w-full max-w-lg rounded-xl border border-border bg-white p-6 shadow-sm">
        <div className="flex items-start justify-between gap-4">
          <div>
            <h1 className="text-lg font-semibold">Choose a library</h1>
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
        {loading && <p className="mt-5 text-sm text-text-muted">Loading libraries…</p>}
        {error && <p className="mt-5 text-sm text-red-600" role="alert">{error}</p>}
        {!loading && !error && roots.length === 0 && (
          <p className="mt-5 text-sm text-text-muted">
            No libraries are mounted. Add a library under the service&apos;s configured root.
          </p>
        )}
        <div className="mt-5 flex flex-col gap-2">
          {roots.map((root) => (
            <button
              key={root.id}
              type="button"
              onClick={() => onSelect(root.path)}
              className="flex items-center justify-between rounded-md border border-border px-3 py-3 text-left hover:border-accent hover:bg-surface-hover"
            >
              <span className="font-medium">{root.name}</span>
              <span className="ml-4 truncate text-sm text-text-muted">{root.path}</span>
            </button>
          ))}
        </div>
      </section>
    </main>
  );
}
