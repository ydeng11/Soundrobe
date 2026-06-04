/**
 * ConversationLogger — persists AI assistant conversations to SQLite.
 *
 * Records every user message, assistant response, API call, and tool
 * interaction with a human-readable session number for retrieval.
 *
 * Session numbers are time-epoch + random (e.g. "1748823456789-48291").
 * The same cache.db is used as MatchCache — adds a `conversation_log` table.
 */

import { getBetterSqlite3, type BetterSqlite3Database } from "./native-check";
import { mkdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { homedir } from "node:os";

// ── Types ───────────────────────────────────────────────────────────

export type ConversationEntryType =
  | "api_request"
  | "api_response"
  | "user_message"
  | "assistant_message"
  | "tool_call"
  | "tool_result"
  | "system";

export interface ConversationEntry {
  id: number;
  sessionUuid: string;
  /** Epoch+random string, e.g. "1748823456789-48291" */
  sessionNumber: string;
  timestamp: string;
  entryType: ConversationEntryType;
  content: string;
  model: string | null;
  promptTokens: number;
  completionTokens: number;
  totalTokens: number;
  cost: number;
  metadata: string | null;
}

export interface SessionSummary {
  /** Epoch+random string, e.g. "1748823456789-48291" */
  sessionNumber: string;
  sessionUuid: string;
  entryCount: number;
  firstMessage: string | null;
  lastActivity: string;
  apiCallCount: number;
  totalCost: number;
}

function epochRandomId(): string {
  return `${Date.now()}-${Math.floor(Math.random() * 1000000)}`;
}

// ── Interface ───────────────────────────────────────────────────────

export interface IConversationLogger {
  close(): void;
  /** Get or create the epoch+random session number for a session UUID. */
  getOrCreateSessionNumber(sessionUuid: string): string;
  recordEntry(entry: {
    sessionUuid: string;
    entryType: ConversationEntryType;
    content: string;
    model?: string | null;
    promptTokens?: number;
    completionTokens?: number;
    totalTokens?: number;
    cost?: number;
    metadata?: Record<string, unknown> | null;
  }): void;
  recordUserMessage(sessionUuid: string, content: string): void;
  recordAssistantMessage(sessionUuid: string, content: string): void;
  recordApiCall(
    sessionUuid: string,
    request: { messages: Array<{ role: string; content: string }> },
    response: {
      data: Record<string, unknown>;
      model: string;
      promptTokens: number;
      completionTokens: number;
      totalTokens: number;
    },
    cost: number,
    label?: string,
  ): void;
  getConversation(sessionUuidOrNumber: string): ConversationEntry[];
  listSessions(limit?: number): SessionSummary[];
  getSessionSummary(sessionUuidOrNumber: string): SessionSummary | null;
}

// ── Null logger (in-memory no-op, used as default for tests) ───────

export class NullConversationLogger implements IConversationLogger {
  private sessionNumberMap = new Map<string, string>();

  close(): void {
    // no-op
  }

  getOrCreateSessionNumber(sessionUuid: string): string {
    let num = this.sessionNumberMap.get(sessionUuid);
    if (num === undefined) {
      num = epochRandomId();
      this.sessionNumberMap.set(sessionUuid, num);
    }
    return num;
  }

  recordEntry(_entry: {
    sessionUuid: string;
    entryType: ConversationEntryType;
    content: string;
    model?: string | null;
    promptTokens?: number;
    completionTokens?: number;
    totalTokens?: number;
    cost?: number;
    metadata?: Record<string, unknown> | null;
  }): void {
    // no-op
  }

  recordUserMessage(_sessionUuid: string, _content: string): void {
    // no-op
  }

  recordAssistantMessage(_sessionUuid: string, _content: string): void {
    // no-op
  }

  recordApiCall(
    _sessionUuid: string,
    _request: { messages: Array<{ role: string; content: string }> },
    _response: {
      data: Record<string, unknown>;
      model: string;
      promptTokens: number;
      completionTokens: number;
      totalTokens: number;
    },
    _cost: number,
    _label?: string,
  ): void {
    // no-op
  }

  getConversation(_sessionUuidOrNumber: string): ConversationEntry[] {
    return [];
  }

  listSessions(_limit?: number): SessionSummary[] {
    return [];
  }

  getSessionSummary(_sessionUuidOrNumber: string): SessionSummary | null {
    return null;
  }
}

// ── SQLite-backed logger ────────────────────────────────────────────

export class ConversationLogger implements IConversationLogger {
  private db: BetterSqlite3Database;
  private sessionNumberMap = new Map<string, string>();

  constructor(cachePath?: string) {
    const resolvedPath = cachePath ?? join(homedir(), ".auto-tagger", "cache.db");
    try {
      mkdirSync(dirname(resolvedPath), { recursive: true });
    } catch {
      // directory may already exist
    }
    const Database = getBetterSqlite3();
    this.db = new Database(resolvedPath);
    this.db.pragma("journal_mode = WAL");
    this.initSchema();
    this.loadSessionNumbers();
  }

  close(): void {
    this.db.close();
  }

  // ── Schema ────────────────────────────────────────────────────────

  private initSchema(): void {
    this.db.exec(`
      CREATE TABLE IF NOT EXISTS conversation_log (
        id INTEGER PRIMARY KEY AUTOINCREMENT,
        session_uuid TEXT NOT NULL,
        session_number TEXT NOT NULL DEFAULT '',
        timestamp TEXT NOT NULL,
        entry_type TEXT NOT NULL,
        content TEXT NOT NULL DEFAULT '',
        model TEXT,
        prompt_tokens INTEGER NOT NULL DEFAULT 0,
        completion_tokens INTEGER NOT NULL DEFAULT 0,
        total_tokens INTEGER NOT NULL DEFAULT 0,
        cost REAL NOT NULL DEFAULT 0.0,
        metadata TEXT
      );
      CREATE INDEX IF NOT EXISTS idx_cl_session_uuid
        ON conversation_log(session_uuid);
      CREATE INDEX IF NOT EXISTS idx_cl_session_number
        ON conversation_log(session_number);
      CREATE INDEX IF NOT EXISTS idx_cl_session_both
        ON conversation_log(session_uuid, session_number);
      CREATE INDEX IF NOT EXISTS idx_cl_timestamp
        ON conversation_log(timestamp);
    `);
  }

  // ── Session numbering ─────────────────────────────────────────────

  private loadSessionNumbers(): void {
    const rows = this.db
      .prepare(
        `SELECT session_uuid, session_number
         FROM conversation_log
         WHERE session_number != ''
         GROUP BY session_uuid`,
      )
      .all() as Array<{ session_uuid: string; session_number: string }>;

    for (const row of rows) {
      if (!this.sessionNumberMap.has(row.session_uuid)) {
        this.sessionNumberMap.set(row.session_uuid, row.session_number);
      }
    }
  }

  getOrCreateSessionNumber(sessionUuid: string): string {
    const existing = this.sessionNumberMap.get(sessionUuid);
    if (existing) return existing;

    const row = this.db
      .prepare(
        `SELECT session_number FROM conversation_log
         WHERE session_uuid = ? AND session_number != ''
         LIMIT 1`,
      )
      .get(sessionUuid) as { session_number: string } | undefined;

    if (row) {
      this.sessionNumberMap.set(sessionUuid, row.session_number);
      return row.session_number;
    }

    const nextNum = epochRandomId();
    this.sessionNumberMap.set(sessionUuid, nextNum);
    return nextNum;
  }

  // ── Recording ─────────────────────────────────────────────────────

  recordEntry(entry: {
    sessionUuid: string;
    entryType: ConversationEntryType;
    content: string;
    model?: string | null;
    promptTokens?: number;
    completionTokens?: number;
    totalTokens?: number;
    cost?: number;
    metadata?: Record<string, unknown> | null;
  }): void {
    let sessionNumber = this.sessionNumberMap.get(entry.sessionUuid);
    if (!sessionNumber) {
      sessionNumber = this.getOrCreateSessionNumber(entry.sessionUuid);
    }

    this.db
      .prepare(
        `INSERT INTO conversation_log
         (session_uuid, session_number, timestamp, entry_type, content,
          model, prompt_tokens, completion_tokens, total_tokens, cost, metadata)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)`,
      )
      .run(
        entry.sessionUuid,
        sessionNumber,
        new Date().toISOString(),
        entry.entryType,
        entry.content,
        entry.model ?? null,
        entry.promptTokens ?? 0,
        entry.completionTokens ?? 0,
        entry.totalTokens ?? 0,
        entry.cost ?? 0,
        entry.metadata ? JSON.stringify(entry.metadata) : null,
      );
  }

  recordUserMessage(sessionUuid: string, content: string): void {
    this.recordEntry({ sessionUuid, entryType: "user_message", content });
  }

  recordAssistantMessage(sessionUuid: string, content: string): void {
    this.recordEntry({ sessionUuid, entryType: "assistant_message", content });
  }

  recordApiCall(
    sessionUuid: string,
    request: { messages: Array<{ role: string; content: string }> },
    response: {
      data: Record<string, unknown>;
      model: string;
      promptTokens: number;
      completionTokens: number;
      totalTokens: number;
    },
    cost: number,
    label?: string,
  ): void {
    this.recordEntry({
      sessionUuid,
      entryType: "api_request",
      content: JSON.stringify(request.messages),
      metadata: { label: label ?? null, messageCount: request.messages.length },
    });

    this.recordEntry({
      sessionUuid,
      entryType: "api_response",
      content: JSON.stringify(response.data),
      model: response.model,
      promptTokens: response.promptTokens,
      completionTokens: response.completionTokens,
      totalTokens: response.totalTokens,
      cost,
      metadata: { label: label ?? null },
    });
  }

  // ── Querying ──────────────────────────────────────────────────────

  getConversation(sessionUuidOrNumber: string): ConversationEntry[] {
    return this.db
      .prepare(
        `SELECT id, session_uuid as sessionUuid, session_number as sessionNumber,
                timestamp, entry_type as entryType, content,
                model, prompt_tokens as promptTokens,
                completion_tokens as completionTokens,
                total_tokens as totalTokens, cost, metadata
         FROM conversation_log
         WHERE session_uuid = ? OR session_number = ?
         ORDER BY id ASC`,
      )
      .all(sessionUuidOrNumber, sessionUuidOrNumber) as ConversationEntry[];
  }

  listSessions(limit = 50): SessionSummary[] {
    const rows = this.db
      .prepare(
        `SELECT
           session_number as sessionNumber,
           session_uuid as sessionUuid,
           COUNT(*) as entryCount,
           (SELECT content FROM conversation_log cl2
            WHERE cl2.session_uuid = cl.session_uuid
              AND cl2.entry_type = 'user_message'
            ORDER BY cl2.id ASC LIMIT 1) as firstMessage,
           MAX(timestamp) as lastActivity,
           SUM(CASE WHEN entry_type IN ('api_request','api_response') THEN 1 ELSE 0 END) as apiCallCount,
           COALESCE(SUM(cost), 0) as totalCost
         FROM conversation_log cl
         GROUP BY session_uuid, session_number
         ORDER BY MAX(id) DESC
         LIMIT ?`,
      )
      .all(limit) as Array<{
        sessionNumber: string;
        sessionUuid: string;
        entryCount: number;
        firstMessage: string | null;
        lastActivity: string;
        apiCallCount: number;
        totalCost: number;
      }>;

    return rows.map((r) => ({
      sessionNumber: r.sessionNumber,
      sessionUuid: r.sessionUuid,
      entryCount: r.entryCount,
      firstMessage: r.firstMessage?.slice(0, 200) ?? null,
      lastActivity: r.lastActivity,
      apiCallCount: r.apiCallCount,
      totalCost: r.totalCost,
    }));
  }

  getSessionSummary(sessionUuidOrNumber: string): SessionSummary | null {
    const summaries = this.listSessions(1000);
    return summaries.find(
      (s) => s.sessionUuid === sessionUuidOrNumber || s.sessionNumber === sessionUuidOrNumber,
    ) ?? null;
  }

  clearAll(): void {
    this.db.exec("DELETE FROM conversation_log");
    this.sessionNumberMap.clear();
  }
}
