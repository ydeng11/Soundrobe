// @vitest-environment node
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

// Mock the "electron" module before importing the module under test
vi.mock("electron", () => ({
  ipcMain: {
    handle: vi.fn(),
  },
  BrowserWindow: {
    getAllWindows: vi.fn(() => []),
  },
}));

let DebugLogger: any;

beforeEach(async () => {
  vi.resetModules();
  const mod = await import("../../electron/handlers/debug");
  DebugLogger = mod.default;
});

describe("DebugLogger", () => {
  describe("setEnabled / getEnabled", () => {
    it("is disabled by default", () => {
      expect(DebugLogger.getEnabled()).toBe(false);
    });

    it("enables and disables", () => {
      DebugLogger.setEnabled(true);
      expect(DebugLogger.getEnabled()).toBe(true);

      DebugLogger.setEnabled(false);
      expect(DebugLogger.getEnabled()).toBe(false);
    });

    it("sets a log file path when enabled", () => {
      DebugLogger.setEnabled(true);
      const logPath = DebugLogger.getLogFilePath();
      expect(logPath).toBeTruthy();
      expect(logPath).toContain("auto-tag-debug");
      DebugLogger.setEnabled(false);
    });

    it("returns null log path when disabled", () => {
      expect(DebugLogger.getLogFilePath()).toBeNull();
    });
  });

  describe("logging methods", () => {
    beforeEach(() => {
      DebugLogger.setEnabled(true);
    });

    afterEach(() => {
      DebugLogger.setEnabled(false);
    });

    it("info logs with timestamp and tag", () => {
      const spy = vi.spyOn(console, "log").mockImplementation(() => {});
      DebugLogger.info("test", "hello from test");
      expect(spy).toHaveBeenCalledOnce();
      const msg = spy.mock.calls[0][0];
      expect(msg).toContain("[test");
      expect(msg).toContain("INFO");
      expect(msg).toContain("hello from test");
      spy.mockRestore();
    });

    it("warn logs to console.warn", () => {
      const spy = vi.spyOn(console, "warn").mockImplementation(() => {});
      DebugLogger.warn("test", "warning message");
      expect(spy).toHaveBeenCalledOnce();
      expect(spy.mock.calls[0][0]).toContain("WARN");
      expect(spy.mock.calls[0][0]).toContain("warning message");
      spy.mockRestore();
    });

    it("error logs to console.error", () => {
      const spy = vi.spyOn(console, "error").mockImplementation(() => {});
      DebugLogger.error("test", "error message");
      expect(spy).toHaveBeenCalledOnce();
      expect(spy.mock.calls[0][0]).toContain("ERROR");
      expect(spy.mock.calls[0][0]).toContain("error message");
      spy.mockRestore();
    });

    it("debug logs to console.log", () => {
      const spy = vi.spyOn(console, "log").mockImplementation(() => {});
      DebugLogger.debug("test", "debug message");
      expect(spy).toHaveBeenCalledOnce();
      expect(spy.mock.calls[0][0]).toContain("DEBUG");
      expect(spy.mock.calls[0][0]).toContain("debug message");
      spy.mockRestore();
    });

    it("does not log when disabled", () => {
      DebugLogger.setEnabled(false);
      const spy = vi.spyOn(console, "log").mockImplementation(() => {});
      DebugLogger.info("test", "should not appear");
      expect(spy).not.toHaveBeenCalled();
      spy.mockRestore();
    });

    it("logs with additional data", () => {
      const spy = vi.spyOn(console, "log").mockImplementation(() => {});
      DebugLogger.info("test", "with data", { foo: "bar" });
      expect(spy.mock.calls[0][1]).toEqual({ foo: "bar" });
      spy.mockRestore();
    });
  });

  describe("timing", () => {
    it("startTimer / endTimer logs elapsed time", () => {
      DebugLogger.setEnabled(true);
      const spy = vi.spyOn(console, "log").mockImplementation(() => {});

      DebugLogger.startTimer("op1");
      DebugLogger.endTimer("op1", "test-tag", "Operation 1");

      expect(spy).toHaveBeenCalledOnce();
      expect(spy.mock.calls[0][0]).toContain("Operation 1");
      expect(spy.mock.calls[0][0]).toContain("ms");

      DebugLogger.setEnabled(false);
      spy.mockRestore();
    });

    it("endTimer returns 0 for unknown timer", () => {
      const elapsed = DebugLogger.endTimer("nonexistent");
      expect(elapsed).toBe(0);
    });
  });

  describe("subscriber notification", () => {
    it("notifies subscribers of log entries", () => {
      DebugLogger.setEnabled(true);
      const subscriber = vi.fn();
      const unsub = DebugLogger.subscribe(subscriber);

      DebugLogger.info("test", "msg");

      expect(subscriber).toHaveBeenCalledOnce();
      const entry = subscriber.mock.calls[0][0];
      expect(entry).toHaveProperty("timestamp");
      expect(entry.tag).toBe("test");
      expect(entry.level).toBe("info");
      expect(entry.message).toBe("msg");

      unsub();
      DebugLogger.setEnabled(false);
    });

    it("does not notify after unsubscribe", () => {
      DebugLogger.setEnabled(true);
      const subscriber = vi.fn();
      const unsub = DebugLogger.subscribe(subscriber);
      unsub();

      DebugLogger.info("test", "after unsub");
      expect(subscriber).not.toHaveBeenCalled();
      DebugLogger.setEnabled(false);
    });

    it("subscriber errors do not crash the logger", () => {
      DebugLogger.setEnabled(true);
      const subscriber = vi.fn(() => {
        throw new Error("subscriber error");
      });
      DebugLogger.subscribe(subscriber);

      expect(() => DebugLogger.info("test", "boom")).not.toThrow();
      DebugLogger.setEnabled(false);
    });
  });
});

describe("registerDebugIpc", () => {
  it("registers IPC handlers", async () => {
    vi.resetModules();
    const { registerDebugIpc } = await import("../../electron/handlers/debug");
    const { ipcMain } = await import("electron");

    registerDebugIpc();

    expect(ipcMain.handle).toHaveBeenCalledTimes(3);
    expect(ipcMain.handle).toHaveBeenCalledWith(
      "debug:subscribe",
      expect.any(Function),
    );
    expect(ipcMain.handle).toHaveBeenCalledWith(
      "debug:status",
      expect.any(Function),
    );
    expect(ipcMain.handle).toHaveBeenCalledWith(
      "debug:toggle",
      expect.any(Function),
    );
  });
});

describe("forwardRendererLog", () => {
  it("forwards log entries from renderer", async () => {
    vi.resetModules();
    const mod = await import("../../electron/handlers/debug");

    mod.default.setEnabled(true);
    const spy = vi.spyOn(console, "log").mockImplementation(() => {});

    mod.forwardRendererLog({
      timestamp: "now",
      tag: "renderer",
      level: "info",
      message: "from renderer",
    });

    expect(spy).toHaveBeenCalledOnce();
    expect(spy.mock.calls[0][0]).toContain("from renderer");

    mod.default.setEnabled(false);
    spy.mockRestore();
  });

  it("forwards error level entries", async () => {
    vi.resetModules();
    const mod = await import("../../electron/handlers/debug");

    mod.default.setEnabled(true);
    const spy = vi.spyOn(console, "error").mockImplementation(() => {});

    mod.forwardRendererLog({
      timestamp: "now",
      tag: "renderer",
      level: "error",
      message: "renderer error",
    });

    expect(spy).toHaveBeenCalledOnce();
    expect(spy.mock.calls[0][0]).toContain("renderer error");

    mod.default.setEnabled(false);
    spy.mockRestore();
  });
});
