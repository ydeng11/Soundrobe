/**
 * Debug logger for auto-tag pipeline.
 *
 * Features:
 *  - Timestamped, tagged log entries
 *  - Configurable output: console + optional file
 *  - Tag-based filtering (e.g., "auto-tag", "cache", "dataset", "musicbrainz")
 *  - Forwarding logs to the renderer process via IPC
 *  - Step timing (start/end pairs)
 */

import { appendFileSync, existsSync, mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { homedir } from "node:os";
import { BrowserWindow, ipcMain } from "electron";

// ── Types ───────────────────────────────────────────────────────────

export interface LogEntry {
  timestamp: string;
  tag: string;
  level: "info" | "warn" | "error" | "debug";
  message: string;
  data?: unknown;
}

type LogCallback = (entry: LogEntry) => void;

// ── Singleton ───────────────────────────────────────────────────────

class DebugLogger {
  private enabled = false;
  private logToFile = false;
  private logFilePath: string | null = null;
  private subscribers: Set<LogCallback> = new Set();
  private logDir: string;
  private timers = new Map<string, number>();
  private truncatedLogFiles = new Set<string>();

  constructor() {
    this.logDir = join(homedir(), ".auto-tagger");
  }

  /** Enable/disable debug logging. */
  setEnabled(enabled: boolean): void {
    this.enabled = enabled;
    if (enabled) {
      this.ensureLogDir();
      this.logFilePath = join(
        this.logDir,
        `auto-tag-debug-${new Date().toISOString().slice(0, 10)}.log`,
      );
      this.logToFile = true;
      if (!this.truncatedLogFiles.has(this.logFilePath)) {
        this.truncatedLogFiles.add(this.logFilePath);
        // Truncate once per process so toggling debug mode does not erase diagnostics.
        try {
          writeFileSync(this.logFilePath, "", "utf-8");
        } catch {
          // Silently skip if write fails
        }
      }
      this.info("debug", `Debug logging enabled → ${this.logFilePath}`);
    } else {
      this.info("debug", "Debug logging disabled");
      this.logToFile = false;
    }
  }

  getEnabled(): boolean {
    return this.enabled;
  }

  getLogFilePath(): string | null {
    return this.logFilePath;
  }

  /** Subscribe to receive all log entries (e.g., for forwarding to renderer). */
  subscribe(cb: LogCallback): () => void {
    this.subscribers.add(cb);
    return () => this.subscribers.delete(cb);
  }

  // ── Logging methods ────────────────────────────────────────────

  info(tag: string, message: string, data?: unknown): void {
    this.emit("info", tag, message, data);
  }

  warn(tag: string, message: string, data?: unknown): void {
    this.emit("warn", tag, message, data);
  }

  error(tag: string, message: string, data?: unknown): void {
    this.emit("error", tag, message, data);
  }

  debug(tag: string, message: string, data?: unknown): void {
    this.emit("debug", tag, message, data);
  }

  // ── Timing ─────────────────────────────────────────────────────

  /** Start a named timer. Call `endTimer` to log elapsed ms. */
  startTimer(timerName: string): void {
    this.timers.set(timerName, performance.now());
  }

  /** End a named timer and log the elapsed time. Returns elapsed ms or 0. */
  endTimer(timerName: string, tag?: string, message?: string): number {
    const start = this.timers.get(timerName);
    if (start === undefined) return 0;
    const elapsed = Math.round(performance.now() - start);
    this.timers.delete(timerName);
    const logTag = tag ?? "timer";
    const logMsg = message
      ? `${message} (${elapsed}ms)`
      : `${timerName}: ${elapsed}ms`;
    this.info(logTag, logMsg);
    return elapsed;
  }

  // ── Serialization helpers ──────────────────────────────────────

  /** Recursively convert Error objects to plain objects for JSON serialization. */
  private serializeData(data: unknown): unknown {
    if (data instanceof Error) {
      const plain: Record<string, unknown> = {
        name: data.name,
        message: data.message,
        stack: data.stack,
      };
      // Extract any custom enumerable properties the Error might carry
      for (const key of Object.keys(data)) {
        plain[key] = (data as unknown as Record<string, unknown>)[key];
      }
      // Recursively handle cause chains
      if (data.cause !== undefined) {
        plain.cause = this.serializeData(data.cause);
      }
      return plain;
    }
    return data;
  }

  // ── Internals ──────────────────────────────────────────────────

  private emit(
    level: LogEntry["level"],
    tag: string,
    message: string,
    data?: unknown,
  ): void {
    if (!this.enabled) return;

    const entry: LogEntry = {
      timestamp: new Date().toISOString(),
      tag,
      level,
      message,
      data: this.serializeData(data),
    };

    // Console output
    const prefix = `[${entry.timestamp.slice(11, 23)}] [${tag.padEnd(12)}] ${level.toUpperCase()}`;
    switch (level) {
      case "error":
        console.error(`${prefix} ${message}`, data ?? "");
        break;
      case "warn":
        console.warn(`${prefix} ${message}`, data ?? "");
        break;
      default:
        console.log(`${prefix} ${message}`, data ?? "");
    }

    // File output
    if (this.logToFile && this.logFilePath) {
      try {
        const line = JSON.stringify(entry) + "\n";
        appendFileSync(this.logFilePath, line, "utf-8");
      } catch {
        // Silently drop log if file write fails
      }
    }

    // Notify subscribers (renderer forwarding)
    for (const cb of this.subscribers) {
      try {
        cb(entry);
      } catch {
        // Silently drop subscriber errors
      }
    }
  }

  private ensureLogDir(): void {
    if (!existsSync(this.logDir)) {
      mkdirSync(this.logDir, { recursive: true });
    }
  }
}

// ── Global singleton ────────────────────────────────────────────────

const logger = new DebugLogger();

export default logger;

// ── IPC bridge: forward logs to renderer ────────────────────────────

let forwardedLogCount = 0;
const debugSubscriptions = new Map<number, () => void>();

/**
 * Register IPC handler so the renderer can forward logs to DevTools.
 * Actually, we use a push model: the renderer calls `debug:subscribe`
 * once, and the main process sends `debug:log` events back.
 */
export function registerDebugIpc(): void {
  // Renderer subscribes to receive live log entries
  ipcMain.handle("debug:subscribe", (event) => {
    const senderId = event.sender.id;
    debugSubscriptions.get(senderId)?.();

    const unsub = logger.subscribe((entry) => {
      forwardedLogCount++;
      // Send to all renderer windows
      for (const win of BrowserWindow.getAllWindows()) {
        if (!win.isDestroyed()) {
          try {
            win.webContents.send("debug:log", entry);
          } catch {
            // Window might be closing
          }
        }
      }
    });

    debugSubscriptions.set(senderId, unsub);
    event.sender.once("destroyed", () => {
      debugSubscriptions.get(senderId)?.();
      debugSubscriptions.delete(senderId);
    });

    return { subscribed: true };
  });

  // Get current debug state
  ipcMain.handle("debug:status", () => ({
    enabled: logger.getEnabled(),
    logFile: logger.getLogFilePath(),
    forwardedCount: forwardedLogCount,
  }));

  // Toggle debug mode
  ipcMain.handle("debug:toggle", (_event, enabled: boolean) => {
    logger.setEnabled(enabled);
    return { enabled: logger.getEnabled() };
  });
}

/** Forward a log entry from the renderer to the main process logger. */
export function forwardRendererLog(entry: LogEntry): void {
  switch (entry.level) {
    case "error":
      logger.error(entry.tag, entry.message, entry.data);
      break;
    case "warn":
      logger.warn(entry.tag, entry.message, entry.data);
      break;
    case "debug":
      logger.debug(entry.tag, entry.message, entry.data);
      break;
    default:
      logger.info(entry.tag, entry.message, entry.data);
  }
}
