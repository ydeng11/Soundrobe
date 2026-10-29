/**
 * Browser-runtime shell for the Soundrobe tagging workspace.
 *
 * Layout follows the measured Music Tag Web V2 chrome: a floating rounded
 * header inset from the viewport edge, brand block on the left, and primary
 * navigation beside it.
 *
 * This component belongs to the browser-only UI. The desktop composition keeps
 * using its own window chrome.
 */

import React from "react";
import "./tokens.css";
import "./web.css";

/** In-scope browser destinations. Library/player surfaces are out of scope. */
export type WebView = "workspace" | "sources" | "monitoring" | "settings";

export interface WebShellProps {
  activeView?: WebView;
  onViewChange?: (view: WebView) => void;
  children?: React.ReactNode;
}

const NAVIGATION: Array<{ view: WebView; label: string; icon: React.ReactNode }> = [
  { view: "workspace", label: "Workspace", icon: <WorkspaceIcon /> },
  { view: "monitoring", label: "Monitoring", icon: <MonitoringIcon /> },
  { view: "sources", label: "Tag sources", icon: <SourcesIcon /> },
  { view: "settings", label: "Settings", icon: <SettingsIcon /> },
];

export function WebShell({ activeView = "workspace", onViewChange, children }: WebShellProps) {
  return (
    <div data-testid="web-shell" className="sr-web sr-app">
      <header className="sr-header">
        <div className="sr-brand">
          <span aria-hidden="true" className="sr-brand__mark">
            <MusicIcon />
          </span>
          <span className="sr-brand__name">Soundrobe</span>
          <span className="sr-brand__badge">WEB</span>
        </div>

        <nav aria-label="Primary" className="sr-nav">
          {NAVIGATION.map(({ view, label, icon }) => {
            const selected = view === activeView;
            return (
              <button
                key={view}
                type="button"
                aria-label={label}
                aria-current={selected ? "page" : undefined}
                className={`sr-nav__item ${selected ? "is-active" : ""}`}
                onClick={() => onViewChange?.(view)}
              >
                <span aria-hidden="true">{icon}</span>
                <span>{label}</span>
              </button>
            );
          })}
        </nav>

        <div className="sr-header__spacer" />
      </header>

      <main className={`sr-main sr-main--${activeView}`}>{children}</main>
    </div>
  );
}

function WorkspaceIcon() {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8">
      <rect x="3" y="4" width="18" height="16" rx="2" />
      <path d="M3 9h18M9 9v11" />
    </svg>
  );
}

function SourcesIcon() {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8">
      <path d="M12 3v4M12 17v4M3 12h4M17 12h4" />
      <circle cx="12" cy="12" r="3.5" />
    </svg>
  );
}

function MonitoringIcon() {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8">
      <path d="M3 12h4l2.5-6 3 12 2.5-6h6" />
    </svg>
  );
}

function SettingsIcon() {
  return (
    <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8">
      <circle cx="12" cy="12" r="3" />
      <path d="M12 2.5v3M12 18.5v3M4.2 4.2l2.1 2.1M17.7 17.7l2.1 2.1M2.5 12h3M18.5 12h3M4.2 19.8l2.1-2.1M17.7 6.3l2.1-2.1" />
    </svg>
  );
}

function MusicIcon() {
  return (
    <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8">
      <path d="M9 18V5l11-2v12" />
      <circle cx="6" cy="18" r="3" />
      <circle cx="17" cy="15" r="3" />
    </svg>
  );
}
