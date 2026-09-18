import React from "react";
import "../styles/web.css";

export type WebServiceView = "home" | "workspace" | "library" | "activity" | "settings";

export interface WebServiceShellProps {
  activeView?: WebServiceView;
  onViewChange?: (view: WebServiceView) => void;
  onLogout?: () => void;
  children?: React.ReactNode;
}

const navigation: Array<{ view: WebServiceView; label: string; icon: React.ReactNode }> = [
  { view: "home", label: "Home", icon: <HomeIcon /> },
  { view: "workspace", label: "Workspace", icon: <WorkspaceIcon /> },
  { view: "library", label: "Library", icon: <LibraryIcon /> },
  { view: "activity", label: "Activity", icon: <ActivityIcon /> },
  { view: "settings", label: "Settings", icon: <SettingsIcon /> },
];

export function WebServiceShell({
  activeView = "library",
  onViewChange,
  onLogout,
  children,
}: WebServiceShellProps) {
  return (
    <div data-testid="web-service-shell" className="web-app h-screen overflow-hidden">
      <header className="web-header">
        <div className="web-brand">
          <span aria-hidden="true" className="web-brand-mark"><MusicIcon /></span>
          <span className="web-brand-name">Soundrobe</span>
          <span className="web-brand-badge">WEB</span>
        </div>
        <nav aria-label="Primary navigation" className="web-header-nav">
          {navigation.map(({ view, label, icon }) => {
            const selected = view === activeView;
            return (
              <button
                key={view}
                type="button"
                aria-label={label}
                aria-current={selected ? "page" : undefined}
                onClick={() => onViewChange?.(view)}
                className={`web-header-nav-item ${selected ? "is-selected" : ""}`}
              >
                <span className="web-nav-icon" aria-hidden="true">{icon}</span>
                <span>{label}</span>
              </button>
            );
          })}
        </nav>
        <div className="web-header-spacer" />
        <span className="web-session" title="Session active"><span aria-hidden="true" />Session active</span>
        {onLogout && <button type="button" className="web-account" onClick={onLogout} aria-label="Sign out">Sign out</button>}
      </header>

      <div className="web-body">
        <main className="web-main min-h-0 overflow-y-auto">
          <div className="web-content">{children}</div>
        </main>
      </div>
    </div>
  );
}

function HomeIcon() {
  return <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8"><path d="m3 10 9-7 9 7" /><path d="M5 9v11h14V9" /><path d="M9 20v-6h6v6" /></svg>;
}

function WorkspaceIcon() {
  return <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8"><rect x="3" y="4" width="18" height="16" rx="2" /><path d="M3 9h18M8 4v5M16 4v5" /></svg>;
}

function LibraryIcon() {
  return <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8"><path d="M4 5h16v15H4z" /><path d="M8 5v15M4 9h4M4 13h4M4 17h4M12 9h5M12 13h5M12 17h3" /></svg>;
}

function ActivityIcon() {
  return <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8"><path d="M4 19V5M4 19h16" /><path d="m7 15 3-4 3 2 5-6" /></svg>;
}

function SettingsIcon() {
  return <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8"><path d="M12 3v3M12 18v3M3 12h3M18 12h3M5.6 5.6l2.1 2.1M16.3 16.3l2.1 2.1M18.4 5.6l-2.1 2.1M7.7 16.3l-2.1 2.1" /><circle cx="12" cy="12" r="4" /></svg>;
}

function MusicIcon() {
  return <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8"><path d="M9 18V5l11-2v12" /><circle cx="6" cy="18" r="3" /><circle cx="17" cy="15" r="3" /></svg>;
}
