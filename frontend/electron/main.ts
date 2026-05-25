import { app, BrowserWindow, ipcMain, dialog } from "electron";
import path from "path";
import { fileURLToPath } from "url";
import { registerLibraryHandlers } from "./handlers/library";
import { registerTrackHandlers } from "./handlers/tracks";
import { registerCoverHandlers } from "./handlers/cover";
import { registerDirectoryHandlers } from "./handlers/directory";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

const isDev = !app.isPackaged;

let mainWindow: BrowserWindow | null = null;

function createWindow() {
  mainWindow = new BrowserWindow({
    width: 1200,
    height: 800,
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
    show: false,
  });

  mainWindow.once("ready-to-show", () => {
    mainWindow?.show();
  });

  if (isDev) {
    const devServerUrl =
      process.env.VITE_DEV_SERVER_URL ?? "http://localhost:5173";
    mainWindow.loadURL(devServerUrl);
    mainWindow.webContents.openDevTools({ mode: "detach" });

    // Suppress benign DevTools protocol errors (version mismatch noise)
    mainWindow.webContents.on("console-message", (event, level, message, line, sourceId) => {
      if (sourceId?.startsWith("devtools://") && message.includes("Autofill")) {
        event.preventDefault();
      }
    });
  } else {
    mainWindow.loadFile(path.join(__dirname, "../dist/index.html"));
  }

  mainWindow.on("closed", () => {
    mainWindow = null;
  });
}

app.whenReady().then(() => {
  // Register all IPC handlers
  registerLibraryHandlers();
  registerTrackHandlers();
  registerCoverHandlers();
  registerDirectoryHandlers();

  // Native folder picker — use event.sender to get the invoking window, with a
  // mainWindow fallback for stale/reloaded dev renderers.
  ipcMain.handle("dialog:open-folder", async (event) => {
    try {
      const senderWindow = BrowserWindow.fromWebContents(event.sender);
      const win =
        senderWindow && !senderWindow.isDestroyed() ? senderWindow : mainWindow;
      if (!win || win.isDestroyed()) return null;

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

  // Stub handlers for Phase 3 (auto-tag)
  ipcMain.handle("album:auto-tag", async (_event, _albumPath: string) => {
    throw new Error("Auto-tag not yet implemented (Phase 3)");
  });

  ipcMain.handle("task:progress", async (_event, _taskId: string) => {
    throw new Error("Task polling not yet implemented (Phase 3)");
  });

  ipcMain.handle("task:cancel", async (_event, _taskId: string) => {
    throw new Error("Task cancellation not yet implemented (Phase 3)");
  });

  ipcMain.handle("dataset:status", async () => {
    return {
      musicbrainz: false,
      spotify: false,
      totalRecords: 0,
      lastUpdated: null,
    };
  });

  ipcMain.handle("config:get", async () => ({}));
  ipcMain.handle("config:set", async (_event, _key: string, _value: unknown) => {});

  ipcMain.handle("window:focused", async () => {});

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
