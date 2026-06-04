import { describe, it, expect, vi, beforeEach } from "vitest";
import {
  AssistantRuntime,
  detectToolIntentMismatch,
} from "../../electron/services/AssistantRuntime";
import { AssistantToolRegistry } from "../../electron/services/AssistantToolRegistry";
import { LlmTaskRunner } from "../../electron/services/LlmTaskRunner";

// Mock LlmTaskRunner
vi.mock("../../electron/services/LlmTaskRunner", () => ({
  LlmTaskRunner: vi.fn().mockImplementation(() => ({
    runToolLoop: vi.fn().mockResolvedValue({
      finalMessage: "Hello!",
      steps: [{ type: "message", content: "Hello!" }],
      usage: { promptTokens: 10, completionTokens: 5, totalTokens: 15 },
      model: "test-model",
      cost: 0.001,
      costFormatted: "$0.0010",
      stoppedEarly: false,
    }),
    runStructuredTask: vi.fn(),
    onApiCall: vi.fn().mockReturnValue(vi.fn()),
  })),
  redactSensitive: (text: string) => text,
}));

function makeRegistry(): AssistantToolRegistry {
  const reg = new AssistantToolRegistry();
  reg.register({
    name: "library.summarize",
    description: "Summarize library",
    inputSchema: { type: "object", properties: {}, required: [] },
    executor: vi.fn().mockResolvedValue({ ok: true, summary: "Library has 10 tracks" }),
    isReadOnly: true,
  });
  return reg;
}

function makeRuntime(
  reg: AssistantToolRegistry = makeRegistry(),
  autonomous = false,
): AssistantRuntime {
  const runner = new (LlmTaskRunner as any)({ apiKey: "test-key" });
  return new AssistantRuntime(runner, reg, autonomous);
}

describe("AssistantRuntime", () => {
  it("generates a session ID", () => {
    const runtime = makeRuntime();
    expect(runtime.getSessionId()).toMatch(/^session-/);
  });

  it("toggles autonomous mode", () => {
    const runtime = makeRuntime();
    expect(runtime.isAutonomous()).toBe(false);
    runtime.setAutonomous(true);
    expect(runtime.isAutonomous()).toBe(true);
  });

  it("creates and manages action batches", () => {
    const runtime = makeRuntime();
    const batch = runtime.createActionBatch({
      kind: "tag-update",
      title: "Update genre",
      summary: "Change genre on 1 track",
      riskLevel: "low",
      actions: [{ trackPath: "/test.flac", field: "genre", oldValue: "Rock", newValue: "Pop" }],
      reversible: true,
    });

    expect(batch.id).toContain("batch-");
    expect(batch.status).toBe("pending");
    expect(runtime.getActionBatch(batch.id)).toBe(batch);
    expect(runtime.getPendingBatches()).toHaveLength(1);
  });

  it("marks batches as applied", () => {
    const runtime = makeRuntime();
    const batch = runtime.createActionBatch({
      kind: "tag-update",
      title: "Update",
      summary: "Summary",
      riskLevel: "low",
      actions: [],
      reversible: true,
    });
    runtime.markBatchApplied(batch.id);
    expect(batch.status).toBe("applied");
    expect(runtime.getPendingBatches()).toHaveLength(0);
  });

  it("marks batches as rejected", () => {
    const runtime = makeRuntime();
    const batch = runtime.createActionBatch({
      kind: "tag-update",
      title: "Update",
      summary: "Summary",
      riskLevel: "low",
      actions: [],
      reversible: true,
    });
    runtime.markBatchRejected(batch.id);
    expect(batch.status).toBe("rejected");
  });

  it("marks batches as failed", () => {
    const runtime = makeRuntime();
    const batch = runtime.createActionBatch({
      kind: "tag-update",
      title: "Update",
      summary: "Summary",
      riskLevel: "low",
      actions: [],
      reversible: true,
    });
    runtime.markBatchFailed(batch.id, "Something broke");
    expect(batch.status).toBe("failed");
  });

  it("manages conversation history", () => {
    const runtime = makeRuntime();
    runtime.addUserMessage("Hello");
    runtime.addAssistantMessage("Hi there!");
    expect(runtime.getSessionId()).toBeDefined();
    runtime.clearConversation();
    // Should not throw
  });

  it("notifies event subscribers", async () => {
    const runtime = makeRuntime();
    const events: string[] = [];
    runtime.onEvent((event) => events.push(event.type));

    const batch = runtime.createActionBatch({
      kind: "tag-update",
      title: "Update",
      summary: "Summary",
      riskLevel: "low",
      actions: [],
      reversible: true,
    });
    runtime.markBatchApplied(batch.id);

    expect(events).toContain("action_batch_applied");
  });

  it("allows unsubscribing from events", async () => {
    const runtime = makeRuntime();
    const events: string[] = [];
    const unsub = runtime.onEvent((event) => events.push(event.type));
    unsub();

    const batch = runtime.createActionBatch({
      kind: "tag-update",
      title: "Update",
      summary: "Summary",
      riskLevel: "low",
      actions: [],
      reversible: true,
    });
    runtime.markBatchApplied(batch.id);

    expect(events).toHaveLength(0);
  });

  it("cancels the session", () => {
    const runtime = makeRuntime();
    runtime.cancel();
    // Should not throw
  });

  it("generates a session number in epoch-random format", () => {
    const runtime = makeRuntime();
    const num = runtime.getSessionNumber();
    expect(num).toMatch(/^\d+-\d+$/);
  });

  it("returns the same session number on repeated calls", () => {
    const runtime = makeRuntime();
    const n1 = runtime.getSessionNumber();
    const n2 = runtime.getSessionNumber();
    expect(n1).toBe(n2);
  });

  it("resetSession clears conversation and generates a new session ID", () => {
    const runtime = makeRuntime();
    const originalId = runtime.getSessionId();
    const originalNum = runtime.getSessionNumber();

    runtime.addUserMessage("Hello");
    runtime.addAssistantMessage("Hi");
    runtime.resetSession();

    // Session ID changed
    expect(runtime.getSessionId()).not.toBe(originalId);
    expect(runtime.getSessionId()).toMatch(/^session-/);

    // Session number changed (new epochs+random)
    expect(runtime.getSessionNumber()).not.toBe(originalNum);

    // Conversation cleared (new message should be isolated)
    runtime.addUserMessage("Fresh start");
    // No throw
  });

  it("dispose cleans up without throwing", () => {
    const runtime = makeRuntime();
    expect(() => runtime.dispose()).not.toThrow();
  });

  it("getConversationLogger returns a logger", () => {
    const runtime = makeRuntime();
    const logger = runtime.getConversationLogger();
    expect(logger).toBeDefined();
    expect(typeof logger.getOrCreateSessionNumber).toBe("function");
  });

  it("blocks file-moving tools when the user is correcting track-number metadata", async () => {
    const moveExecutor = vi.fn().mockResolvedValue({ ok: true, summary: "Move files" });
    const numberingExecutor = vi.fn().mockResolvedValue({
      ok: true,
      summary: "Preview created: Number tracks",
      pendingActionBatchId: "batch-numbering",
    });
    const reg = new AssistantToolRegistry();
    reg.register({
      name: "group_by_album",
      description: "Move tracks into album folders",
      inputSchema: { type: "object", properties: {}, required: [] },
      executor: moveExecutor,
      isReadOnly: false,
      riskLevel: "medium",
      operationKind: "file_move",
    });
    reg.register({
      name: "auto_numbering_tracks",
      description: "Fix track number metadata",
      inputSchema: { type: "object", properties: {}, required: [] },
      executor: numberingExecutor,
      isReadOnly: false,
      riskLevel: "low",
      operationKind: "metadata_edit",
    });

    const runner = {
      onApiCall: vi.fn().mockReturnValue(vi.fn()),
      runToolLoop: vi.fn()
        .mockResolvedValueOnce({
          steps: [{
            type: "tool_call",
            content: "I will group by album",
            toolName: "group_by_album",
            toolArgs: {},
          }],
        })
        .mockResolvedValueOnce({
          steps: [{
            type: "tool_call",
            content: "I will renumber metadata",
            toolName: "auto_numbering_tracks",
            toolArgs: {},
          }],
        }),
    };
    const runtime = new AssistantRuntime(runner as any, reg, false);

    const result = await runtime.send("no. the track should numbering within the album");

    expect(moveExecutor).not.toHaveBeenCalled();
    expect(numberingExecutor).toHaveBeenCalledOnce();
    expect(result.type).toBe("action_batch_created");
  });
});

describe("detectToolIntentMismatch", () => {
  it("flags file moves for track-number correction intent", () => {
    const mismatch = detectToolIntentMismatch({
      userMessage: "no. the track should numbering within the album",
      toolName: "group_by_album",
      operationKind: "file_move",
    });

    expect(mismatch?.expectedOperationKind).toBe("metadata_edit");
    expect(mismatch?.summary).toContain("auto_numbering_tracks");
  });

  it("allows explicit folder organization requests", () => {
    const mismatch = detectToolIntentMismatch({
      userMessage: "move these files into album folders",
      toolName: "group_by_album",
      operationKind: "file_move",
    });

    expect(mismatch).toBeNull();
  });
});
