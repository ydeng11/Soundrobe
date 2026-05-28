import { app, BrowserWindow, ipcMain, dialog, screen, Menu, clipboard } from "electron";
import path from "path";
import fs from "fs";
import { fileURLToPath } from "url";
import { ensureNativeModules } from "./handlers/native-check";
import { registerLibraryHandlers } from "./handlers/library";
import { registerTrackHandlers } from "./handlers/tracks";
import { registerCoverHandlers } from "./handlers/cover";
import { registerDirectoryHandlers } from "./handlers/directory";
import {
  startAutoTag,
  getProgress,
  cancelTask,
  getDatasetStatus,
  getConfig,
  onAutoTagEvent,
  refreshConfig,
  saveConfig,
  setDebugMode,
} from "./handlers/auto-tag";
import { registerDebugIpc } from "./handlers/debug";
import { registerAuditHandlers, onAuditEvent } from "./handlers/audit";
const __dirname = path.dirname(fileURLToPath(import.meta.url));

const isDev = !app.isPackaged;

let mainWindow: BrowserWindow | null = null;

// ── Window state persistence ─────────────────────────────────────

const WINDOW_STATE_PATH = path.join(
  app.getPath("home"),
  ".auto-tagger",
  "window-state.json",
);

interface WindowState {
  x?: number;
  y?: number;
  width: number;
  height: number;
  isMaximized?: boolean;
}

function loadWindowState(): WindowState | null {
  try {
    if (fs.existsSync(WINDOW_STATE_PATH)) {
      const raw = fs.readFileSync(WINDOW_STATE_PATH, "utf-8");
      return JSON.parse(raw);
    }
  } catch {
    // Corrupted state — ignore
  }
  return null;
}

function saveWindowState(win: BrowserWindow): void {
  try {
    // Don't save if minimized or destroyed
    if (win.isMinimized() || win.isDestroyed()) return;

    const bounds = win.getBounds();
    const state: WindowState = {
      x: bounds.x,
      y: bounds.y,
      width: bounds.width,
      height: bounds.height,
      isMaximized: win.isMaximized(),
    };

    const dir = path.dirname(WINDOW_STATE_PATH);
    if (!fs.existsSync(dir)) {
      fs.mkdirSync(dir, { recursive: true });
    }
    fs.writeFileSync(WINDOW_STATE_PATH, JSON.stringify(state, null, 2));
  } catch {
    // Best-effort
  }
}

function createWindow() {
  const savedState = loadWindowState();
  mainWindow = new BrowserWindow({
    width: savedState?.width ?? 1200,
    height: savedState?.height ?? 800,
    x: savedState?.x,
    y: savedState?.y,
    minWidth: 900,
    minHeight: 600,
    backgroundColor: "#1a1a2e",
    webPreferences: {
      preload: path.join(__dirname, "preload.mjs"),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: false,
    },
    titleBarStyle: "hiddenInset",
    titleBarOverlay: {
      color: "rgba(255, 255, 255, 0.95)",
      symbolColor: "#1d1d1f",
      height: 38,
    },
    show: false,
  });

  // Check saved position is on an available display
  if (savedState?.x != null && savedState?.y != null) {
    const displays = screen.getAllDisplays();
    const onScreen = displays.some((d) => {
      const { x, y, width, height } = d.workArea;
      return (
        savedState.x! >= x &&
        savedState.x! < x + width - 100 &&
        savedState.y! >= y &&
        savedState.y! < y + height - 100
      );
    });
    if (!onScreen) {
      // Center on primary display if saved position is off-screen
      mainWindow.center();
    }
  }

  // Restore maximized state
  if (savedState?.isMaximized) {
    mainWindow.maximize();
  }

  // Save window state on changes
  const debouncedSave = debounce(() => {
    if (mainWindow && !mainWindow.isDestroyed()) {
      saveWindowState(mainWindow);
    }
  }, 300);

  mainWindow.on("resize", debouncedSave);
  mainWindow.on("move", debouncedSave);
  mainWindow.on("maximize", () => saveWindowState(mainWindow!));
  mainWindow.on("unmaximize", () => saveWindowState(mainWindow!));

  mainWindow.once("ready-to-show", () => {
    mainWindow?.show();
  });

  if (isDev) {
    const e2eRendererPath = process.env.AUTO_TAGGER_E2E_RENDERER_PATH;
    if (e2eRendererPath) {
      mainWindow.loadFile(e2eRendererPath);
    } else {
      const devServerUrl =
        process.env.VITE_DEV_SERVER_URL ?? "http://localhost:5173";
      mainWindow.loadURL(devServerUrl);
      mainWindow.webContents.openDevTools({ mode: "detach" });
    }

    // Suppress benign DevTools protocol errors (version mismatch noise)
    mainWindow.webContents.on("console-message", (event, level, message, line, sourceId) => {
      if (sourceId?.startsWith("devtools://") && message.includes("Autofill")) {
        event.preventDefault();
      }
    });
  } else {
    mainWindow.loadFile(path.join(__dirname, "../dist/index.html"));
  }

  // Save state on close
  mainWindow.on("close", () => {
    if (mainWindow && !mainWindow.isDestroyed()) {
      saveWindowState(mainWindow);
    }
  });

  mainWindow.on("closed", () => {
    mainWindow = null;
  });
}

// Simple debounce helper
function debounce<T extends (...args: unknown[]) => void>(
  fn: T,
  delay: number,
): (...args: Parameters<T>) => void {
  let timer: ReturnType<typeof setTimeout>;
  return (...args: Parameters<T>) => {
    clearTimeout(timer);
    timer = setTimeout(() => fn(...args), delay);
  };
}

/** Subscribe to an event emitter and forward each event to all BrowserWindows. */
function forwardToWindows<T>(
  subscribe: (fn: (event: T) => void) => () => void,
  channel: string,
): void {
  subscribe((event) => {
    for (const win of BrowserWindow.getAllWindows()) {
      if (!win.isDestroyed()) {
        win.webContents.send(channel, event);
      }
    }
  });
}

/** Resolve the sender window, falling back to mainWindow for stale renderers. */
function resolveWindow(event: { sender: Electron.WebContents }): BrowserWindow | null {
  const sender = BrowserWindow.fromWebContents(event.sender);
  const win = sender && !sender.isDestroyed() ? sender : mainWindow;
  return win && !win.isDestroyed() ? win : null;
}

/**
 * Verify native modules (better-sqlite3) are compiled for Electron's ABI.
 * If not, offer to auto-rebuild. Returns false if the user chose to quit.
 */

app.whenReady().then(async () => {
  // Verify native modules are compiled for Electron's ABI before proceeding
  const modulesOk = await ensureNativeModules();
  if (!modulesOk) {
    // User chose to quit rather than rebuild
    app.quit();
    return;
  }

  // Register all IPC handlers
  registerLibraryHandlers();
  registerTrackHandlers();
  registerCoverHandlers();
  registerDirectoryHandlers();
  registerDebugIpc();
  registerAuditHandlers();
  forwardToWindows(onAutoTagEvent, "auto-tag:event");
  forwardToWindows(onAuditEvent, "audit:event");

  // Native folder picker — use event.sender to get the invoking window, with a
  // mainWindow fallback for stale/reloaded dev renderers.
  ipcMain.handle("window:focused", async () => {});
  ipcMain.handle("dialog:open-folder", async (event) => {
    try {
      if (process.env.AUTO_TAGGER_E2E_LIBRARY_PATH) {
        return process.env.AUTO_TAGGER_E2E_LIBRARY_PATH;
      }

      const win = resolveWindow(event);
      if (!win) return null;

      const result = await dialog.showOpenDialog(win, {
        title: "Open Music Folder",
        properties: ["openDirectory"],
      });
      if (result.canceled || result.filePaths.length === 0) {
        return null;
      }
      return result.filePaths[0];
    } catch (error) {
      console.error("Failed to open folder dialog", error);
      throw error;
    }
  });

  ipcMain.handle(
    "track:context-menu",
    async (event, trackPath: string, labels: Record<string, string>) => {
      if (process.env.AUTO_TAGGER_E2E_TRACK_CONTEXT_ACTION === "extra-tags") {
        return "extra-tags";
      }

      const win = resolveWindow(event);
      if (!win) return null;

      return new Promise<"extra-tags" | null>((resolve) => {
        let resolved = false;
        const finish = (action: "extra-tags" | null) => {
          if (resolved) return;
          resolved = true;
          resolve(action);
        };

        const copy = (text: string | undefined) => {
          if (text) clipboard.writeText(text);
          finish(null);
        };

        const menu = Menu.buildFromTemplate([
          { label: "Extra Tags...", click: () => finish("extra-tags") },
          { type: "separator" },
          { label: "Copy Title", click: () => copy(labels.title) },
          { label: "Copy Artist", click: () => copy(labels.artist) },
          { label: "Copy Album Artist", click: () => copy(labels.albumArtist) },
          { label: "Copy Album", click: () => copy(labels.album) },
          { label: "Copy Path", click: () => copy(trackPath) },
          { type: "separator" },
          {
            label: "Copy All Details",
            click: () =>
              copy(
                [
                  `Title: ${labels.title || "-"}`,
                  `Artist: ${labels.artist || "-"}`,
                  `Album Artist: ${labels.albumArtist || "-"}`,
                  `Album: ${labels.album || "-"}`,
                  `Year: ${labels.year || "-"}`,
                  `Track: ${labels.track || "-"}`,
                  `Genre: ${labels.genre || "-"}`,
                  `Path: ${trackPath}`,
                ].join("\n"),
              ),
          },
        ]);

        menu.popup({
          window: win,
          callback: () => finish(null),
        });
      });
    },
  );

  // Auto-tag handlers
  ipcMain.handle("album:auto-tag", async (_event, albumPath: string) => {
    try {
      refreshConfig();
      return startAutoTag(albumPath);
    } catch (error) {
      console.error("Failed to start auto-tag:", error);
      throw error;
    }
  });

  ipcMain.handle("task:progress", async (_event, taskId: string) => {
    try {
      return getProgress(taskId);
    } catch (error) {
      console.error("Failed to get task progress:", error);
      throw error;
    }
  });

  ipcMain.handle("task:cancel", async (_event, taskId: string) => {
    try {
      cancelTask(taskId);
    } catch (error) {
      console.error("Failed to cancel task:", error);
      throw error;
    }
  });

  ipcMain.handle("dataset:status", async () => {
    try {
      return getDatasetStatus();
    } catch (error) {
      console.error("Failed to get dataset status:", error);
      return {
        available: false,
        musicbrainz: false,
        totalRecords: 0,
        lastUpdated: null,
      };
    }
  });

  ipcMain.handle("config:get", async () => {
    try {
      return getConfig();
    } catch (error) {
      console.error("Failed to get config:", error);
      return {};
    }
  });

  ipcMain.handle("config:set", async (_event, key: string, value: unknown) => {
    try {
      saveConfig(key, value);
      refreshConfig();
    } catch (error) {
      console.error("Failed to set config:", error);
    }
  });

  // Debug mode toggle — also saved to config
  ipcMain.handle("debug:set-mode", async (_event, enabled: boolean) => {
    setDebugMode(enabled);
    saveConfig("debug", enabled);
  });

  createWindow();

  app.on("activate", () => {
    if (BrowserWindow.getAllWindows().length === 0) {
      createWindow();
    }
  });
});

app.on("window-all-closed", () => {
  if (process.platform !== "darwin") {
    app.quit();
  }
});
