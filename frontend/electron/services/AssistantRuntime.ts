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
- Prefer composite macro tools for write/task work: auto_numbering_tracks, strip_track_title_prefixes, strip_filename_prefixes, infer_tags_from_filenames, extract_tag_value, edit_metadata, organize_files, group_by_album, and run_library_task.
- Use read-only tools to discover context, then call one macro with clear parameters.
- Before calling a mutating tool, identify the intended operation type. If the user asks to fix metadata, do not choose a file-moving tool.
- Be concise but helpful.
- When asked about library content, use library.summarize or query.metadata.
- To inspect specific tracks use tracks.inspect or tracks.search.
- To get album details use albums.inspect.
- tracks.inspect returns at most 20 tracks by default. Pass a limit (up to 500) to see more: e.g., tracks.inspect with limit: 500 shows all tracks.
- All track-level tool results include the full file path for each track. Use these paths with target_scope "explicit_paths" in mutating tools to target specific files.

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

type AssistantTaskContractKind =
  | "read_only_answer"
  | "action_preview_required"
  | "clarification_required"
  | "chat_only";

interface AssistantTaskRoute {
  toolName: string;
  args: Record<string, unknown>;
}

interface AssistantTaskContract {
  kind: AssistantTaskContractKind;
  route?: AssistantTaskRoute;
  reason: string;
  requiresCompletionEvidence: boolean;
}

function hasTrackNumberIntent(text: string): boolean {
  return /\b(track\s*(number|numbers|#)|tracknumber|tracktotal|renumber|renumbering|numbering)\b/i.test(text)
    || /number(?:ed|ing)?\s+within\s+(each\s+)?album/i.test(text)
    || /within\s+(each\s+)?album/i.test(text) && /\bnumber/i.test(text);
}

function hasExplicitFileMoveIntent(text: string): boolean {
  return /\b(move|moving|organize|organise|folder|folders|directory|directories|rename|relocate)\b/i.test(text);
}

function hasExtractTagValueIntent(text: string): boolean {
  // Detect requests to clean/extract values from tag fields.
  // e.g. "remove number and - from Album", "strip suffix from title", or "clean album tag value"
  const TAG_FIELDS = /\b(album|title|artist|genre|year|composer|comment|description|artists)\b/i;
  const EXTRACT_VERBS = /\b(remove|strip|extract|clear|delete|clean|cleanup)\b/i;
  return EXTRACT_VERBS.test(text) && TAG_FIELDS.test(text);
}

function buildExtractTagValueRoute(text: string): AssistantTaskRoute | undefined {
  const fieldMatch = /\b(album|title|artist|artists|genre|year|composer|comment|description)\b(?:\s+tag)?/i.exec(text);
  if (!fieldMatch) return undefined;

  const isValueCleanup = /\b(value|tag)\b/i.test(text);
  const mentionsNumberish = /\b(number|numbers|numeric|digit|digits)\b/i.test(text);
  const mentionsSeparator = /[-–—.]|\bdash\b|\bdashes\b|\bdot\b|\bdots\b/i.test(text);
  const mentionsPrefix = /\b(prefix|leading|start|beginning)\b/i.test(text);
  const mentionsSuffix = /\b(suffix|trailing|ending|end)\b/i.test(text);
  if (!isValueCleanup && !mentionsNumberish && !mentionsSeparator && !mentionsPrefix && !mentionsSuffix) {
    return undefined;
  }

  let pattern = "^(?:\\d+[\\s.\\)\\-–—]+)?(.+?)(?:[\\s.\\)\\-–—]+\\d+)?$";
  if (mentionsPrefix && !mentionsSuffix) {
    pattern = "^\\d+[\\s.\\)\\-–—]+(.+)$";
  } else if (mentionsSuffix && !mentionsPrefix) {
    pattern = "^(.+?)[\\s.\\)\\-–—]+\\d+$";
  }

  return {
    toolName: "extract_tag_value",
    args: {
      target_scope: "library",
      field: fieldMatch[1].toLowerCase(),
      pattern,
      group_index: 1,
    },
  };
}

function hasGeneralActionIntent(text: string): boolean {
  return /\b(apply|change|fix|update|edit|set|write|audit|number|renumber|infer|parse|strip|organize|organise|move|run)\b/i.test(text)
    || /^\s*tag\b/i.test(text);
}

function hasReadOnlyIntent(text: string): boolean {
  return /\b(summarize|summary|find|search|list|show|inspect|what|which|how many|count|missing|duplicate|duplicates)\b/i.test(text);
}

export function deriveAssistantTaskContract(userMessage: string): AssistantTaskContract {
  const text = userMessage.trim();
  const normalized = text.toLowerCase();

  if (normalized.length === 0) {
    return {
      kind: "clarification_required",
      reason: "empty_user_message",
      requiresCompletionEvidence: false,
    };
  }

  if (/^(number|renumber|track\s*numbers?|fix\s+track\s+numbers?|number\s+tracks)$/i.test(text) || hasTrackNumberIntent(text)) {
    return {
      kind: "action_preview_required",
      route: { toolName: "auto_numbering_tracks", args: { target_scope: "library" } },
      reason: "track_numbering_intent",
      requiresCompletionEvidence: true,
    };
  }

  if (/\b(auto[-\s]?tag|tag this|fill tags|fill missing tags)\b/i.test(text)) {
    return {
      kind: "action_preview_required",
      route: { toolName: "run_library_task", args: { task: "auto_tag", target_scope: "library" } },
      reason: "auto_tag_intent",
      requiresCompletionEvidence: true,
    };
  }

  if (/\b(audit|check missing|check metadata|scan metadata)\b/i.test(text)) {
    return {
      kind: "action_preview_required",
      route: { toolName: "run_library_task", args: { task: "audit", target_scope: "library" } },
      reason: "audit_intent",
      requiresCompletionEvidence: true,
    };
  }

  if (/\b(infer|parse)\b.*\b(filename|filenames)\b/i.test(text)
    || /\b(filename|filenames)\b.*\b(title|artist|artists)\b/i.test(text)) {
    return {
      kind: "action_preview_required",
      route: { toolName: "infer_tags_from_filenames", args: { target_scope: "library" } },
      reason: "filename_inference_intent",
      requiresCompletionEvidence: true,
    };
  }

  if (hasExtractTagValueIntent(text)) {
    const route = buildExtractTagValueRoute(text);
    return {
      kind: "action_preview_required",
      route,
      reason: "extract_tag_value_intent",
      requiresCompletionEvidence: true,
    };
  }

  if (hasGeneralActionIntent(text)) {
    return {
      kind: "action_preview_required",
      reason: "general_action_intent",
      requiresCompletionEvidence: true,
    };
  }

  if (hasReadOnlyIntent(text)) {
    return {
      kind: "read_only_answer",
      reason: "read_only_intent",
      requiresCompletionEvidence: false,
    };
  }

  return {
    kind: "chat_only",
    reason: "no_action_or_read_only_intent",
    requiresCompletionEvidence: false,
  };
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
  private maxSteps = 10;
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

  private emitError(message: string, metadata?: Record<string, unknown>): AssistantEvent {
    this.logger.recordEntry({
      sessionUuid: this.sessionId,
      entryType: "system",
      content: message,
      metadata,
    });
    const event: AssistantEvent = {
      sessionId: this.sessionId,
      type: "error",
      message,
      data: metadata,
    };
    this.emit(event);
    return event;
  }

  private isToolArgumentValidationError(toolResult: AssistantToolResult): boolean {
    if (toolResult.ok || !toolResult.error) return false;
    return /^(Missing required field|Unknown field|Field ".+" should be|Field ".+" should be one of)/.test(toolResult.error);
  }

  private async executeToolCall(input: {
    userMessage: string;
    toolName: string;
    toolArgs: Record<string, unknown>;
    contract: AssistantTaskContract;
    source: "llm" | "deterministic";
  }): Promise<{ event?: AssistantEvent; continueLoop: boolean; toolResult?: AssistantToolResult }> {
    const { userMessage, toolName, toolArgs, contract, source } = input;

    this.logger.recordEntry({
      sessionUuid: this.sessionId,
      entryType: "tool_call",
      content: JSON.stringify({ toolName, toolArgs, source }),
      metadata: { toolName, args: toolArgs, source, taskContract: contract },
    });

    this.conversation.push({
      role: "assistant",
      content: JSON.stringify({ type: "tool_call", toolName, args: toolArgs, reason: source }),
    });

    this.emit({
      sessionId: this.sessionId,
      type: "tool_running",
      message: `Running tool: ${toolName}`,
      data: { toolName, toolArgs, source },
    });

    const tool = this.registry.get(toolName);
    if (!tool) {
      this.conversation.push({
        role: "user",
        content: `Error: Unknown tool: ${toolName}. Available tools: ${this.registry.getAll().map((t) => t.name).join(", ")}`,
      });
      return {
        continueLoop: source === "deterministic",
        event: source === "deterministic"
          ? undefined
          : this.emitError(`Unknown tool: ${toolName}`, { toolName, source, taskContract: contract }),
      };
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
        metadata: { toolName, args: toolArgs, ok: toolResult.ok, blocked: true, source, taskContract: contract },
      });
      this.conversation.push({ role: "user", content: toolResult.summary });
      this.emit({
        sessionId: this.sessionId,
        type: "tool_result",
        message: toolResult.summary,
        data: toolResult,
      });
      return { continueLoop: true, toolResult };
    }

    const toolResult = await this.registry.execute(toolName, toolArgs);

    this.logger.recordEntry({
      sessionUuid: this.sessionId,
      entryType: "tool_result",
      content: toolResult.summary,
      metadata: {
        toolName,
        args: toolArgs,
        ok: toolResult.ok,
        source,
        taskContract: contract,
        completionEvidence: Boolean(toolResult.pendingActionBatchId),
      },
    });

    if (this.isToolArgumentValidationError(toolResult)) {
      this.conversation.push({ role: "user", content: toolResult.summary });
      this.emit({
        sessionId: this.sessionId,
        type: "tool_result",
        message: toolResult.summary,
        data: toolResult,
      });
      return { continueLoop: true, toolResult };
    }

    if (!tool.isReadOnly && !this.autonomous) {
      this.conversation.push({ role: "user", content: toolResult.summary });
      this.emit({
        sessionId: this.sessionId,
        type: "tool_result",
        message: toolResult.summary,
        data: toolResult,
      });

      if (toolResult.pendingActionBatchId) {
        const event: AssistantEvent = {
          sessionId: this.sessionId,
          type: "action_batch_created",
          message: toolResult.summary,
          data: { actionBatchId: toolResult.pendingActionBatchId, toolResult },
        };
        this.emit(event);
        return { event, continueLoop: false, toolResult };
      }

      if (contract.requiresCompletionEvidence) {
        return {
          event: this.emitError(
            `No action was performed. The assistant selected "${toolName}", but it did not create a preview batch: ${toolResult.summary}`,
            { toolName, args: toolArgs, source, taskContract: contract, toolResult },
          ),
          continueLoop: false,
          toolResult,
        };
      }

      return { continueLoop: true, toolResult };
    }

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
    return { continueLoop: true, toolResult };
  }

  /**
   * Send a user message and process the assistant loop.
   */
  async send(userMessage: string): Promise<AssistantEvent> {
    this.cancelled = false;
    this.conversation.push({ role: "user", content: userMessage });

    let repeatedCalls: { toolName: string; callCount: number } | null = null;
    let repairedInvalidArgs = false;
    const contract = deriveAssistantTaskContract(userMessage);

    this.logger.recordEntry({
      sessionUuid: this.sessionId,
      entryType: "system",
      content: `Assistant task contract: ${contract.kind}`,
      metadata: { taskContract: contract },
    });

    if (contract.route && this.registry.get(contract.route.toolName)) {
      this.emit({
        sessionId: this.sessionId,
        type: "step",
        message: `Deterministic route: ${contract.route.toolName}`,
      });
      const routed = await this.executeToolCall({
        userMessage,
        toolName: contract.route.toolName,
        toolArgs: contract.route.args,
        contract,
        source: "deterministic",
      });
      if (routed.event) return routed.event;
    }

    for (let step = 0; step < this.maxSteps; step++) {
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
        return this.emitError(
          `The assistant repeated "${detected.toolName}" with the same arguments ${detected.callCount} times, so I stopped instead of claiming the task was complete.`,
          { repeatedCalls, taskContract: contract },
        );
      }

      this.emit({
        sessionId: this.sessionId,
        type: "step",
        message: `Step ${step + 1}/${this.maxSteps}`,
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
        return this.emitError("No response from assistant", { taskContract: contract });
      }

      if (result.stoppedEarly && result.reason && result.reason !== "awaiting_tool_execution") {
        return this.emitError(
          lastStep.content || `Assistant stopped early: ${result.reason}`,
          { reason: result.reason, taskContract: contract },
        );
      }

      if (lastStep.type === "message") {
        this.conversation.push({ role: "assistant", content: lastStep.content });
        if (contract.requiresCompletionEvidence) {
          return this.emitError(
            `No action was performed. This request requires a preview batch, but the assistant only replied: ${lastStep.content}`,
            { taskContract: contract, finalMessage: lastStep.content },
          );
        }
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

        const tool = this.registry.get(toolName);
        const mismatch = tool ? detectToolIntentMismatch({
          userMessage,
          toolName,
          operationKind: tool.operationKind,
        }) : null;

        const executed = await this.executeToolCall({
          userMessage,
          toolName,
          toolArgs,
          contract,
          source: "llm",
        });
        if (executed.event) return executed.event;

        if (executed.toolResult && this.isToolArgumentValidationError(executed.toolResult)) {
          if (!repairedInvalidArgs) {
            repairedInvalidArgs = true;
            const repairHint =
              `Tool argument validation failed for "${toolName}": ${executed.toolResult.error}. ` +
              `Retry once with only fields allowed by that tool schema.`;
            this.logger.recordEntry({
              sessionUuid: this.sessionId,
              entryType: "system",
              content: repairHint,
              metadata: { toolName, retryReason: "invalid_tool_args", taskContract: contract },
            });
            this.conversation.push({ role: "system", content: repairHint });
            continue;
          }
          return this.emitError(
            `Tool argument validation failed after retry for "${toolName}": ${executed.toolResult.error}`,
            { toolName, args: toolArgs, retryReason: "invalid_tool_args", taskContract: contract },
          );
        }

        if (executed.toolResult && !executed.toolResult.ok && !mismatch) {
          return this.emitError(
            executed.toolResult.summary,
            { toolName, args: toolArgs, taskContract: contract, toolResult: executed.toolResult },
          );
        }
      }
    }

    // Max steps reached without final message
    // Build diagnostic info: last few conversation entries + repeated calls
    const recentEntries = this.conversation
      .slice(-4)
      .map((m) => `${m.role}: ${m.content.slice(0, 200)}`)
      .join("\n");
    let diagMsg =
      `I reached the maximum step limit (${this.maxSteps}) and couldn't complete the task in one response. ` +
      `You can try rephrasing your request or doing less per message. ` +
      `Common fixes:\n` +
      `- For per-track title/artist fixes: ask to \"infer tags from filenames\" (parses \"Artist - Title.ext\" patterns)\n` +
      `- For track numbering: ask to \"fix track numbers\" (uses auto_numbering_tracks)\n` +
      `- For stripping number prefixes from titles: ask to \"strip title prefixes\"\n` +
      `- Be more specific: provide exact file paths or a narrower filter`;
    diagMsg += `\n\nLast conversation entries:\n${recentEntries}`;
    return this.emitError(diagMsg, { reason: "max_steps_reached", taskContract: contract });
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
    this.maxSteps = 10;
    console.log(`[AssistantRuntime] Session reset, new id: ${this.sessionId}`);
  }

  private generateSessionId(): string {
    const id = `session-${Date.now()}-${Math.random().toString(36).slice(2, 9)}`;
    console.log(`[AssistantRuntime] Created session id: ${id}`);
    return id;
  }
}
