/**
 * Soundrobe browser application.
 *
 * This is the entry point for the self-hosted web runtime. It is deliberately
 * independent of the standalone desktop composition in `src/App.tsx`: the two
 * share the Rust backend, the `DesktopAPI` contract and the token/state
 * helpers, but not the window chrome, layout or navigation.
 */

import React from "react";
import { MonitoringView } from "./MonitoringView";
import { SettingsView } from "./SettingsView";
import { TagSourcesView } from "./TagSourcesView";
import { WebShell, type WebView } from "./WebShell";
import { WorkspaceView } from "./WorkspaceView";
import "./tokens.css";
import "./web.css";

export interface WebAppProps {
  /** Injected for tests; defaults to the installed `window.api` facade. */
  api?: Window["api"];
}

export function WebApp({ api = window.api }: WebAppProps) {
  const [view, setView] = React.useState<WebView>("workspace");
  /** Set when Monitoring sends the reader to the album it flagged. */
  const [pendingAlbum, setPendingAlbum] = React.useState<string | null>(null);

  return (
    <WebShell activeView={view} onViewChange={setView}>
      {view === "workspace" && (
        <WorkspaceView
          api={api}
          openAlbumPath={pendingAlbum}
          onAlbumOpened={() => setPendingAlbum(null)}
        />
      )}
      {view === "sources" && <TagSourcesView api={api} />}
      {view === "monitoring" && (
        <MonitoringView
          loadTasks={() => import("../shared/web-adapter").then((m) => m.listWebTasks())}
          cancelTask={(taskId) => api.cancelTask(taskId)}
          onOpenAlbum={(path) => {
            setPendingAlbum(path);
            setView("workspace");
          }}
          onOpenSettings={() => setView("settings")}
        />
      )}
      {view === "settings" && <SettingsView />}
    </WebShell>
  );
}
