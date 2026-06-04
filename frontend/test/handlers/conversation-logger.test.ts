/**
 * Tests for ConversationLogger and NullConversationLogger.
 *
 * ConversationLogger tests use a temp SQLite DB (via better-sqlite3).
 * These tests require the native module to be compiled for the host Node,
 * so they may be skipped in CI environments.
 */

import { describe, it, expect } from "vitest";
import { NullConversationLogger, ConversationLogger } from "../../electron/handlers/conversation-logger";
import { mkdtempSync, rmSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";

const SESSION_NUM_RE = /^\d+-\d+$/;

// ── NullConversationLogger ─────────────────────────────────────────

describe("NullConversationLogger", () => {
  it("returns epoch+random session numbers", () => {
    const logger = new NullConversationLogger();
    expect(logger.getOrCreateSessionNumber("session-a")).toMatch(SESSION_NUM_RE);
    expect(logger.getOrCreateSessionNumber("session-b")).toMatch(SESSION_NUM_RE);
  });

  it("returns the same number for the same session", () => {
    const logger = new NullConversationLogger();
    const n1 = logger.getOrCreateSessionNumber("session-x");
    const n2 = logger.getOrCreateSessionNumber("session-x");
    expect(n1).toBe(n2);
  });

  it("returns empty list for getConversation", () => {
    const logger = new NullConversationLogger();
    expect(logger.getConversation("session-a")).toEqual([]);
    expect(logger.getConversation("1")).toEqual([]);
  });

  it("returns empty list for listSessions", () => {
    const logger = new NullConversationLogger();
    expect(logger.listSessions()).toEqual([]);
  });

  it("returns null for getSessionSummary", () => {
    const logger = new NullConversationLogger();
    expect(logger.getSessionSummary("session-a")).toBeNull();
  });

  it("all no-op methods do not throw", () => {
    const logger = new NullConversationLogger();
    expect(() => {
      logger.recordEntry({ sessionUuid: "s", entryType: "user_message", content: "hi" });
      logger.recordUserMessage("s", "hi");
      logger.recordAssistantMessage("s", "hello");
      logger.recordApiCall(
        "s",
        { messages: [{ role: "user", content: "hi" }] },
        { data: {}, model: "test", promptTokens: 0, completionTokens: 0, totalTokens: 0 },
        0,
      );
      logger.close();
    }).not.toThrow();
  });
});

// ── ConversationLogger (SQLite-backed) ────────────────────────────

function tryCreateLogger(): ConversationLogger | null {
  try {
    const tmpDir = mkdtempSync(join(tmpdir(), "conv-log-test-"));
    const logger = new ConversationLogger(join(tmpDir, "test.db"));
    return logger;
  } catch {
    return null;
  }
}

describe.runIf(tryCreateLogger() !== null)("ConversationLogger", () => {
  it("assigns epoch+random session numbers", () => {
    const tmpDir = mkdtempSync(join(tmpdir(), "conv-log-test-"));
    const logger = new ConversationLogger(join(tmpDir, "test.db"));

    const n1 = logger.getOrCreateSessionNumber("session-1");
    const n2 = logger.getOrCreateSessionNumber("session-2");
    const n3 = logger.getOrCreateSessionNumber("session-3");

    expect(n1).toMatch(SESSION_NUM_RE);
    expect(n2).toMatch(SESSION_NUM_RE);
    expect(n3).toMatch(SESSION_NUM_RE);
    // Different sessions should have different numbers
    expect(n1).not.toBe(n2);
    expect(n2).not.toBe(n3);

    logger.close();
    rmSync(tmpDir, { recursive: true, force: true });
  });

  it("returns the same number for repeated calls", () => {
    const tmpDir = mkdtempSync(join(tmpdir(), "conv-log-test-"));
    const logger = new ConversationLogger(join(tmpDir, "test.db"));

    const n1 = logger.getOrCreateSessionNumber("session-x");
    const n2 = logger.getOrCreateSessionNumber("session-x");
    expect(n1).toBe(n2);

    logger.close();
    rmSync(tmpDir, { recursive: true, force: true });
  });

  it("records and retrieves user and assistant messages", () => {
    const tmpDir = mkdtempSync(join(tmpdir(), "conv-log-test-"));
    const logger = new ConversationLogger(join(tmpDir, "test.db"));

    const session = "session-msg-test";
    logger.recordUserMessage(session, "Hello");
    logger.recordAssistantMessage(session, "Hi there!");

    const entries = logger.getConversation(session);
    expect(entries).toHaveLength(2);
    expect(entries[0].entryType).toBe("user_message");
    expect(entries[0].content).toBe("Hello");
    expect(entries[1].entryType).toBe("assistant_message");
    expect(entries[1].content).toBe("Hi there!");

    logger.close();
    rmSync(tmpDir, { recursive: true, force: true });
  });

  it("records and retrieves API calls", () => {
    const tmpDir = mkdtempSync(join(tmpdir(), "conv-log-test-"));
    const logger = new ConversationLogger(join(tmpDir, "test.db"));

    const session = "session-api-test";
    logger.recordApiCall(
      session,
      { messages: [{ role: "user", content: "search for album" }] },
      { data: { result: "found" }, model: "test-model", promptTokens: 10, completionTokens: 5, totalTokens: 15 },
      0.001,
      "tool_loop",
    );

    const entries = logger.getConversation(session);
    expect(entries).toHaveLength(2);
    expect(entries[0].entryType).toBe("api_request");
    expect(entries[0].content).toContain("search for album");
    expect(entries[1].entryType).toBe("api_response");
    expect(entries[1].model).toBe("test-model");
    expect(entries[1].promptTokens).toBe(10);
    expect(entries[1].completionTokens).toBe(5);
    expect(entries[1].cost).toBeCloseTo(0.001, 5);

    logger.close();
    rmSync(tmpDir, { recursive: true, force: true });
  });

  it("retrieves by session number string", () => {
    const tmpDir = mkdtempSync(join(tmpdir(), "conv-log-test-"));
    const logger = new ConversationLogger(join(tmpDir, "test.db"));

    const session = "session-retrieve-test";
    logger.recordUserMessage(session, "Test");

    const entriesByUuid = logger.getConversation(session);
    expect(entriesByUuid).toHaveLength(1);
    const sessionNumber = entriesByUuid[0].sessionNumber;

    expect(sessionNumber).toMatch(SESSION_NUM_RE);

    const entriesByNum = logger.getConversation(sessionNumber);
    expect(entriesByNum).toEqual(entriesByUuid);

    logger.close();
    rmSync(tmpDir, { recursive: true, force: true });
  });

  it("lists sessions with summary", () => {
    const tmpDir = mkdtempSync(join(tmpdir(), "conv-log-test-"));
    const logger = new ConversationLogger(join(tmpDir, "test.db"));

    logger.recordUserMessage("session-a", "First message A");
    logger.recordUserMessage("session-b", "First message B");

    const sessions = logger.listSessions();
    expect(sessions.length).toBeGreaterThanOrEqual(2);

    const sessionA = sessions.find((s) => s.sessionUuid === "session-a");
    expect(sessionA).toBeDefined();
    expect(sessionA!.firstMessage).toContain("First message A");
    expect(sessionA!.sessionNumber).toMatch(SESSION_NUM_RE);

    const sessionB = sessions.find((s) => s.sessionUuid === "session-b");
    expect(sessionB).toBeDefined();
    expect(sessionB!.firstMessage).toContain("First message B");

    logger.close();
    rmSync(tmpDir, { recursive: true, force: true });
  });

  it("gets session summary by number or uuid", () => {
    const tmpDir = mkdtempSync(join(tmpdir(), "conv-log-test-"));
    const logger = new ConversationLogger(join(tmpDir, "test.db"));

    logger.recordUserMessage("session-summary", "Hello");

    const entries = logger.getConversation("session-summary");
    const sn = entries[0].sessionNumber;

    const byNum = logger.getSessionSummary(sn);
    expect(byNum).not.toBeNull();
    expect(byNum!.sessionNumber).toBe(sn);

    const byUuid = logger.getSessionSummary("session-summary");
    expect(byUuid).not.toBeNull();
    expect(byUuid!.sessionUuid).toBe("session-summary");

    logger.close();
    rmSync(tmpDir, { recursive: true, force: true });
  });

  it("persists across logger instances within same DB", () => {
    const tmpDir = mkdtempSync(join(tmpdir(), "conv-log-test-"));
    const dbPath = join(tmpDir, "test.db");

    const logger1 = new ConversationLogger(dbPath);
    logger1.recordUserMessage("session-persist", "Hello from instance 1");
    const entries1 = logger1.getConversation("session-persist");
    const sn = entries1[0].sessionNumber;
    expect(sn).toMatch(SESSION_NUM_RE);
    logger1.close();

    const logger2 = new ConversationLogger(dbPath);
    const entries2 = logger2.getConversation(sn);
    expect(entries2).toHaveLength(1);
    expect(entries2[0].content).toBe("Hello from instance 1");

    // Session number should be the same across restarts
    const num2 = logger2.getOrCreateSessionNumber("session-persist");
    expect(num2).toBe(sn);
    logger2.close();

    rmSync(tmpDir, { recursive: true, force: true });
  });

  it("clears all entries", () => {
    const tmpDir = mkdtempSync(join(tmpdir(), "conv-log-test-"));
    const logger = new ConversationLogger(join(tmpDir, "test.db"));

    logger.recordUserMessage("session-clear", "Before clear");
    expect(logger.getConversation("session-clear")).toHaveLength(1);

    logger.clearAll();
    expect(logger.getConversation("session-clear")).toHaveLength(0);

    logger.close();
    rmSync(tmpDir, { recursive: true, force: true });
  });
});
