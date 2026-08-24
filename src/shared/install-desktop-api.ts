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
import { createWebDesktopApi } from "./web-adapter";

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
 * Install `window.api` for the active runtime. Idempotent; plain browsers use
 * the fetch-backed web adapter and Tauri uses the native bridge. Call once
 * before React renders.
 */
export function installDesktopApi(): void {
  const w = window as unknown as { api?: unknown };
  if (w.api) {
    return;
  }
  if (!isTauriRuntime()) {
    w.api = createWebDesktopApi();
    return;
  }
  w.api = createTauriDesktopApi();

  // A failed attach is logged so a broken live-log stream stays observable.
  listen<LogEntry>("debug:log", (event) => {
    const entry = event.payload;
    const prefix = `[${entry.tag}] ${entry.level.toUpperCase()}`;
    const method = CONSOLE_METHOD[entry.level] ?? "log";
    console[method](`[soundrobe] ${prefix} ${entry.message}`, entry.data ?? "");
  }).catch((err) => {
    console.error('[soundrobe] failed to attach Tauri "debug:log" listener:', err);
  });
}
