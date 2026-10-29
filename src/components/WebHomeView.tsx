import React from "react";
import type { WebServiceView } from "./WebServiceShell";

interface WebHomeViewProps {
  libraryPath: string | null;
  albumCount: number;
  trackCount: number;
  onViewChange: (view: WebServiceView) => void;
  onOpenLibrary: () => void;
}

export function WebHomeView({
  libraryPath,
  albumCount,
  trackCount,
  onViewChange,
  onOpenLibrary,
}: WebHomeViewProps) {
  return (
    <section data-testid="web-home-view" className="web-home">
      <div className="web-home-hero">
        <p className="web-eyebrow">Soundrobe browser service</p>
        <h1>Make every track feel at home.</h1>
        <p>
          Edit metadata, review your library, and keep background tagging work
          visible without changing the desktop app.
        </p>
      </div>

      <div className="web-home-stats" aria-label="Current library summary">
        <div className="web-home-stat">
          <div className="web-home-stat-label">Current root</div>
          <div className="web-home-stat-value">{libraryPath ? "Loaded" : "Not loaded"}</div>
        </div>
        <div className="web-home-stat">
          <div className="web-home-stat-label">Albums</div>
          <div className="web-home-stat-value">{libraryPath ? albumCount : "—"}</div>
        </div>
        <div className="web-home-stat">
          <div className="web-home-stat-label">Tracks</div>
          <div className="web-home-stat-value">{libraryPath ? trackCount : "—"}</div>
        </div>
      </div>
      <p className="web-home-root" title={libraryPath ?? undefined}>
        {libraryPath ? `Mounted root: ${libraryPath}` : "Choose a mounted library to begin."}
      </p>

      <div className="web-home-actions">
        <button type="button" className="web-button-primary" onClick={onOpenLibrary}>
          {libraryPath ? "Change library" : "Choose a library"}
        </button>
        {libraryPath && (
          <button type="button" className="web-button-secondary" onClick={() => onViewChange("workspace")}>
            Open workspace
          </button>
        )}
        {!libraryPath && (
          <button type="button" className="web-button-secondary" onClick={() => onViewChange("library")}>
            View library roots
          </button>
        )}
      </div>
    </section>
  );
}
