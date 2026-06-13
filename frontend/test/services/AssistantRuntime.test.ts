import { describe, it, expect, vi, beforeEach } from "vitest";
import {
  AssistantRuntime,
  deriveAssistantTaskContract,
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

function registerPreviewTool(
  runtime: AssistantRuntime,
  reg: AssistantToolRegistry,
  name: string,
  schema: Record<string, unknown>,
): ReturnType<typeof vi.fn> {
  const executor = vi.fn().mockImplementation(async (args: Record<string, unknown>) => {
    const batch = runtime.createActionBatch({
      kind: name === "run_library_task" ? "auto-tag-run" : "metadata-update",
      title: `Preview ${name}`,
      summary: `Preview ${name}`,
      riskLevel: "low",
      actions: [{ description: JSON.stringify(args) }],
      reversible: true,
    });
    return {
      ok: true,
      summary: `Preview created (${batch.id}): ${name}`,
      pendingActionBatchId: batch.id,
      data: { batch, args },
    };
  });
  reg.register({
    name,
    description: `Preview ${name}`,
    inputSchema: schema,
    executor,
    isReadOnly: false,
    riskLevel: "low",
    operationKind: "metadata_edit",
  });
  return executor;
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

  it("routes 'number' directly to auto_numbering_tracks without an LLM call", async () => {
    const reg = new AssistantToolRegistry();
    const runner = {
      onApiCall: vi.fn().mockReturnValue(vi.fn()),
      runToolLoop: vi.fn(),
    };
    const runtime = new AssistantRuntime(runner as any, reg, false);
    const executor = registerPreviewTool(runtime, reg, "auto_numbering_tracks", {
      type: "object",
      properties: {
        target_scope: { type: "string", enum: ["library"] },
      },
      required: ["target_scope"],
    });

    const result = await runtime.send("number");

    expect(result.type).toBe("action_batch_created");
    expect(executor).toHaveBeenCalledWith({ target_scope: "library" });
    expect(runner.runToolLoop).not.toHaveBeenCalled();
  });

  it("routes auto-tag requests directly to run_library_task without an LLM call", async () => {
    const reg = new AssistantToolRegistry();
    const runner = {
      onApiCall: vi.fn().mockReturnValue(vi.fn()),
      runToolLoop: vi.fn(),
    };
    const runtime = new AssistantRuntime(runner as any, reg, false);
    const executor = registerPreviewTool(runtime, reg, "run_library_task", {
      type: "object",
      properties: {
        task: { type: "string", enum: ["auto_tag"] },
        target_scope: { type: "string", enum: ["library"] },
      },
      required: ["task", "target_scope"],
    });

    const result = await runtime.send("auto tag this");

    expect(result.type).toBe("action_batch_created");
    expect(executor).toHaveBeenCalledWith({ task: "auto_tag", target_scope: "library" });
    expect(runner.runToolLoop).not.toHaveBeenCalled();
  });

  it("does not route album prefix cleanup to extract_tag_value (albums don't have leading numbers)", async () => {
    const reg = new AssistantToolRegistry();
    const runner = {
      onApiCall: vi.fn().mockReturnValue(vi.fn()),
      runToolLoop: vi.fn().mockResolvedValue({
        steps: [{ type: "message", content: "Album values don't start with numbers, so there's nothing to strip." }],
      }),
    };
    const runtime = new AssistantRuntime(runner as any, reg, false);
    const executor = registerPreviewTool(runtime, reg, "extract_tag_value", {
      type: "object",
      properties: {
        target_scope: { type: "string", enum: ["library"] },
        field: { type: "string" },
        pattern: { type: "string" },
        group_index: { type: "number" },
      },
      required: ["target_scope", "field", "pattern"],
    });

    // Album prefix cleanup should not hard-route to extract_tag_value
    // because albums never have leading track numbers.
    // Falls through to general_action_intent → LLM handles it.
    // Falls through to general_action_intent → LLM must produce a preview.
    // The mock LLM returns only a message, so the runtime emits an error.
    const result = await runtime.send("keep album name in Album tag only. Remove the prefix - number and dash.");

    expect(result.type).toBe("error");
    expect(result.message).toContain("No action was performed");
    expect(executor).not.toHaveBeenCalled();
    expect(runner.runToolLoop).toHaveBeenCalled();
  });

  it("allows vague chat to complete as a normal message", async () => {
    const reg = makeRegistry();
    const runner = {
      onApiCall: vi.fn().mockReturnValue(vi.fn()),
      runToolLoop: vi.fn().mockResolvedValue({
        stoppedEarly: false,
        steps: [{ type: "message", content: "Hello!" }],
      }),
    };
    const runtime = new AssistantRuntime(runner as any, reg, false);

    const result = await runtime.send("hello");

    expect(result.type).toBe("message");
    expect(result.message).toBe("Hello!");
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

  it("fails loudly when an action request reaches max steps without completing", async () => {
    const reg = new AssistantToolRegistry();
    reg.register({
      name: "mock.read",
      description: "Read",
      inputSchema: {
        type: "object",
        properties: { n: { type: "number" } },
        required: ["n"],
      },
      executor: vi.fn().mockResolvedValue({ ok: true, summary: "read" }),
      isReadOnly: true,
    });
    let n = 0;
    const runner = {
      onApiCall: vi.fn().mockReturnValue(vi.fn()),
      runToolLoop: vi.fn().mockImplementation(async () => ({
        stoppedEarly: true,
        reason: "awaiting_tool_execution",
        steps: [{
          type: "tool_call",
          content: "read",
          toolName: "mock.read",
          toolArgs: { n: n++ },
        }],
      })),
    };
    const runtime = new AssistantRuntime(runner as any, reg, false);
    const events: string[] = [];
    runtime.onEvent((event) => events.push(event.type));

    const result = await runtime.send("please update the metadata");

    expect(result.type).toBe("error");
    expect(result.message).toContain("maximum step limit");
    expect(events).not.toContain("completed");
  });

  it("stops repeated identical tool calls instead of fake-completing", async () => {
    const reg = makeRegistry();
    const runner = {
      onApiCall: vi.fn().mockReturnValue(vi.fn()),
      runToolLoop: vi.fn().mockResolvedValue({
        stoppedEarly: true,
        reason: "awaiting_tool_execution",
        steps: [{
          type: "tool_call",
          content: "summarize",
          toolName: "library.summarize",
          toolArgs: {},
        }],
      }),
    };
    const runtime = new AssistantRuntime(runner as any, reg, false);

    const result = await runtime.send("please update the metadata");

    expect(result.type).toBe("error");
    expect(result.message).toContain("repeated");
  });

  it("does not accept a no-tool final message for an actionable request", async () => {
    const reg = makeRegistry();
    const runner = {
      onApiCall: vi.fn().mockReturnValue(vi.fn()),
      runToolLoop: vi.fn().mockResolvedValue({
        stoppedEarly: false,
        steps: [{
          type: "message",
          content: "Applying automatic track numbering to all tracks in the library.",
        }],
      }),
    };
    const runtime = new AssistantRuntime(runner as any, reg, false);

    const result = await runtime.send("please update the metadata");

    expect(result.type).toBe("error");
    expect(result.message).toContain("No action was performed");
  });

  it("surfaces malformed tool loop responses as errors", async () => {
    const reg = makeRegistry();
    const runner = {
      onApiCall: vi.fn().mockReturnValue(vi.fn()),
      runToolLoop: vi.fn().mockResolvedValue({
        stoppedEarly: true,
        reason: "malformed_tool_call",
        steps: [{
          type: "message",
          content: "Malformed tool call: missing toolName.",
        }],
      }),
    };
    const runtime = new AssistantRuntime(runner as any, reg, false);

    const result = await runtime.send("please update the metadata");

    expect(result.type).toBe("error");
    expect(result.message).toContain("Malformed tool call");
  });

  it("retries invalid tool arguments once, then fails loudly", async () => {
    const reg = new AssistantToolRegistry();
    reg.register({
      name: "mock.edit",
      description: "Edit",
      inputSchema: {
        type: "object",
        properties: {
          target_scope: { type: "string", enum: ["library"] },
        },
        required: ["target_scope"],
      },
      executor: vi.fn().mockResolvedValue({ ok: true, summary: "should not run" }),
      isReadOnly: false,
      riskLevel: "low",
      operationKind: "metadata_edit",
    });
    const runner = {
      onApiCall: vi.fn().mockReturnValue(vi.fn()),
      runToolLoop: vi.fn().mockResolvedValue({
        stoppedEarly: true,
        reason: "awaiting_tool_execution",
        steps: [{
          type: "tool_call",
          content: "edit",
          toolName: "mock.edit",
          toolArgs: {},
        }],
      }),
    };
    const runtime = new AssistantRuntime(runner as any, reg, false);

    const result = await runtime.send("please update the metadata");

    expect(runner.runToolLoop).toHaveBeenCalledTimes(2);
    expect(result.type).toBe("error");
    expect(result.message).toContain("after retry");
  });
});

describe("deriveAssistantTaskContract", () => {
  it("classifies common action requests with deterministic routes", () => {
    expect(deriveAssistantTaskContract("number")).toMatchObject({
      kind: "action_preview_required",
      route: { toolName: "auto_numbering_tracks", args: { target_scope: "library" } },
    });
    expect(deriveAssistantTaskContract("auto tag this")).toMatchObject({
      route: { toolName: "run_library_task", args: { task: "auto_tag", target_scope: "library" } },
    });
    expect(deriveAssistantTaskContract("infer tags from filenames")).toMatchObject({
      route: { toolName: "infer_tags_from_filenames", args: { target_scope: "library" } },
    });
  });

  it("routes explicit tag value cleanup to extract_tag_value", () => {
    // Album number cleanup: album is not a valid number-strip target,
    // so falls through to general_action_intent (route undefined)
    const contract = deriveAssistantTaskContract('remove number and "-" from Album');
    expect(contract).toMatchObject({
      kind: "action_preview_required",
      reason: "general_action_intent",
      requiresCompletionEvidence: true,
    });
    expect(contract.route).toBeUndefined();

    // Title prefix cleanup: title IS a valid number-strip target → route exists
    const contract2 = deriveAssistantTaskContract("strip prefix from title");
    expect(contract2).toMatchObject({
      route: {
        toolName: "extract_tag_value",
        args: {
          target_scope: "library",
          field: "title",
          pattern: "^\\d+[\\s.\\)\\-–—]+(.+)$",
          group_index: 1,
        },
      },
      reason: "extract_tag_value_intent",
      requiresCompletionEvidence: true,
    });

    // Album suffix cleanup: album is not a valid number-strip target → falls through
    const contract3 = deriveAssistantTaskContract("remove suffix number from album tag");
    expect(contract3).toMatchObject({
      kind: "action_preview_required",
      reason: "general_action_intent",
      requiresCompletionEvidence: true,
    });
    expect(contract3.route).toBeUndefined();

    // Generic album cleanup without number hint: extract_tag_value returns no
    // deterministic route, and "clean" is not a general-action verb, so the
    // assistant will chat with the user to clarify intent.
    const contract4 = deriveAssistantTaskContract("clean album tag value");
    expect(contract4).toMatchObject({
      kind: "chat_only",
      reason: "no_action_or_read_only_intent",
      requiresCompletionEvidence: false,
    });
    expect(contract4.route).toBeUndefined();

    // "clean album name" has no deterministic route → chat with user to clarify
    const contract5 = deriveAssistantTaskContract("clean album name");
    expect(contract5).toMatchObject({
      kind: "chat_only",
      reason: "no_action_or_read_only_intent",
      requiresCompletionEvidence: false,
    });
    expect(contract5.route).toBeUndefined();
  });

  it("keeps non-action chat out of preview gating", () => {
    expect(deriveAssistantTaskContract("hello")).toMatchObject({
      kind: "chat_only",
      requiresCompletionEvidence: false,
    });
  });

  it("keeps missing-tag discovery read-only", () => {
    expect(deriveAssistantTaskContract("find tracks missing tags")).toMatchObject({
      kind: "read_only_answer",
      requiresCompletionEvidence: false,
    });
  });

  it("keeps read-only track-number questions read-only", () => {
    expect(deriveAssistantTaskContract("find tracks missing track numbers")).toMatchObject({
      kind: "read_only_answer",
      requiresCompletionEvidence: false,
    });
    expect(deriveAssistantTaskContract("show track numbers")).toMatchObject({
      kind: "read_only_answer",
      requiresCompletionEvidence: false,
    });
    expect(deriveAssistantTaskContract("how many tracks have no track number?")).toMatchObject({
      kind: "read_only_answer",
      requiresCompletionEvidence: false,
    });
  });

  it("still routes imperative track-number fixes to auto_numbering_tracks", () => {
    expect(deriveAssistantTaskContract("fix track numbers")).toMatchObject({
      kind: "action_preview_required",
      route: { toolName: "auto_numbering_tracks", args: { target_scope: "library" } },
      requiresCompletionEvidence: true,
    });
    expect(deriveAssistantTaskContract("number tracks within album")).toMatchObject({
      kind: "action_preview_required",
      route: { toolName: "auto_numbering_tracks", args: { target_scope: "library" } },
      requiresCompletionEvidence: true,
    });
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
