/**
 * Soundrobe browser application.
 *
 * This is the entry point for the self-hosted web runtime. It is deliberately
 * independent of the standalone desktop composition in `src/App.tsx`: the two
 * share the Rust backend, the `DesktopAPI` contract and the token/state
 * helpers, but not the window chrome, layout or navigation.
 */

import React from "react";
import { WebLoginScreen } from "../components/WebLoginScreen";
import { getWebSession, loginWebSession, logoutWebSession } from "../shared/web-adapter";
import { MonitoringView } from "./MonitoringView";
import { SettingsView } from "./SettingsView";
import { TagSourcesView } from "./TagSourcesView";
import { WebShell, type WebView } from "./WebShell";
import { WorkspaceView } from "./WorkspaceView";
import "./tokens.css";
import "./web.css";

type AuthState = "checking" | "login" | "authenticated";

export interface WebAppProps {
  /** Injected for tests; defaults to the installed `window.api` facade. */
  api?: Window["api"];
}

export function WebApp({ api = window.api }: WebAppProps) {
  const [authState, setAuthState] = React.useState<AuthState>("checking");
  const [view, setView] = React.useState<WebView>("workspace");
  /** Set when Monitoring sends the reader to the album it flagged. */
  const [pendingAlbum, setPendingAlbum] = React.useState<string | null>(null);

  React.useEffect(() => {
    let active = true;
    void (async () => {
      try {
        const session = await getWebSession();
        if (active) setAuthState(session.authenticated ? "authenticated" : "login");
      } catch {
        if (active) setAuthState("login");
      }
    })();
    return () => {
      active = false;
    };
  }, []);

  const handleLogin = React.useCallback(async (password: string) => {
    const session = await loginWebSession(password);
    if (!session.authenticated) {
      throw new Error("Incorrect password");
    }
    setAuthState("authenticated");
  }, []);

  const handleSignOut = React.useCallback(async () => {
    try {
      await logoutWebSession();
    } finally {
      setAuthState("login");
      setView("workspace");
    }
  }, []);

  if (authState === "checking") {
    return (
      <div className="sr-web sr-app">
        <div className="sr-empty" data-testid="web-auth-checking">
          <p className="sr-empty__body">Checking session…</p>
        </div>
      </div>
    );
  }

  if (authState === "login") {
    return <WebLoginScreen onLogin={handleLogin} />;
  }

  return (
    <WebShell activeView={view} onViewChange={setView} onSignOut={() => void handleSignOut()}>
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
