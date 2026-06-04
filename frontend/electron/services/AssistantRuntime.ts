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

import { LlmTaskRunner, redactSensitive, type ApiCallEvent } from "./LlmTaskRunner";
import {
  AssistantToolRegistry,
  type AssistantToolOperationKind,
  type AssistantToolResult,
} from "./AssistantToolRegistry";
import { type IConversationLogger, NullConversationLogger } from "../handlers/conversation-logger";

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
  If both selection and active album are empty, use target_scope "library" to target all loaded tracks.
- For destructive or mutating work, create a preview action batch.
- Ask a concise clarification only when the target scope is genuinely unclear.
- Never request or expose API keys.
- Never invent file paths outside the current library.
- Prefer small, reversible batches.
- Prefer composite macro tools for write/task work: auto_numbering_tracks, infer_tags_from_filenames, edit_metadata, organize_files, group_by_album, and run_library_task.
- Use read-only tools to discover context, then call one macro with clear parameters.
- Before calling a mutating tool, identify the intended operation type. If the user asks to fix metadata, do not choose a file-moving tool.
- Be concise but helpful.
- When asked about library content, use library.summarize or query.metadata.
- To inspect specific tracks use tracks.inspect or tracks.search.
- To get album details use albums.inspect.
- tracks.inspect returns at most 20 tracks by default. Pass a limit (up to 500) to see more: e.g., tracks.inspect with limit: 500 shows all tracks.

SAFETY RULES FOR edit_metadata:
- The standard_updates and extra_upserts fields in edit_metadata apply the SAME values to EVERY targeted track.
- NEVER use target_scope "library" or "active_album" with per-track fields (title, artist, artists, trackNumber, trackTotal, discNumber, discTotal) unless every track should get the exact same value.
- When asked to fix title/artist/artists from file names, use infer_tags_from_filenames. Do not manually parse many filenames into edit_metadata.
- If each track needs different values, first use tracks.inspect (with a high limit like 500) or tracks.search to find the specific tracks, then call edit_metadata with target_scope "explicit_paths" and list the exact paths for each batch of tracks that share the same values.`;

interface ToolIntentMismatchInput {
  userMessage: string;
  toolName: string;
  operationKind?: AssistantToolOperationKind;
}

interface ToolIntentMismatch {
  expectedOperationKind: AssistantToolOperationKind;
  summary: string;
}

function hasTrackNumberIntent(text: string): boolean {
  return /\b(track\s*(number|numbers|#)|tracknumber|tracktotal|renumber|renumbering|numbering)\b/i.test(text)
    || /number(?:ed|ing)?\s+within\s+(each\s+)?album/i.test(text)
    || /within\s+(each\s+)?album/i.test(text) && /\bnumber/i.test(text);
}

function hasExplicitFileMoveIntent(text: string): boolean {
  return /\b(move|moving|organize|organise|folder|folders|directory|directories|rename|relocate)\b/i.test(text);
}

export function detectToolIntentMismatch(
  input: ToolIntentMismatchInput,
): ToolIntentMismatch | null {
  if (input.operationKind !== "file_move") return null;
  if (!hasTrackNumberIntent(input.userMessage)) return null;
  if (hasExplicitFileMoveIntent(input.userMessage)) return null;

  return {
    expectedOperationKind: "metadata_edit",
    summary:
      `Pre-action check blocked "${input.toolName}": the latest user message asks to fix track-number metadata within albums, but this tool moves files on disk. Use auto_numbering_tracks for track numbering instead.`,
  };
}

export class AssistantRuntime {
  private runner: LlmTaskRunner;
  private sessionId: string;
  private conversation: ConversationMessage[] = [];
  private registry: AssistantToolRegistry;
  private autonomous: boolean;
  private cancelled = false;
  private eventCallbacks: AssistantEventCallback[] = [];
  private pendingBatches: Map<string, AssistantActionBatch> = new Map();
  private nextBatchId = 1;
  private logger: IConversationLogger;
  private unsubscribeApiCall: (() => void) | null = null;

  constructor(
    runner: LlmTaskRunner,
    registry: AssistantToolRegistry,
    autonomous: boolean = false,
    logger?: IConversationLogger,
  ) {
    this.runner = runner;
    this.sessionId = this.generateSessionId();
    this.registry = registry;
    this.autonomous = autonomous;

    this.logger = logger ?? new NullConversationLogger();

    this.unsubscribeApiCall = runner.onApiCall((event: ApiCallEvent) => {
      this.logger.recordApiCall(
        this.sessionId,
        { messages: event.messages },
        {
          data: event.data,
          model: event.model,
          promptTokens: event.promptTokens,
          completionTokens: event.completionTokens,
          totalTokens: event.totalTokens,
        },
        event.cost,
        event.type,
      );
    });
  }

  /**
   * Clean up resources. Call when the runtime is no longer needed.
   */
  dispose(): void {
    this.unsubscribeApiCall?.();
    this.unsubscribeApiCall = null;
    try { this.logger.close(); } catch { /* ignore */ }
  }

  getSessionId(): string {
    return this.sessionId;
  }

  /**
   * Get the human-readable session number (1, 2, 3, ...) for this session.
   */
  getSessionNumber(): string {
    return this.logger.getOrCreateSessionNumber(this.sessionId);
  }

  /**
   * Get the conversation logger for querying historical sessions.
   */
  getConversationLogger(): IConversationLogger {
    return this.logger;
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
    this.logger.recordEntry({
      sessionUuid: this.sessionId,
      entryType: "system",
      content: "Session cancelled",
    });
    this.emit({
      sessionId: this.sessionId,
      type: "cancelled",
      message: "Session cancelled",
    });
  }

  /**
   * Check the last N assistant messages for repeated tool calls.
   * Returns { repeated: true, toolName, callCount } when the same tool+args
   * appears 3+ consecutive times.
   */
  private detectRepeatedToolCalls(): { repeated: boolean; toolName: string; callCount: number } {
    const parsed = this.conversation
      .filter((m) => m.role === "assistant")
      .map((m) => {
        try { return JSON.parse(m.content) as Record<string, unknown>; }
        catch { return null; }
      })
      .filter((p): p is Record<string, unknown> => p?.type === "tool_call")
      .slice(-5);

    if (parsed.length < 3) return { repeated: false, toolName: "", callCount: 0 };

    const signatures = parsed.map((p) => {
      const args = (p.args ?? {}) as Record<string, unknown>;
      return `${p.toolName as string}|${JSON.stringify(args, Object.keys(args).sort())}`;
    });
    const lastSig = signatures[signatures.length - 1];
    const count = signatures.filter((s) => s === lastSig).length;
    if (count >= 3) {
      return { repeated: true, toolName: (parsed[parsed.length - 1].toolName as string) ?? "", callCount: count };
    }
    return { repeated: false, toolName: "", callCount: 0 };
  }

  /**
   * Send a user message and process the assistant loop.
   */
  async send(userMessage: string): Promise<AssistantEvent> {
    this.cancelled = false;
    this.conversation.push({ role: "user", content: userMessage });

    const maxSteps = 6;
    let repeatedCalls: { toolName: string; callCount: number } | null = null;

    for (let step = 0; step < maxSteps; step++) {
      if (this.cancelled) {
        return {
          sessionId: this.sessionId,
          type: "cancelled",
          message: "Cancelled",
        };
      }

      // Detect repeated tool calls before the next API call
      const detected = this.detectRepeatedToolCalls();
      if (detected.repeated && !repeatedCalls) {
        repeatedCalls = { toolName: detected.toolName, callCount: detected.callCount };
        const hint = `[System note: You called "${detected.toolName}" with the same arguments ${detected.callCount} times in a row. Consider a different approach or different arguments.]`;
        this.conversation.push({ role: "system", content: hint });
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

      const lastStep = result.steps.at(-1);

      if (!lastStep) {
        const errMsg = "No response from assistant";
        this.logger.recordEntry({
          sessionUuid: this.sessionId,
          entryType: "system",
          content: errMsg,
        });
        const errorEvent: AssistantEvent = {
          sessionId: this.sessionId,
          type: "error",
          message: errMsg,
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

        this.logger.recordEntry({
          sessionUuid: this.sessionId,
          entryType: "tool_call",
          content: JSON.stringify({ toolName, toolArgs, reason: lastStep.content }),
          metadata: { toolName, args: toolArgs },
        });

        // Push tool call into conversation history (shared by all branches)
        this.conversation.push({
          role: "assistant",
          content: JSON.stringify({ type: "tool_call", toolName, args: toolArgs, reason: "" }),
        });

        this.emit({
          sessionId: this.sessionId,
          type: "tool_running",
          message: `Running tool: ${toolName}`,
          data: { toolName, toolArgs },
        });

        const tool = this.registry.get(toolName);
        if (!tool) {
          const errorMsg = `Unknown tool: ${toolName}`;
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

        const mismatch = detectToolIntentMismatch({
          userMessage,
          toolName,
          operationKind: tool.operationKind,
        });
        if (mismatch) {
          const toolResult: AssistantToolResult = {
            ok: false,
            summary: mismatch.summary,
            error: "TOOL_INTENT_MISMATCH",
          };

          this.logger.recordEntry({
            sessionUuid: this.sessionId,
            entryType: "tool_result",
            content: toolResult.summary,
            metadata: { toolName, args: toolArgs, ok: toolResult.ok, blocked: true },
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
          continue;
        }

        const toolResult = await this.registry.execute(toolName, toolArgs);

        this.logger.recordEntry({
          sessionUuid: this.sessionId,
          entryType: "tool_result",
          content: toolResult.summary,
          metadata: { toolName, args: toolArgs, ok: toolResult.ok },
        });

        if (!tool.isReadOnly && !this.autonomous) {
          this.conversation.push({ role: "user", content: toolResult.summary });
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
    // Build diagnostic info: last few conversation entries + repeated calls
    const recentEntries = this.conversation
      .slice(-4)
      .map((m) => `${m.role}: ${m.content.slice(0, 200)}`)
      .join("\n");
    let diagMsg =
      `I reached the maximum step limit (${maxSteps}) and couldn't complete the task. ` +
      `This often happens when tool calls return unexpected results or the task ` +
      `requires more steps than allowed.`;
    if (repeatedCalls) {
      diagMsg += `\n\nThe assistant called "${repeatedCalls.toolName}" with the same arguments ${repeatedCalls.callCount} times. This suggests the tool results were not what was expected. Try rephrasing your request or providing more specific file paths.`;
    }
    diagMsg += `\n\nLast conversation entries:\n${recentEntries}`;
    const event: AssistantEvent = {
      sessionId: this.sessionId,
      type: "completed",
      message: diagMsg,
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

  /**
   * Reset the session: clear conversation history and generate a new session ID.
   */
  resetSession(): void {
    this.conversation = [];
    this.sessionId = this.generateSessionId();
    console.log(`[AssistantRuntime] Session reset, new id: ${this.sessionId}`);
  }

  private generateSessionId(): string {
    const id = `session-${Date.now()}-${Math.random().toString(36).slice(2, 9)}`;
    console.log(`[AssistantRuntime] Created session id: ${id}`);
    return id;
  }
}
