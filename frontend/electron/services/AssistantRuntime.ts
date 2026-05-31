/**
 * AssistantRuntime — manages the assistant session lifecycle.
 *
 * Flow:
 * 1. Build context: compact prompt with app state, tool list, recent chat
 * 2. Ask the LLM for a message or tool call
 * 3. Validate tool name and arguments
 * 4. Execute read-only tools immediately
 * 5. For mutating tools, create a pending action batch (preview mode)
 * 6. Feed tool result back to the LLM
 * 7. Stop after a final response or max-step limit
 *
 * Mutating tools in default (preview) mode create a pending action batch
 * instead of executing immediately. The renderer applies or rejects via
 * applyActionBatch / rejectActionBatch.
 */

import { LlmTaskRunner, redactSensitive } from "./LlmTaskRunner";
import type { AssistantToolDef } from "./AssistantToolRegistry";
import { AssistantToolRegistry, type AssistantToolResult } from "./AssistantToolRegistry";

export type AssistantActionBatchKind =
  | "tag-update"
  | "extra-tag-update"
  | "metadata-update"
  | "folder-move"
  | "auto-tag-run"
  | "audit-run";

export interface AssistantAction {
  tagKind?: "standard" | "extra";
  trackPath?: string;
  field?: string;
  oldValue?: string | null;
  newValue?: string | null;
  operation?: string;
  destinationPath?: string;
  sourcePath?: string;
  skipReason?: string;
  description?: string;
}

export interface AssistantActionBatch {
  id: string;
  createdAt: string;
  sessionId: string;
  kind: AssistantActionBatchKind;
  title: string;
  summary: string;
  riskLevel: "low" | "medium" | "high";
  actions: AssistantAction[];
  reversible: boolean;
  status: "pending" | "applied" | "rejected" | "failed";
}

export interface AssistantEvent {
  sessionId: string;
  type:
    | "step"
    | "tool_running"
    | "tool_result"
    | "action_batch_created"
    | "action_batch_applied"
    | "action_batch_rejected"
    | "action_batch_failed"
    | "message"
    | "error"
    | "completed"
    | "cancelled";
  message: string;
  data?: unknown;
}

export type AssistantEventCallback = (event: AssistantEvent) => void;

interface ConversationMessage {
  role: string;
  content: string;
}

const SYSTEM_PROMPT = `You are an assistant for Auto Tagger, a tool for organizing and tagging music libraries.

Guidelines:
- Use tools only through the provided registry.
- Default to the current selection, then active album, then current library.
- For destructive or mutating work, create a preview action batch.
- Ask a concise clarification only when the target scope is genuinely unclear.
- Never request or expose API keys.
- Never invent file paths outside the current library.
- Prefer small, reversible batches.
- Prefer composite macro tools for write/task work: edit_metadata, organize_files, and run_library_task.
- Use read-only tools to discover context, then call one macro with clear parameters.
- Be concise but helpful.
- When asked about library content, use library.summarize or query.metadata.
- To inspect specific tracks use tracks.inspect or tracks.search.
- To get album details use albums.inspect.`;

export class AssistantRuntime {
  private runner: LlmTaskRunner;
  private sessionId: string;
  private conversation: ConversationMessage[] = [];
  private registry: AssistantToolRegistry;
  private autonomous: boolean;
  private cancelled = false;
  private eventCallbacks: AssistantEventCallback[] = [];
  private pendingBatches: Map<string, AssistantActionBatch> = new Map();
  private appliedBatchIds: Set<string> = new Set();
  private nextBatchId = 1;

  constructor(
    runner: LlmTaskRunner,
    registry: AssistantToolRegistry,
    autonomous: boolean = false,
  ) {
    this.runner = runner;
    this.sessionId = this.generateSessionId();
    this.registry = registry;
    this.autonomous = autonomous;
  }

  getSessionId(): string {
    return this.sessionId;
  }

  setAutonomous(autonomous: boolean): void {
    this.autonomous = autonomous;
  }

  isAutonomous(): boolean {
    return this.autonomous;
  }

  onEvent(callback: AssistantEventCallback): () => void {
    this.eventCallbacks.push(callback);
    return () => {
      const idx = this.eventCallbacks.indexOf(callback);
      if (idx >= 0) this.eventCallbacks.splice(idx, 1);
    };
  }

  private emit(event: AssistantEvent): void {
    for (const cb of this.eventCallbacks) {
      try {
        cb(event);
      } catch {
        // Silently ignore callback errors
      }
    }
  }

  /**
   * Cancel the current session.
   */
  cancel(): void {
    this.cancelled = true;
    this.emit({
      sessionId: this.sessionId,
      type: "cancelled",
      message: "Session cancelled",
    });
  }

  /**
   * Send a user message and process the assistant loop.
   */
  async send(userMessage: string): Promise<AssistantEvent> {
    this.cancelled = false;
    this.conversation.push({ role: "user", content: userMessage });

    const maxSteps = 6;

    for (let step = 0; step < maxSteps; step++) {
      if (this.cancelled) {
        return {
          sessionId: this.sessionId,
          type: "cancelled",
          message: "Cancelled",
        };
      }

      this.emit({
        sessionId: this.sessionId,
        type: "step",
        message: `Step ${step + 1}/${maxSteps}`,
      });

      // Ask the LLM for a response
      const result = await this.runner.runToolLoop({
        systemPrompt: SYSTEM_PROMPT,
        messages: this.conversation,
        tools: this.registry.getAll(),
        maxSteps: 1, // One step at a time so we can process tool results
        maxTokens: 1024,
      });

      // Check the result
      const lastStep = result.steps[result.steps.length - 1];

      if (!lastStep) {
        const errorEvent: AssistantEvent = {
          sessionId: this.sessionId,
          type: "error",
          message: "No response from assistant",
        };
        this.emit(errorEvent);
        return errorEvent;
      }

      if (lastStep.type === "message") {
        this.conversation.push({ role: "assistant", content: lastStep.content });
        const event: AssistantEvent = {
          sessionId: this.sessionId,
          type: "message",
          message: lastStep.content,
        };
        this.emit(event);
        return event;
      }

      if (lastStep.type === "tool_call") {
        const toolName = lastStep.toolName ?? "unknown";
        const toolArgs = lastStep.toolArgs ?? {};

        this.emit({
          sessionId: this.sessionId,
          type: "tool_running",
          message: `Running tool: ${toolName}`,
          data: { toolName, toolArgs },
        });

        // Check if tool exists
        const tool = this.registry.get(toolName);
        if (!tool) {
          const errorMsg = `Unknown tool: ${toolName}`;
          this.conversation.push({
            role: "assistant",
            content: JSON.stringify({ type: "tool_call", toolName, args: toolArgs, reason: "" }),
          });
          this.conversation.push({
            role: "user",
            content: `Error: ${errorMsg}. Available tools: ${this.registry.getAll().map((t) => t.name).join(", ")}`,
          });
          const errorEvent: AssistantEvent = {
            sessionId: this.sessionId,
            type: "error",
            message: errorMsg,
          };
          this.emit(errorEvent);
          continue;
        }

        // Execute the tool
        const toolResult = await this.registry.execute(toolName, toolArgs);

        // For mutating tools in preview mode, create action batch
        if (!tool.isReadOnly && !this.autonomous) {
          // Tool executor already created the batch internally
          // Feed the result back
          this.conversation.push({
            role: "assistant",
            content: JSON.stringify({ type: "tool_call", toolName, args: toolArgs, reason: "" }),
          });
          this.conversation.push({
            role: "user",
            content: toolResult.summary,
          });

          this.emit({
            sessionId: this.sessionId,
            type: "tool_result",
            message: toolResult.summary,
            data: toolResult,
          });

          if (toolResult.pendingActionBatchId) {
            this.emit({
              sessionId: this.sessionId,
              type: "action_batch_created",
              message: toolResult.summary,
              data: { actionBatchId: toolResult.pendingActionBatchId, toolResult },
            });
            return {
              sessionId: this.sessionId,
              type: "action_batch_created",
              message: toolResult.summary,
              data: { actionBatchId: toolResult.pendingActionBatchId, toolResult },
            };
          }

          continue;
        }

        // Read-only tools: feed result back and continue
        const safeResult = redactSensitive(toolResult.summary);
        this.conversation.push({
          role: "assistant",
          content: JSON.stringify({ type: "tool_call", toolName, args: toolArgs, reason: "" }),
        });
        this.conversation.push({
          role: "user",
          content: `Tool "${toolName}" result: ${safeResult}`,
        });

        this.emit({
          sessionId: this.sessionId,
          type: "tool_result",
          message: safeResult,
          data: toolResult,
        });
      }
    }

    // Max steps reached without final message
    const event: AssistantEvent = {
      sessionId: this.sessionId,
      type: "completed",
      message: "Reached maximum steps. Let me know if you need anything else.",
    };
    this.emit(event);
    return event;
  }

  /**
   * Create a pending action batch.
   */
  createActionBatch(input: {
    kind: AssistantActionBatchKind;
    title: string;
    summary: string;
    riskLevel: "low" | "medium" | "high";
    actions: AssistantAction[];
    reversible: boolean;
  }): AssistantActionBatch {
    const batch: AssistantActionBatch = {
      id: `batch-${this.sessionId}-${this.nextBatchId++}`,
      createdAt: new Date().toISOString(),
      sessionId: this.sessionId,
      kind: input.kind,
      title: input.title,
      summary: input.summary,
      riskLevel: input.riskLevel,
      actions: input.actions,
      reversible: input.reversible,
      status: "pending",
    };
    this.pendingBatches.set(batch.id, batch);
    return batch;
  }

  /**
   * Get a pending action batch by ID.
   */
  getActionBatch(batchId: string): AssistantActionBatch | undefined {
    return this.pendingBatches.get(batchId);
  }

  /**
   * Get all pending action batches for the current session.
   */
  getPendingBatches(): AssistantActionBatch[] {
    return Array.from(this.pendingBatches.values()).filter(
      (b) => b.status === "pending",
    );
  }

  /**
   * Mark a batch as applied.
   */
  markBatchApplied(batchId: string): void {
    const batch = this.pendingBatches.get(batchId);
    if (batch) {
      batch.status = "applied";
      this.appliedBatchIds.add(batchId);
      this.emit({
        sessionId: this.sessionId,
        type: "action_batch_applied",
        message: `Applied: ${batch.title}`,
        data: { batchId },
      });
    }
  }

  /**
   * Mark a batch as rejected.
   */
  markBatchRejected(batchId: string): void {
    const batch = this.pendingBatches.get(batchId);
    if (batch) {
      batch.status = "rejected";
      this.emit({
        sessionId: this.sessionId,
        type: "action_batch_rejected",
        message: `Rejected: ${batch.title}`,
        data: { batchId },
      });
    }
  }

  /**
   * Mark a batch as failed.
   */
  markBatchFailed(batchId: string, error: string): void {
    const batch = this.pendingBatches.get(batchId);
    if (batch) {
      batch.status = "failed";
      this.emit({
        sessionId: this.sessionId,
        type: "action_batch_failed",
        message: `Failed: ${batch.title}: ${error}`,
        data: { batchId, error },
      });
    }
  }

  /**
   * Add an assistant message to the conversation history.
   */
  addAssistantMessage(content: string): void {
    this.conversation.push({ role: "assistant", content });
  }

  /**
   * Add a user message to the conversation history.
   */
  addUserMessage(content: string): void {
    this.conversation.push({ role: "user", content });
  }

  /**
   * Clear conversation history.
   */
  clearConversation(): void {
    this.conversation = [];
  }

  private generateSessionId(): string {
    return `session-${Date.now()}-${Math.random().toString(36).slice(2, 9)}`;
  }
}
