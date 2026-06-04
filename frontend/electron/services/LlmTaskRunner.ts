/**
 * LlmTaskRunner — lightweight orchestration over OpenRouterClient.
 *
 * Provides two paths:
 * - runStructuredTask: one-shot structured JSON for existing LLM features.
 * - runToolLoop: conversational tool loop for the assistant.
 *
 * Both share config loading, retry behavior, and cost accounting.
 */

import { OpenRouterClient, type TokenUsage, type LLMResponse, estimateCost, formatCost } from "../handlers/openrouter";

export interface LlmTaskConfig {
  apiKey: string;
  model?: string;
  baseUrl?: string;
  temperature?: number;
  maxTokens?: number;
}

export interface StructuredTaskInput {
  taskName: string;
  messages: Array<{ role: string; content: string }>;
  schemaName: string;
  schema: Record<string, unknown>;
  model?: string;
  maxTokens?: number;
}

export interface StructuredTaskResult<T> {
  data: T;
  usage: TokenUsage;
  model: string;
  cost: number;
  costFormatted: string;
}

export interface AssistantToolDef {
  name: string;
  description: string;
  inputSchema: Record<string, unknown>;
}

export interface AssistantLoopInput {
  systemPrompt: string;
  messages: Array<{ role: string; content: string }>;
  tools: AssistantToolDef[];
  model?: string;
  maxTokens?: number;
  maxSteps?: number;
}

export interface AssistantLoopResult {
  finalMessage: string;
  steps: AssistantLoopStep[];
  usage: TokenUsage;
  model: string;
  cost: number;
  costFormatted: string;
  stoppedEarly: boolean;
  reason?: string;
}

export interface AssistantLoopStep {
  type: "message" | "tool_call" | "tool_result";
  content: string;
  toolName?: string;
  toolArgs?: Record<string, unknown>;
  toolResult?: string;
}

/** Shape of the parsed LLM response JSON. */
interface ParsedResponse {
  type: string;
  content: string;
  toolName?: string;
  args?: Record<string, unknown>;
  reason?: string;
}

/**
 * Redact sensitive values from strings.
 * Replaces API keys, tokens, and bearer auth values with [REDACTED].
 */
export function redactSensitive(text: string): string {
  return text
    .replace(/(sk-or-[a-zA-Z0-9]{20,})/g, "[REDACTED-API-KEY]")
    .replace(/(Bearer\s+)[a-zA-Z0-9_-]{20,}/gi, "$1[REDACTED]")
    .replace(/(token=)[a-zA-Z0-9]{20,}/gi, "$1[REDACTED]");
}

export type ApiCallEvent = {
  /** Human-readable label, e.g. 'structured', 'tool_loop', 'continue_tool_loop' */
  type: string;
  /** Messages sent to the LLM (the prompt). */
  messages: Array<{ role: string; content: string }>;
  /** Parsed response data. */
  data: Record<string, unknown>;
  /** Model used for the call. */
  model: string;
  /** Token usage. */
  promptTokens: number;
  completionTokens: number;
  totalTokens: number;
  /** Estimated cost in USD. */
  cost: number;
  /** Duration of the API call in ms. */
  durationMs: number;
};

export type ApiCallCallback = (event: ApiCallEvent) => void;

export class LlmTaskRunner {
  private config: LlmTaskConfig;
  private client: OpenRouterClient;
  private apiCallCallbacks: ApiCallCallback[] = [];

  constructor(config: LlmTaskConfig) {
    this.config = {
      ...config,
      model: config.model ?? "",
      temperature: config.temperature ?? 0.7,
      maxTokens: config.maxTokens ?? 1024,
    };
    this.client = new OpenRouterClient({
      apiKey: this.config.apiKey,
      baseUrl: this.config.baseUrl,
      model: this.config.model,
      temperature: this.config.temperature,
      maxTokens: this.config.maxTokens,
    });
  }

  /**
   * Subscribe to API call events for logging/monitoring.
   * Returns an unsubscribe function.
   */
  onApiCall(callback: ApiCallCallback): () => void {
    this.apiCallCallbacks.push(callback);
    return () => {
      const idx = this.apiCallCallbacks.indexOf(callback);
      if (idx >= 0) this.apiCallCallbacks.splice(idx, 1);
    };
  }

  private notifyApiCall(
    type: string,
    messages: Array<{ role: string; content: string }>,
    response: LLMResponse,
    cost: number,
    durationMs: number,
  ): void {
    const event: ApiCallEvent = {
      type,
      messages,
      data: response.data,
      model: response.model,
      promptTokens: response.usage.promptTokens,
      completionTokens: response.usage.completionTokens,
      totalTokens: response.usage.totalTokens,
      cost,
      durationMs,
    };
    for (const cb of this.apiCallCallbacks) {
      try { cb(event); } catch { /* ignore */ }
    }
  }

  private buildToolCallSchema(tools: AssistantToolDef[]): Record<string, unknown> {
    const toolNames = tools.map((t) => t.name);
    return {
      type: "object",
      properties: {
        type: { type: "string", enum: ["message", "tool_call"] },
        content: { type: "string" },
        toolName: { type: "string", enum: toolNames },
        args: { type: "object" },
        reason: { type: "string" },
      },
      required: ["type", "content"],
    };
  }

  /**
   * Resolve the tool name from a parsed response.
   * Sometimes the LLM puts the tool name in `content` instead of `toolName`.
   * Falls back to matching content against known tool names.
   */
  private resolveToolName(
    parsed: { toolName?: string; content?: string },
    tools: AssistantToolDef[],
  ): string {
    if (parsed.toolName) return parsed.toolName;
    const content = parsed.content ?? "";
    // The content often starts with or is exactly a tool name
    const matched = tools.find(
      (t) => t.name === content.trim() || content.trim().startsWith(t.name),
    );
    return matched?.name ?? "unknown";
  }

  /**
   * Run a structured JSON task and parse the result.
   * Throws on missing API key, malformed response, or network error.
   */
  async runStructuredTask<T>(
    input: StructuredTaskInput,
  ): Promise<StructuredTaskResult<T>> {
    if (!this.config.apiKey) {
      throw new Error("LLM_API_KEY_NOT_CONFIGURED");
    }

    const startTime = performance.now();
    const response = await this.client.completeJson(
      input.messages,
      input.schemaName,
      input.schema,
      input.model,
    );
    const durationMs = Math.round(performance.now() - startTime);

    const data = response.data as unknown as T;
    const cost = estimateCost(response.model, response.usage);

    this.notifyApiCall("structured", input.messages, response, cost, durationMs);

    return {
      data,
      usage: response.usage,
      model: response.model,
      cost,
      costFormatted: formatCost(cost),
    };
  }

  /**
   * Run an assistant tool loop.
   * Sends messages, processes tool calls, and returns the final result.
   * Uses a simple text-in/text-out approach with tool definitions in the system prompt.
   */
  async runToolLoop(input: AssistantLoopInput): Promise<AssistantLoopResult> {
    if (!this.config.apiKey) {
      throw new Error("LLM_API_KEY_NOT_CONFIGURED");
    }

    const maxSteps = input.maxSteps ?? 6;
    const steps: AssistantLoopStep[] = [];
    let totalPromptTokens = 0;
    let totalCompletionTokens = 0;
    let currentModel = input.model ?? this.config.model ?? "unknown";
    let stoppedEarly = false;
    let reason: string | undefined;

    // Build the full system prompt with tool definitions
    const toolDescriptions = input.tools
      .map(
        (tool) =>
          `- ${tool.name}: ${tool.description}\n  Input schema: ${JSON.stringify(tool.inputSchema)}`,
      )
      .join("\n");

    const fullSystemPrompt = `${input.systemPrompt}\n\nAvailable tools:\n${toolDescriptions}\n\nRespond with either:\n1. A natural language message (no JSON wrapper)\n2. A tool call in JSON format: {"type":"tool_call","toolName":"...","args":{...},"reason":"..."}`;

    // Build the conversation messages
    const conversation: Array<{ role: string; content: string }> = [
      { role: "system", content: fullSystemPrompt },
      ...input.messages,
    ];

    const toolCallSchema = this.buildToolCallSchema(input.tools);

    let step = 0;
    for (step = 0; step < maxSteps; step++) {
      const startTime = performance.now();
      const response = await this.client.completeJson(
        conversation,
        "assistant_response",
        toolCallSchema,
        input.model,
      );
      const durationMs = Math.round(performance.now() - startTime);

      totalPromptTokens += response.usage.promptTokens;
      totalCompletionTokens += response.usage.completionTokens;
      currentModel = response.model;

      const stepCost = estimateCost(response.model, response.usage);
      this.notifyApiCall("tool_loop", conversation, response, stepCost, durationMs);

      const parsed = response.data as unknown as ParsedResponse;

      if (parsed.type === "message" || parsed.type === "final") {
        steps.push({
          type: "message",
          content: parsed.content,
        });
        break;
      }

      if (parsed.type === "tool_call") {
        const toolName = this.resolveToolName(parsed, input.tools);
        // When the tool name can't be resolved, the LLM is actually sending
        // a message (the content/reason is its response). Treat as a message.
        if (toolName === "unknown") {
          steps.push({
            type: "message",
            content: parsed.reason || parsed.content || "I couldn't determine the right tool.",
          });
          break;
        }
        const args = parsed.args ?? {};
        steps.push({
          type: "tool_call",
          content: parsed.content ?? `Calling ${toolName}`,
          toolName,
          toolArgs: args,
        });
        conversation.push({
          role: "assistant",
          content: JSON.stringify(parsed),
        });
        conversation.push({
          role: "user",
          content: `[Tool result for ${toolName} will be provided by the caller via executeTool]`,
        });
        // The caller must handle tool execution by calling executeTool
        // We stop after detecting the tool call so the caller can feed results back
        stoppedEarly = true;
        reason = "awaiting_tool_execution";
        break;
      }

      // Unexpected type
      steps.push({
        type: "message",
        content: `Unexpected response: ${JSON.stringify(parsed)}`,
      });
      stoppedEarly = true;
      reason = "unexpected_response_type";
      break;
    }

    if (steps.length === 0) {
      steps.push({
        type: "message",
        content: "No response generated.",
      });
    }

    if (step >= maxSteps && reason !== "awaiting_tool_execution") {
      stoppedEarly = true;
      reason = "max_steps_reached";
    }

    const totalUsage: TokenUsage = {
      promptTokens: totalPromptTokens,
      completionTokens: totalCompletionTokens,
      totalTokens: totalPromptTokens + totalCompletionTokens,
    };

    const cost = estimateCost(currentModel, totalUsage);

    return {
      finalMessage: steps.filter((s) => s.type === "message").map((s) => s.content).join("\n"),
      steps,
      usage: totalUsage,
      model: currentModel,
      cost,
      costFormatted: formatCost(cost),
      stoppedEarly,
      reason,
    };
  }


}
