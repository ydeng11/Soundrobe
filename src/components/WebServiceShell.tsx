import React from "react";

export type WebServiceView = "library" | "activity" | "settings";

export interface WebServiceShellProps {
  /** The page selected by the host application. Navigation remains controlled by props. */
  activeView?: WebServiceView;
  /** Reports a requested page change without coupling the shell to a router. */
  onViewChange?: (view: WebServiceView) => void;
  children?: React.ReactNode;
}

const navigation: Array<{ view: WebServiceView; label: string }> = [
  { view: "library", label: "Library" },
  { view: "activity", label: "Activity" },
  { view: "settings", label: "Settings" },
];

export function WebServiceShell({
  activeView = "library",
  onViewChange,
  children,
}: WebServiceShellProps) {
  return (
    <div
      data-testid="web-service-shell"
      className="flex h-screen min-h-0 flex-col overflow-hidden bg-surface-alt text-text-primary lg:flex-row"
    >
      <aside className="border-b border-border bg-white/80 lg:flex lg:w-60 lg:shrink-0 lg:flex-col lg:border-b-0 lg:border-r">
        <div className="flex items-center gap-2 px-5 py-4 lg:px-6 lg:py-6">
          <span
            aria-hidden="true"
            className="flex h-8 w-8 items-center justify-center rounded-xl bg-accent text-white shadow-sm shadow-accent/20"
          >
            <svg width="17" height="17" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.25" strokeLinecap="round" strokeLinejoin="round">
              <path d="M9 18V5l12-2v13" />
              <circle cx="6" cy="18" r="3" />
              <circle cx="18" cy="16" r="3" />
            </svg>
          </span>
          <span className="text-sm font-semibold tracking-tight">Soundrobe</span>
        </div>

        <nav aria-label="Primary navigation" className="flex gap-1 overflow-x-auto px-3 pb-3 lg:flex-col lg:px-3 lg:py-2">
          {navigation.map(({ view, label }) => {
            const selected = view === activeView;
            return (
              <button
                key={view}
                type="button"
                aria-current={selected ? "page" : undefined}
                onClick={() => onViewChange?.(view)}
                className={`flex min-h-10 shrink-0 items-center gap-2 rounded-lg px-3 text-left text-sm font-medium transition-colors focus:outline-none focus:ring-2 focus:ring-accent/40 lg:w-full ${
                  selected
                    ? "bg-sidebar-active text-accent"
                    : "text-text-secondary hover:bg-sidebar-hover hover:text-text-primary"
                }`}
              >
                <span aria-hidden="true" className="w-4 text-center text-xs">
                  {view === "library" ? "♫" : view === "activity" ? "◷" : "⚙"}
                </span>
                {label}
              </button>
            );
          })}
        </nav>
      </aside>

      <main className="min-h-0 min-w-0 flex-1 overflow-y-auto">
        <div className="mx-auto flex w-full max-w-5xl flex-col gap-6 px-4 py-8 sm:px-6 lg:px-10 lg:py-12">
          <header>
            <p className="text-xs font-semibold uppercase tracking-[0.18em] text-accent">Soundrobe</p>
            <h1 className="mt-2 text-2xl font-semibold tracking-tight sm:text-3xl">
              {activeView === "library" ? "Your music library" : activeView[0].toUpperCase() + activeView.slice(1)}
            </h1>
            <p className="mt-2 max-w-2xl text-sm leading-6 text-text-secondary">
              {activeView === "library"
                ? "Choose a mounted library to start browsing and editing your collection."
                : activeView === "activity"
                  ? "Review recent work from the service."
                  : "Manage the settings available to this service."}
            </p>
          </header>
          <div className="min-w-0">{children}</div>
        </div>
      </main>
    </div>
  );
}
