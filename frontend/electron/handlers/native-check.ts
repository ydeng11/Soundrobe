/**
 * Native module ABI compatibility checker.
 *
 * Verifies that native modules (better-sqlite3) are compiled for the correct
 * Node.js ABI version before the app uses them. If the ABI is wrong (e.g.,
 * compiled for system Node.js but running in Electron), offers to auto-rebuild.
 *
 * Exported separately from main.ts so it can be unit-tested.
 */

import { app, dialog } from "electron";
import { existsSync } from "node:fs";
import { join, resolve } from "node:path";
import { execSync } from "node:child_process";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";

// ── Lazy better-sqlite3 loader ────────────────────────────────────
//
// We cannot import better-sqlite3 at module scope — it would fail if the ABI
// is wrong, and it happens before ensureNativeModules() gets a chance to run.
// Instead, all consumers call getBetterSqlite3() which lazily requires it.

export interface BetterSqlite3Database {
  new (path: string, options?: { readonly?: boolean }): BetterSqlite3Database;
  prototype: BetterSqlite3Database;
  pragma(sql: string, options?: { simple: boolean }): Record<string, string>[] | string;
  prepare(sql: string): BetterSqlite3Statement;
  exec(sql: string): void;
  close(): void;
}

export interface BetterSqlite3Statement {
  run(...params: unknown[]): { changes: number; lastInsertRowid: number };
  get(...params: unknown[]): unknown;
  all(...params: unknown[]): unknown[];
  bind(...params: unknown[]): void;
}

let lazyDb: BetterSqlite3Database | null = null;

/**
 * Lazily load better-sqlite3 — safe to call after ensureNativeModules() has
 * checked ABI compatibility. Returns the Database constructor.
 */
export function getBetterSqlite3(): BetterSqlite3Database {
  if (!lazyDb) {
    lazyDb = createRequire(import.meta.url)("better-sqlite3") as unknown as BetterSqlite3Database;
  }
  return lazyDb;
}

const __dirname = fileURLToPath(new URL(".", import.meta.url));
const requireNative = createRequire(import.meta.url);

function getBetterSqliteNativePath(): string {
  return join(
    __dirname,
    "../../node_modules/better-sqlite3/build/Release/better_sqlite3.node",
  );
}

function loadBetterSqliteNativeBinding(nodeModulePath: string): void {
  const resolvedPath = requireNative.resolve(nodeModulePath);
  delete requireNative.cache[resolvedPath];
  requireNative(resolvedPath);
}

/**
 * Verify native modules (better-sqlite3) are compiled for Electron's ABI.
 * If not, offer to auto-rebuild. Returns false if the user chose to quit.
 */
export async function ensureNativeModules(): Promise<boolean> {
  const nodeModulePath = getBetterSqliteNativePath();

  // Quick pre-check: does the .node file exist?
  if (!existsSync(nodeModulePath)) {
    // Module might not be installed yet — let it fail naturally
    return true;
  }

  // Try to load the module to detect ABI mismatch
  try {
    loadBetterSqliteNativeBinding(nodeModulePath);
    return true; // loaded fine
  } catch (err: unknown) {
    const typedErr = err as NodeJS.ErrnoException;
    if (typedErr?.code !== "ERR_DLOPEN_FAILED") {
      // Some other error (e.g. module not found) — let it fail naturally later
      return true;
    }
  }

  // ABI mismatch detected — offer to rebuild
  if (!app.isReady()) {
    // Can't show dialog before app is ready — defer to first window
    return true;
  }

  const result = await dialog.showMessageBox({
    type: "warning",
    title: "Native Module Mismatch",
    message: "better-sqlite3 was compiled for a different Node.js version.",
    detail:
      "The auto-tag and dataset features won't work until it's rebuilt for Electron's ABI. Rebuild now?",
    buttons: ["Rebuild", "Quit"],
    defaultId: 0,
    cancelId: 1,
  });

  if (result.response === 1) {
    return false; // user chose to quit
  }

  // Run rebuild
  const ok = await attemptRebuild();
  if (ok) return true;

  // Rebuild didn't work — show final error
  await dialog.showMessageBox({
    type: "error",
    title: "Native Module Error",
    message: "Could not rebuild better-sqlite3 for Electron.",
    detail: `Please try running manually:\n\n  cd ${resolve(__dirname, "../..")} && npm run rebuild:electron\n\nOr reinstall: npm install`,
  });

  return true; // let the app start anyway, handlers will fail
}

/**
 * Attempt to rebuild better-sqlite3 for Electron's ABI.
 * Exported so tests can verify rebuild behaviour.
 */
export async function attemptRebuild(): Promise<boolean> {
  const projectRoot = resolve(__dirname, "../..");
  try {
    execSync("npx electron-rebuild -f -w better-sqlite3", {
      cwd: projectRoot,
      stdio: "pipe",
      timeout: 120_000,
    });

    // Verify rebuild succeeded
    loadBetterSqliteNativeBinding(getBetterSqliteNativePath());
    return true;
  } catch {
    return false;
  }
}
