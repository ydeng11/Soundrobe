/**
 * Runtime loader for the `window.api` facade.
 *
 * The renderer never knows which native runtime backs `window.api`:
 *   - Under **Electron**, `electron/preload.ts` already exposes a frozen,
 *     context-bridged `window.api` before the renderer bundle runs; this loader
 *     must not (and cannot) overwrite it.
 *   - Under **Tauri**, no preload runs, so this loader installs the
 *     [`createTauriDesktopApi`]{@link} adapter as `window.api` before React
 *     renders, and forwards the pushed `debug:log` stream to the DevTools
 *     console to match the Electron preload's inline `ipcRenderer.on` behavior.
 *
 * Importing `@tauri-apps/api` is safe under Electron: its `invoke`/`listen` only
 * touch `window.__TAURI_INTERNALS__` when called, never at import time.
 */

import { listen } from "@tauri-apps/api/event";
import type { LogEntry } from "./desktop-api";
import { createTauriDesktopApi } from "./tauri-adapter";

/** True when running inside the Tauri webview (not Electron). */
export function isTauriRuntime(): boolean {
  return (
    typeof window !== "undefined" && "__TAURI_INTERNALS__" in window
  );
}

declare global {
  interface Window {
    /** Tauri internal IPC handle — presence identifies the Tauri runtime. */
    __TAURI_INTERNALS__?: unknown;
  }
}

const CONSOLE_METHOD: Record<LogEntry["level"], "error" | "warn" | "debug" | "log"> = {
  error: "error",
  warn: "warn",
  debug: "debug",
  info: "log",
};

/**
 * Install `window.api` for the Tauri runtime. Idempotent and a no-op under
 * Electron. Call once, before `ReactDOM.createRoot(...).render(...)`.
 */
export function installDesktopApi(): void {
  if (!isTauriRuntime()) {
    // Electron: context bridge already set window.api; nothing to do.
    return;
  }
  const w = window as unknown as { api?: unknown };
  if (w.api) {
    return;
  }
  w.api = createTauriDesktopApi();

  // Forward pushed debug log entries to the renderer console, mirroring the
  // Electron preload's inline `ipcRenderer.on("debug:log", ...)`.
  listen<LogEntry>("debug:log", (event) => {
    const entry = event.payload;
    const prefix = `[${entry.tag}] ${entry.level.toUpperCase()}`;
    const method = CONSOLE_METHOD[entry.level] ?? "log";
    console[method](`[auto-tagger] ${prefix} ${entry.message}`, entry.data ?? "");
  }).catch(() => {});
}