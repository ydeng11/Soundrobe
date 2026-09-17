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
      className="w-full rounded-2xl border border-[#ded5c8] bg-[#fffdf9] p-5 shadow-[0_18px_50px_rgba(65,48,32,0.08)] sm:p-7"
    >
      <div className="flex items-start justify-between gap-4">
        <div>
          <h2 id="web-library-picker-title" className="font-serif text-2xl font-semibold text-[#28231d]">
            Choose a library
          </h2>
          <p className="mt-2 text-sm text-[#71675d]">
            Select one of the mounted music libraries to open.
          </p>
        </div>
        {onLogout && (
          <button
            type="button"
            onClick={onLogout}
            className="rounded-full border border-[#d9cdbd] px-3 py-1.5 text-sm font-medium text-[#544a40] hover:bg-[#eee6da]"
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
        <div className="mt-6 flex flex-col gap-3">
          {roots.map((root) => (
            <button
              key={root.id}
              type="button"
              onClick={() => onSelect(root.path)}
              className="flex min-h-14 items-center justify-between rounded-xl border border-[#e0d6ca] bg-[#faf7f1] px-4 py-3 text-left transition-colors hover:border-[#b66a47] hover:bg-[#f5ece0] focus:outline-none focus:ring-2 focus:ring-[#a85732]/40"
            >
              <span className="font-medium text-[#332c25]">{root.name}</span>
              <span className="ml-4 truncate text-sm text-[#8a7e71]">{root.path}</span>
            </button>
          ))}
        </div>
      )}
    </section>
  );
}
