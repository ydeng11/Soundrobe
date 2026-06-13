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

    it("converts Error objects in data to a plain object via subscriber", () => {
      DebugLogger.setEnabled(true);

      const subscriber = vi.fn();
      const unsub = DebugLogger.subscribe(subscriber);

      const testError = new Error("something broke");
      testError.name = "TypeError";
      DebugLogger.warn("cover", "findDiscogs: threw", testError);

      expect(subscriber).toHaveBeenCalledOnce();
      const entry = subscriber.mock.calls[0][0];

      // The data should be a plain object, not an Error instance
      expect(entry.data).not.toBeInstanceOf(Error);
      expect(entry.data).toHaveProperty("name", "TypeError");
      expect(entry.data).toHaveProperty("message", "something broke");
      expect(entry.data).toHaveProperty("stack");
      expect(entry.data.stack).toContain("something broke");
      expect(entry.tag).toBe("cover");
      expect(entry.level).toBe("warn");
      expect(entry.message).toBe("findDiscogs: threw");

      // Verify JSON serialization works (the file-output path)
      const serialized = JSON.stringify(entry);
      const parsed = JSON.parse(serialized);
      expect(parsed.data.name).toBe("TypeError");
      expect(parsed.data.message).toBe("something broke");
      expect(parsed.data.stack).toContain("something broke");

      unsub();
      DebugLogger.setEnabled(false);
    });

    it("converts Error cause chain in data", () => {
      DebugLogger.setEnabled(true);

      const subscriber = vi.fn();
      const unsub = DebugLogger.subscribe(subscriber);

      const inner = new Error("inner failure");
      const outer = new Error("wrapped", { cause: inner });
      DebugLogger.error("auto-tag", "pipeline failed", outer);

      const entry = subscriber.mock.calls[0][0];
      expect(entry.data.name).toBe("Error");
      expect(entry.data.message).toBe("wrapped");
      expect(entry.data.cause).toBeDefined();
      expect(entry.data.cause.message).toBe("inner failure");

      // Verify JSON serialization preserves the cause chain
      const parsed = JSON.parse(JSON.stringify(entry));
      expect(parsed.data.cause.message).toBe("inner failure");

      unsub();
      DebugLogger.setEnabled(false);
    });

    it("leaves plain objects in data unchanged", () => {
      DebugLogger.setEnabled(true);

      const subscriber = vi.fn();
      const unsub = DebugLogger.subscribe(subscriber);

      DebugLogger.info("test", "plain data", { status: 200, ok: true });

      const entry = subscriber.mock.calls[0][0];
      expect(entry.data).toEqual({ status: 200, ok: true });

      unsub();
      DebugLogger.setEnabled(false);
    });

    it("handles null and undefined data", () => {
      DebugLogger.setEnabled(true);

      const subscriber = vi.fn();
      const unsub = DebugLogger.subscribe(subscriber);

      DebugLogger.info("test", "null data", null);
      const entryNull = subscriber.mock.calls[0][0];
      expect(entryNull.data).toBeNull();

      DebugLogger.info("test", "undefined data", undefined);
      const entryUndef = subscriber.mock.calls[1][0];
      expect(entryUndef.data).toBeUndefined();

      unsub();
      DebugLogger.setEnabled(false);
    });

    it("passes converted Error data to subscribers (data is not Error instance)", () => {
      DebugLogger.setEnabled(true);

      const subscriber = vi.fn();
      const unsub = DebugLogger.subscribe(subscriber);

      const testError = new Error("subscriber sees me");
      DebugLogger.warn("test", "error data", testError);

      const entry = subscriber.mock.calls[0][0];
      expect(entry.data).not.toBeInstanceOf(Error);
      expect(entry.data).toHaveProperty("message", "subscriber sees me");
      expect(entry.data).toHaveProperty("name", "Error");
      expect(entry.data).toHaveProperty("stack");

      unsub();
      DebugLogger.setEnabled(false);
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
