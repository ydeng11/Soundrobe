import React from "react";

export type WebServiceView = "library" | "activity" | "settings";

export interface WebServiceShellProps {
  activeView?: WebServiceView;
  onViewChange?: (view: WebServiceView) => void;
  onLogout?: () => void;
  children?: React.ReactNode;
}

const navigation: Array<{ view: WebServiceView; label: string }> = [
  { view: "library", label: "Library" },
  { view: "activity", label: "Activity" },
  { view: "settings", label: "Settings" },
];

const pageCopy: Record<WebServiceView, { title: string; description: string }> = {
  library: { title: "Your collection, in focus.", description: "Browse a mounted library and make precise metadata updates without leaving the service." },
  activity: { title: "Work in progress.", description: "Follow background tagging work and review outcomes as the service completes them." },
  settings: { title: "Settings.", description: "Control the browser service with the same focused tools as the desktop app." },
};

export function WebServiceShell({ activeView = "library", onViewChange, onLogout, children }: WebServiceShellProps) {
  const copy = pageCopy[activeView];
  const showPageIntro = activeView !== "settings";
  return (
    <div data-testid="web-service-shell" className="flex h-screen min-h-0 flex-col overflow-hidden bg-[#f5f0e8] text-[#28231d]">
      <header className="shrink-0 border-b border-[#ded5c8] bg-[#faf7f1]/95 px-4 py-3 backdrop-blur sm:px-6">
        <div className="mx-auto flex max-w-[90rem] items-center gap-4">
          <div className="flex min-w-0 items-center gap-2.5">
            <span aria-hidden="true" className="flex h-9 w-9 items-center justify-center rounded-full bg-[#332c25] text-[#f9f4eb] shadow-sm">
              <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.1" strokeLinecap="round" strokeLinejoin="round"><path d="M9 18V5l12-2v13" /><circle cx="6" cy="18" r="3" /><circle cx="18" cy="16" r="3" /></svg>
            </span>
            <span className="font-serif text-lg font-semibold tracking-tight">Soundrobe</span>
          </div>
          <nav aria-label="Primary navigation" className="flex min-w-0 flex-1 items-center gap-1 overflow-x-auto">
            {navigation.map(({ view, label }) => {
              const selected = view === activeView;
              return <button key={view} type="button" aria-current={selected ? "page" : undefined} onClick={() => onViewChange?.(view)} className={`shrink-0 rounded-full px-3 py-2 text-sm font-medium transition-colors focus:outline-none focus:ring-2 focus:ring-[#a85732]/40 ${selected ? "bg-[#e7d7c4] text-[#6f351f]" : "text-[#71675d] hover:bg-[#eee6da] hover:text-[#332c25]"}`}>{label}</button>;
            })}
          </nav>
          <div className="flex shrink-0 items-center gap-3">
            <span className="hidden items-center gap-1.5 text-xs font-medium text-[#62594f] sm:flex"><span aria-hidden="true" className="h-2 w-2 rounded-full bg-[#4d7c57]" />Session active</span>
            {onLogout && <button type="button" onClick={onLogout} className="rounded-full border border-[#d9cdbd] px-3 py-1.5 text-sm font-medium text-[#544a40] transition-colors hover:border-[#b9a58e] hover:bg-[#eee6da] focus:outline-none focus:ring-2 focus:ring-[#a85732]/40">Sign out</button>}
          </div>
        </div>
      </header>
      <main className="min-h-0 min-w-0 flex-1 overflow-y-auto">
        <div className={`mx-auto flex w-full max-w-[90rem] flex-col px-4 sm:px-6 lg:px-8 ${showPageIntro ? "gap-6 py-7" : "py-10 sm:py-12"}`}>
          {showPageIntro && <div className="max-w-3xl"><p className="text-xs font-semibold uppercase tracking-[0.18em] text-[#a85732]">Soundrobe service</p><h1 className="mt-2 font-serif text-3xl font-semibold tracking-tight text-[#28231d] sm:text-4xl">{copy.title}</h1><p className="mt-2 text-sm leading-6 text-[#71675d]">{copy.description}</p></div>}
          <div className="min-w-0">{children}</div>
        </div>
      </main>
    </div>
  );
}
