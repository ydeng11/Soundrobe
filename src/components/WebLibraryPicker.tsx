import React from "react";
import type { LibraryRoot } from "../shared/desktop-api";

interface WebLibraryPickerProps {
  roots: LibraryRoot[];
  loading?: boolean;
  error?: string | null;
  onSelect: (path: string) => void;
}

export function WebLibraryPicker({
  roots,
  loading = false,
  error = null,
  onSelect,
}: WebLibraryPickerProps) {
  return (
    <section aria-labelledby="web-library-picker-title" className="web-library">
      <div className="web-page-heading">
        <div>
          <p className="web-eyebrow">Library</p>
          <h1 id="web-library-picker-title" className="web-page-title">
            Choose a library
          </h1>
          <p className="web-page-description">Select a mounted root to open the Soundrobe workspace.</p>
        </div>
      </div>
      {loading && (
        <p className="web-library-empty" role="status" aria-live="polite">
          Loading libraries…
        </p>
      )}
      {!loading && error && (
        <p className="web-library-empty" role="alert">
          {error}
        </p>
      )}
      {!loading && !error && roots.length === 0 && (
        <p className="web-library-empty">
          No libraries are mounted. Add a library under the service&apos;s configured root.
        </p>
      )}
      {!loading && !error && roots.length > 0 && (
        <div className="web-library-list">
          {roots.map((root) => (
            <button
              key={root.id}
              type="button"
              onClick={() => onSelect(root.path)}
              className="web-library-item"
            >
              <span className="min-w-0">
                <span className="web-library-item-name block">{root.name}</span>
                <span className="web-library-item-path block">{root.path}</span>
              </span>
              <span aria-hidden="true" className="text-lg text-[#9b7cf7]">›</span>
            </button>
          ))}
        </div>
      )}
    </section>
  );
}
