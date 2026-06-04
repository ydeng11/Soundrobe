import { describe, it, expect, vi, beforeEach } from "vitest";
import {
  PlanExecutor,
  type Plan,
  type PlanStepDef,
  type PlanStepOutput,
} from "../../electron/services/PlanExecutor";
import { AssistantToolRegistry } from "../../electron/services/AssistantToolRegistry";
import { AssistantRuntime } from "../../electron/services/AssistantRuntime";
import { LlmTaskRunner } from "../../electron/services/LlmTaskRunner";

// ── Mocks ──────────────────────────────────────────────────────────

vi.mock("../../electron/services/LlmTaskRunner", () => ({
  LlmTaskRunner: vi.fn().mockImplementation(() => ({
    runToolLoop: vi.fn(),
    onApiCall: vi.fn().mockReturnValue(vi.fn()),
  })),
}));

vi.mock("../../electron/handlers/conversation-logger", () => ({
  ConversationLogger: vi.fn().mockImplementation(() => ({
    recordEntry: vi.fn(),
    recordApiCall: vi.fn(),
    getOrCreateSessionNumber: vi.fn().mockReturnValue("1"),
    close: vi.fn(),
  })),
  NullConversationLogger: class NullConversationLogger {
    recordEntry() {}
    recordApiCall() {}
    getOrCreateSessionNumber() { return "1"; }
    close() {}
  },
}));

// ── Helpers ────────────────────────────────────────────────────────

function createTestSetup() {
  const registry = new AssistantToolRegistry();

  // Register a read-only inspect tool
  registry.register({
    name: "mock.inspect",
    description: "Mock inspect",
    inputSchema: {
      type: "object",
      properties: {
        path: { type: "string" },
      },
      required: [],
    },
    isReadOnly: true,
    executor: async (args) => {
      return {
        ok: true,
        summary: `Inspected: ${args.path ?? "none"}`,
        data: {
          path: args.path ?? "/default",
          tracks: [
            { path: "/music/01. 法老 - 百变酒精.flac", artist: "法老" },
          ],
          filename: "01. 法老 - 百变酒精.flac",
        },
      };
    },
  });

  // Create the runtime
  const runner = new LlmTaskRunner({ apiKey: "test-key" });
  const runtime = new AssistantRuntime(runner, registry, false);

  // Register a mutating edit tool (after runtime so executor can create batches)
  registry.register({
    name: "mock.edit",
    description: "Mock edit metadata",
    inputSchema: {
      type: "object",
      properties: {
        target_scope: { type: "string" },
        standard_updates: { type: "object" },
        standard_removes: { type: "array", items: { type: "string" } },
      },
      required: ["target_scope"],
    },
    isReadOnly: false,
    riskLevel: "low",
    executor: async (args) => {
      const batch = runtime.createActionBatch({
        kind: "metadata-update",
        title: "Mock edit",
        summary: `Edit: ${JSON.stringify(args.standard_updates ?? {})}`,
        riskLevel: "low",
        actions: [],
        reversible: true,
      });
      return {
        ok: true,
        summary: `Edit planned: ${JSON.stringify(args.standard_updates ?? {})}`,
        pendingActionBatchId: batch.id,
        data: { batchId: batch.id, updates: args.standard_updates },
      };
    },
  });

  // Register a tool that fails
  registry.register({
    name: "mock.fail",
    description: "Mock tool that fails",
    inputSchema: { type: "object", properties: {} },
    isReadOnly: true,
    executor: async () => ({
      ok: false,
      summary: "Something went wrong",
      error: "INTERNAL_ERROR",
    }),
  });

  const executor = new PlanExecutor(registry, runtime);

  return { registry, runtime, executor };
}

// ── Dependency Resolution Tests ────────────────────────────────────

describe("PlanExecutor — dependency resolution", () => {
  it("resolves linear dependencies: step1 → step2 → step3", () => {
    const { executor } = createTestSetup();
    const plan: Plan = {
      steps: [
        { id: "step3", tool: "mock.inspect", args: {}, depends_on: ["step2"] },
        { id: "step1", tool: "mock.inspect", args: {} },
        { id: "step2", tool: "mock.inspect", args: {}, depends_on: ["step1"] },
      ],
    };

    // Access private resolveDependencyOrder via any
    const order = (executor as any).resolveDependencyOrder(plan.steps);
    expect(order).toEqual(["step1", "step2", "step3"]);
  });

  it("resolves diamond dependencies: A → [B, C] → D", () => {
    const { executor } = createTestSetup();
    const plan: Plan = {
      steps: [
        { id: "A", tool: "mock.inspect", args: {} },
        { id: "B", tool: "mock.inspect", args: {}, depends_on: ["A"] },
        { id: "C", tool: "mock.inspect", args: {}, depends_on: ["A"] },
        { id: "D", tool: "mock.inspect", args: {}, depends_on: ["B", "C"] },
      ],
    };

    const order = (executor as any).resolveDependencyOrder(plan.steps);
    // A must be first, D must be last
    expect(order[0]).toBe("A");
    expect(order[order.length - 1]).toBe("D");
    // B and C come after A, order doesn't matter between them
    expect(order.indexOf("B")).toBeGreaterThan(order.indexOf("A"));
    expect(order.indexOf("C")).toBeGreaterThan(order.indexOf("A"));
    // D comes after both B and C
    expect(order.indexOf("D")).toBeGreaterThan(order.indexOf("B"));
    expect(order.indexOf("D")).toBeGreaterThan(order.indexOf("C"));
  });

  it("handles no dependencies (all steps run in definition order)", () => {
    const { executor } = createTestSetup();
    const plan: Plan = {
      steps: [
        { id: "first", tool: "mock.inspect", args: {} },
        { id: "second", tool: "mock.inspect", args: {} },
      ],
    };

    const order = (executor as any).resolveDependencyOrder(plan.steps);
    expect(order).toEqual(["first", "second"]);
  });

  it("throws on circular dependencies", () => {
    const { executor } = createTestSetup();
    const plan: Plan = {
      steps: [
        { id: "A", tool: "mock.inspect", args: {}, depends_on: ["B"] },
        { id: "B", tool: "mock.inspect", args: {}, depends_on: ["C"] },
        { id: "C", tool: "mock.inspect", args: {}, depends_on: ["A"] },
      ],
    };

    expect(() => (executor as any).resolveDependencyOrder(plan.steps)).toThrow(
      /circular/i,
    );
  });

  it("throws when depends_on references a non-existent step", () => {
    const { executor } = createTestSetup();
    const plan: Plan = {
      steps: [
        { id: "A", tool: "mock.inspect", args: {}, depends_on: ["GHOST"] },
      ],
    };

    expect(() => (executor as any).resolveDependencyOrder(plan.steps)).toThrow(
      /GHOST/,
    );
  });

  it("handles a single step", () => {
    const { executor } = createTestSetup();
    const plan: Plan = {
      steps: [{ id: "solo", tool: "mock.inspect", args: {} }],
    };

    const order = (executor as any).resolveDependencyOrder(plan.steps);
    expect(order).toEqual(["solo"]);
  });
});

// ── Argument Resolution Tests ──────────────────────────────────────

describe("PlanExecutor — $ref argument resolution", () => {
  it("resolves $stepId to full step output", () => {
    const { executor } = createTestSetup();
    const scratchpad = new Map<string, unknown>();
    scratchpad.set("inspect", {
      path: "/music/album",
      tracks: [{ path: "/music/track.flac" }],
    });

    const resolved = (executor as any).resolveArgs(
      { paths: "$inspect.tracks" },
      scratchpad,
    );
    expect(resolved.paths).toEqual([{ path: "/music/track.flac" }]);
  });

  it("resolves $stepId.field to a nested property", () => {
    const { executor } = createTestSetup();
    const scratchpad = new Map<string, unknown>();
    scratchpad.set("inspect", {
      path: "/music/album",
      filename: "01. 法老 - 百变酒精.flac",
    });

    const resolved = (executor as any).resolveArgs(
      { file: "$inspect.filename" },
      scratchpad,
    );
    expect(resolved.file).toBe("01. 法老 - 百变酒精.flac");
  });

  it("resolves $ref inside arrays", () => {
    const { executor } = createTestSetup();
    const scratchpad = new Map<string, unknown>();
    scratchpad.set("prev", {
      paths: ["/a.flac", "/b.flac"],
    });

    const resolved = (executor as any).resolveArgs(
      { items: ["$prev.paths", "/static.flac"] },
      scratchpad,
    );
    expect(resolved.items).toEqual([["/a.flac", "/b.flac"], "/static.flac"]);
  });

  it("passes through non-$ref values unchanged", () => {
    const { executor } = createTestSetup();
    const resolved = (executor as any).resolveArgs(
      {
        target_scope: "active_album",
        standard_updates: { albumArtist: "法老" },
      },
      new Map(),
    );
    expect(resolved.target_scope).toBe("active_album");
    expect(resolved.standard_updates).toEqual({ albumArtist: "法老" });
  });

  it("returns undefined for unresolvable references", () => {
    const { executor } = createTestSetup();
    const resolved = (executor as any).resolveArgs(
      { paths: "$ghost.tracks" },
      new Map(),
    );
    expect(resolved.paths).toBeUndefined();
  });
});

// ── End-to-End Plan Execution Tests ────────────────────────────────

describe("PlanExecutor — full execution", () => {
  it("executes a 3-step linear plan and collects batches", async () => {
    const { executor } = createTestSetup();

    const plan: Plan = {
      steps: [
        {
          id: "inspect",
          label: "Read current metadata",
          tool: "mock.inspect",
          args: { path: "/music/法老" },
        },
        {
          id: "edit",
          label: "Fix album artist and artist",
          tool: "mock.edit",
          args: {
            target_scope: "active_album",
            standard_updates: {
              albumArtist: "法老",
              artist: "法老",
              artists: ["法老"],
            },
          },
          depends_on: ["inspect"],
        },
        {
          id: "verify",
          label: "Verify changes",
          tool: "mock.inspect",
          args: { path: "$inspect.path" },
          depends_on: ["edit"],
        },
      ],
    };

    const result = await executor.execute(plan);

    // All 3 steps executed
    expect(result.stepOutputs).toHaveLength(3);
    expect(result.stepOutputs[0].stepId).toBe("inspect");
    expect(result.stepOutputs[0].ok).toBe(true);
    expect(result.stepOutputs[1].stepId).toBe("edit");
    expect(result.stepOutputs[1].ok).toBe(true);
    expect(result.stepOutputs[2].stepId).toBe("verify");
    expect(result.stepOutputs[2].ok).toBe(true);

    // 1 batch collected (from the edit step)
    expect(result.batches).toHaveLength(1);

    // Scratchpad has all step outputs
    expect(result.scratchpad.has("inspect")).toBe(true);
    expect(result.scratchpad.has("edit")).toBe(true);
    expect(result.scratchpad.has("verify")).toBe(true);

    // Verify step had $ref resolved: inspect path forwarded to verify
    expect(result.stepOutputs[2].summary).toContain("/music/法老");

    // No errors
    expect(result.errors).toHaveLength(0);
  });

  it("continues execution when a non-critical step fails", async () => {
    const { executor } = createTestSetup();

    const plan: Plan = {
      steps: [
        { id: "first", tool: "mock.inspect", args: {} },
        {
          id: "fail",
          label: "This step fails",
          tool: "mock.fail",
          args: {},
        },
        { id: "last", tool: "mock.inspect", args: {}, depends_on: ["first"] },
      ],
    };

    const result = await executor.execute(plan);

    // All 3 steps attempted
    expect(result.stepOutputs).toHaveLength(3);
    expect(result.stepOutputs[1].ok).toBe(false);

    // Error collected
    expect(result.errors).toHaveLength(1);
    expect(result.errors[0].stepId).toBe("fail");

    // Third step still ran (depends on first, not fail)
    expect(result.stepOutputs[2].ok).toBe(true);
  });

  it("runs a plan with no mutating steps (no batches)", async () => {
    const { executor } = createTestSetup();

    const plan: Plan = {
      steps: [
        { id: "a", tool: "mock.inspect", args: { path: "/p1" } },
        { id: "b", tool: "mock.inspect", args: { path: "/p2" } },
      ],
    };

    const result = await executor.execute(plan);

    expect(result.stepOutputs).toHaveLength(2);
    expect(result.batches).toHaveLength(0);
    expect(result.errors).toHaveLength(0);
  });
});
