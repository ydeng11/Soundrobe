/**
 * LlmTaskRunner — lightweight orchestration over OpenRouterClient.
 *
 * Provides two paths:
 * - runStructuredTask: one-shot structured JSON for existing LLM features.
 * - runToolLoop: conversational tool loop for the assistant.
 *
 * Both share config loading, retry behavior, and cost accounting.
 */

import { OpenRouterClient, type TokenUsage, estimateCost, formatCost } from "../handlers/openrouter";

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

export class LlmTaskRunner {
  private config: LlmTaskConfig;
  private client: OpenRouterClient;

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
   * Run a structured JSON task and parse the result.
   * Throws on missing API key, malformed response, or network error.
   */
  async runStructuredTask<T>(
    input: StructuredTaskInput,
  ): Promise<StructuredTaskResult<T>> {
    if (!this.config.apiKey) {
      throw new Error("LLM_API_KEY_NOT_CONFIGURED");
    }

    const response = await this.client.completeJson(
      input.messages,
      input.schemaName,
      input.schema,
      input.model,
    );

    const data = response.data as unknown as T;
    const cost = estimateCost(response.model, response.usage);

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

    // Build tool-name enum so the LLM can only produce valid tool names
    const toolNames = input.tools.map((t) => t.name);
    const toolCallSchema = {
      type: "object",
      properties: {
        type: {
          type: "string",
          enum: ["message", "tool_call"],
        },
        content: { type: "string" },
        toolName: { type: "string", enum: toolNames },
        args: { type: "object" },
        reason: { type: "string" },
      },
      required: ["type", "content"],
    };

    let step = 0;
    for (step = 0; step < maxSteps; step++) {
      const response = await this.client.completeJson(
        conversation,
        "assistant_response",
        toolCallSchema,
        input.model,
      );

      totalPromptTokens += response.usage.promptTokens;
      totalCompletionTokens += response.usage.completionTokens;
      currentModel = response.model;

      const parsed = response.data as {
        type: string;
        content: string;
        toolName?: string;
        args?: Record<string, unknown>;
        reason?: string;
      };

      if (parsed.type === "message" || parsed.type === "final") {
        steps.push({
          type: "message",
          content: parsed.content,
        });
        break;
      }

      if (parsed.type === "tool_call") {
        const toolName = parsed.toolName ?? "unknown";
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

  /**
   * Continue a tool loop after executing a tool call.
   * The caller provides the tool result as a new message in the conversation.
   */
  async continueToolLoop(
    conversation: Array<{ role: string; content: string }>,
    toolResult: { toolName: string; result: string },
    tools: AssistantToolDef[],
    model?: string,
    maxSteps: number = 6,
    step: number = 0,
  ): Promise<{
    steps: AssistantLoopStep[];
    finalMessage: string;
    stoppedEarly: boolean;
    reason?: string;
  }> {
    // Add the tool result to the conversation
    conversation.push({
      role: "user",
      content: `Tool "${toolResult.toolName}" returned: ${toolResult.result}`,
    });

    // Build tool-name enum so the LLM can only produce valid tool names
    const toolNames = tools.map((t) => t.name);
    const toolCallSchema = {
      type: "object",
      properties: {
        type: {
          type: "string",
          enum: ["message", "tool_call"],
        },
        content: { type: "string" },
        toolName: { type: "string", enum: toolNames },
        args: { type: "object" },
        reason: { type: "string" },
      },
      required: ["type", "content"],
    };

    // Continue the loop
    for (let i = step; i < maxSteps; i++) {
      const response = await this.client.completeJson(
        conversation,
        "assistant_response",
        toolCallSchema,
        model,
      );

      const parsed = response.data as {
        type: string;
        content: string;
        toolName?: string;
        args?: Record<string, unknown>;
        reason?: string;
      };

      const steps: AssistantLoopStep[] = [];

      if (parsed.type === "message" || parsed.type === "final") {
        steps.push({ type: "message", content: parsed.content });
        return { steps, finalMessage: parsed.content, stoppedEarly: false };
      }

      if (parsed.type === "tool_call") {
        steps.push({
          type: "tool_call",
          content: parsed.content ?? `Calling ${parsed.toolName}`,
          toolName: parsed.toolName,
          toolArgs: parsed.args,
        });
        return {
          steps,
          finalMessage: "",
          stoppedEarly: true,
          reason: "awaiting_tool_execution",
        };
      }
    }

    return {
      steps: [],
      finalMessage: "Reached maximum tool steps",
      stoppedEarly: true,
      reason: "max_steps_reached",
    };
  }

  /**
   * Get the raw OpenRouterClient for advanced use.
   * Only for backward compatibility during migration.
   */
  getClient(): OpenRouterClient {
    return this.client;
  }
}
