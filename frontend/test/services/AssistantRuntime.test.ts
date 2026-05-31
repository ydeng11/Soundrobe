import { describe, it, expect, vi, beforeEach } from "vitest";
import { AssistantRuntime } from "../../electron/services/AssistantRuntime";
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
    getClient: vi.fn(),
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
});
