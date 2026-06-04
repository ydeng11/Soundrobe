/**
 * PlanExecutor — multi-step plan execution on top of the assistant tool registry.
 *
 * Turns a dependency-ordered plan into sequential tool calls with:
 * - Inter-step state tracking (scratchpad)
 * - Dependency resolution (topological sort)
 * - Argument referencing ($stepId.field → previous output)
 * - Post-step validation
 * - Batch collection for user approval
 *
 * The LLM defines the plan via the `create_plan` tool, then the PlanExecutor
 * runs it. All mutating steps produce preview batches that the user approves
 * once at the end.
 */

import { AssistantToolRegistry, type AssistantToolResult } from "./AssistantToolRegistry";
import { AssistantRuntime, type AssistantActionBatch } from "./AssistantRuntime";

// ── Types ──────────────────────────────────────────────────────────

export interface PlanStepDef {
  /** Unique step identifier (e.g. "step-1", "inspect-files") */
  id: string;
  /** Human-readable label for the step */
  label?: string;
  /** Name of the registered assistant tool to call */
  tool: string;
  /** Tool arguments. Supports reference syntax: "$step-1.tracks" or "$inspect.paths" */
  args: Record<string, unknown>;
  /** Step IDs this step depends on (empty for first step) */
  depends_on?: string[];
  /** Optional validation callback to run after execution */
  validate?: (
    output: PlanStepOutput,
  ) => { pass: boolean; errors: string[] };
}

export interface Plan {
  steps: PlanStepDef[];
}

export interface PlanStepOutput {
  stepId: string;
  label: string;
  ok: boolean;
  summary: string;
  data?: unknown;
  pendingActionBatchId?: string;
}

export interface PlanStepError {
  stepId: string;
  error: string;
}

export interface PlanResult {
  stepOutputs: PlanStepOutput[];
  batches: AssistantActionBatch[];
  errors: PlanStepError[];
  /** Key-value store of all step outputs keyed by stepId */
  scratchpad: Map<string, unknown>;
}

// ── Executor ───────────────────────────────────────────────────────

export class PlanExecutor {
  private registry: AssistantToolRegistry;
  private runtime: AssistantRuntime;

  constructor(registry: AssistantToolRegistry, runtime: AssistantRuntime) {
    this.registry = registry;
    this.runtime = runtime;
  }

  /**
   * Execute a plan through dependency-ordered tool calls.
   *
   * Flow per step:
   *   resolve args → execute tool → validate output → store scratchpad
   *
   * Mutating steps produce preview batches; the user approves them once
   * after all steps complete.
   */
  async execute(plan: Plan): Promise<PlanResult> {
    const order = this.resolveDependencyOrder(plan.steps);
    const scratchpad = new Map<string, unknown>();
    const stepOutputs: PlanStepOutput[] = [];
    const batches: AssistantActionBatch[] = [];
    const errors: PlanStepError[] = [];

    for (const stepId of order) {
      const step = plan.steps.find((s) => s.id === stepId);
      if (!step) {
        errors.push({ stepId, error: `Step "${stepId}" not found in plan` });
        continue;
      }

      const label = step.label ?? step.id;

      // Resolve any $reference arguments from previous steps
      const resolvedArgs = this.resolveArgs(step.args, scratchpad);

      // Execute the tool
      const toolResult: AssistantToolResult =
        await this.registry.execute(step.tool, resolvedArgs);

      const output: PlanStepOutput = {
        stepId: step.id,
        label,
        ok: toolResult.ok,
        summary: toolResult.summary,
        data: toolResult.data,
        pendingActionBatchId: toolResult.pendingActionBatchId,
      };

      // Store in scratchpad for reference by downstream steps
      scratchpad.set(step.id, toolResult.data ?? toolResult.summary);

      // If this step produced a pending batch, collect it
      if (toolResult.pendingActionBatchId) {
        const batch = this.runtime.getActionBatch(
          toolResult.pendingActionBatchId,
        );
        if (batch) {
          batches.push(batch);
        }
      }

      // Run validation if the step has one
      if (!toolResult.ok) {
        errors.push({
          stepId: step.id,
          error: toolResult.error ?? "Unknown error",
        });
        // Continue with remaining steps (the LLM can adapt)
      }

      if (step.validate && toolResult.ok && toolResult.data) {
        const validation = step.validate(output);
        if (!validation.pass) {
          errors.push({
            stepId: step.id,
            error: `Validation: ${validation.errors.join("; ")}`,
          });
        }
      }

      stepOutputs.push(output);
    }

    return { stepOutputs, batches, errors, scratchpad };
  }

  /**
   * Resolve "$stepId.field" references in arguments against the scratchpad.
   *
   * Example:
   *   args = { paths: "$inspect-files.tracks" }
   *   scratchpad has "inspect-files" = { tracks: ["/a.flac", "/b.flac"] }
   *   → resolved = { paths: ["/a.flac", "/b.flac"] }
   */
  private resolveArgs(
    args: Record<string, unknown>,
    scratchpad: Map<string, unknown>,
  ): Record<string, unknown> {
    const resolved: Record<string, unknown> = {};

    for (const [key, value] of Object.entries(args)) {
      if (typeof value === "string" && value.startsWith("$")) {
        // Format: $stepId or $stepId.field
        const ref = value.slice(1); // remove "$"
        const dotIdx = ref.indexOf(".");
        if (dotIdx >= 0) {
          const refStepId = ref.slice(0, dotIdx);
          const refField = ref.slice(dotIdx + 1);
          const stepData = scratchpad.get(refStepId);
          if (
            stepData &&
            typeof stepData === "object" &&
            !Array.isArray(stepData)
          ) {
            const stepObj = stepData as Record<string, unknown>;
            resolved[key] = stepObj[refField];
          } else {
            resolved[key] = stepData;
          }
        } else {
          resolved[key] = scratchpad.get(ref);
        }
      } else if (Array.isArray(value)) {
        resolved[key] = value.map((item) => {
          if (typeof item === "string" && item.startsWith("$")) {
            const ref = item.slice(1);
            const dotIdx = ref.indexOf(".");
            if (dotIdx >= 0) {
              const refStepId = ref.slice(0, dotIdx);
              const refField = ref.slice(dotIdx + 1);
              const stepData = scratchpad.get(refStepId);
              if (
                stepData &&
                typeof stepData === "object" &&
                !Array.isArray(stepData)
              ) {
                return (stepData as Record<string, unknown>)[refField];
              }
              return stepData;
            }
            return scratchpad.get(ref);
          }
          return item;
        });
      } else {
        resolved[key] = value;
      }
    }

    return resolved;
  }

  /**
   * Topological sort of plan steps based on depends_on.
   * Throws if a circular dependency is detected.
   */
  private resolveDependencyOrder(steps: PlanStepDef[]): string[] {
    const visited = new Set<string>();
    const visiting = new Set<string>();
    const order: string[] = [];
    const stepMap = new Map(steps.map((s) => [s.id, s]));

    function visit(stepId: string): void {
      if (visited.has(stepId)) return;
      if (visiting.has(stepId)) {
        throw new Error(`Circular dependency detected: ${stepId}`);
      }
      visiting.add(stepId);

      const step = stepMap.get(stepId);
      if (step?.depends_on) {
        for (const dep of step.depends_on) {
          if (!stepMap.has(dep)) {
            throw new Error(
              `Step "${stepId}" depends on unknown step "${dep}"`,
            );
          }
          visit(dep);
        }
      }

      visiting.delete(stepId);
      visited.add(stepId);
      order.push(stepId);
    }

    for (const step of steps) {
      visit(step.id);
    }

    return order;
  }
}
