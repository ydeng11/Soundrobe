/**
 * Runtime loader for the `window.api` facade.
 *
 * In the Tauri webview this installs the [`createTauriDesktopApi`]{@link}
 * adapter before React renders and forwards `debug:log` events to DevTools.
 * In plain-browser unit tests it is an intentional no-op.
 */

import { listen } from "@tauri-apps/api/event";
import type { LogEntry } from "./desktop-api";
import { createTauriDesktopApi } from "./tauri-adapter";

/** True when running inside the Tauri webview. */
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
 * Install `window.api` for the Tauri runtime. Idempotent and a no-op in a
 * plain browser. Call once before React renders.
 */
export function installDesktopApi(): void {
  if (!isTauriRuntime()) {
    return;
  }
  const w = window as unknown as { api?: unknown };
  if (w.api) {
    return;
  }
  w.api = createTauriDesktopApi();

  // A failed attach is logged so a broken live-log stream stays observable.
  listen<LogEntry>("debug:log", (event) => {
    const entry = event.payload;
    const prefix = `[${entry.tag}] ${entry.level.toUpperCase()}`;
    const method = CONSOLE_METHOD[entry.level] ?? "log";
    console[method](`[auto-tagger] ${prefix} ${entry.message}`, entry.data ?? "");
  }).catch((err) => {
    console.error('[auto-tagger] failed to attach Tauri "debug:log" listener:', err);
  });
}
