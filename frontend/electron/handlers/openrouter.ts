/**
 * OpenRouter API client for LLM chat completions.
 * Ported from Python auto_tagger.llm.client + auto_tagger.llm.cost.
 *
 * Pure fetch() — no external dependencies.
 */

import debug from "./debug";

const OPENROUTER_BASE = "https://openrouter.ai/api/v1";
const RETRYABLE_STATUSES = new Set([429, 500, 502, 503, 504]);

export interface TokenUsage {
  promptTokens: number;
  completionTokens: number;
  totalTokens: number;
}

export interface LLMResponse {
  data: Record<string, unknown>;
  usage: TokenUsage;
  model: string;
}

export interface OpenRouterConfig {
  apiKey: string;
  baseUrl?: string;
  model?: string;
  temperature?: number;
  maxTokens?: number;
}

/**
 * Cost estimates for common models (USD per 1K tokens).
 * Source: openrouter.ai/models as of 2026-05.
 */
const MODEL_COST_RATES: Record<string, { prompt: number; completion: number }> = {
  "deepseek/deepseek-chat": { prompt: 0.00014, completion: 0.00028 },
  "openrouter/owl-alpha": { prompt: 0, completion: 0 },
  "deepseek/deepseek-v4-flash:free": { prompt: 0, completion: 0 },
  "anthropic/claude-3.5-haiku": { prompt: 0.0008, completion: 0.004 },
  "google/gemini-2.0-flash-lite": { prompt: 0.000075, completion: 0.0003 },
};

/**
 * Estimate the cost of a model call in USD.
 */
export function estimateCost(
  model: string,
  usage: TokenUsage,
): number {
  const rates = MODEL_COST_RATES[model] ?? MODEL_COST_RATES["deepseek/deepseek-chat"];
  return (
    (usage.promptTokens * rates.prompt + usage.completionTokens * rates.completion) /
    1000
  );
}

/**
 * Format a cost value for display.
 */
export function formatCost(cost: number): string {
  if (cost < 0.001) return `$${(cost * 1000).toFixed(2)}/1K`;
  if (cost < 0.01) return `$${cost.toFixed(4)}`;
  return `$${cost.toFixed(3)}`;
}

export class OpenRouterClient {
  private config: OpenRouterConfig;

  constructor(config: OpenRouterConfig) {
    this.config = {
      ...config,
      baseUrl: config.baseUrl ?? OPENROUTER_BASE,
      // model comes from ...config; no fallback — must be provided by the caller
      temperature: config.temperature ?? 0.7,
      maxTokens: config.maxTokens ?? 1024,
    };
  }

  /**
   * Call OpenRouter and parse structured JSON content.
   */
  async completeJson(
    messages: Array<{ role: string; content: string }>,
    schemaName: string,
    schema: Record<string, unknown>,
    model?: string,
  ): Promise<LLMResponse> {
    let payload: Record<string, unknown> | null = null;
    let content = "";
    let parseError: Error | null = null;

    for (let attempt = 0; attempt < 2; attempt++) {
      const response = await this.postWithRetries(messages, schemaName, schema, model);
      const responsePayload = await response.json() as Record<string, unknown>;
      payload = responsePayload;

      const choices = (responsePayload as any).choices ?? [];
      if (choices.length === 0) {
        throw new Error("OpenRouter response did not include choices");
      }
      content = choices[0]?.message?.content ?? "";
      if (!content) {
        throw new Error("OpenRouter response did not include message content");
      }
      content = String(content);

      try {
        const data = JSON.parse(content) as Record<string, unknown>;
        return this.buildResponse(responsePayload, data, model);
      } catch (err) {
        parseError = err as Error;
      }
    }

    const choices = ((payload ?? {}) as any).choices ?? [];
    const finishReason = choices[0]?.finish_reason;
    const reason = finishReason ? ` (finish_reason=${String(finishReason)})` : "";
    throw new Error(`LLM returned malformed JSON${reason}: ${parseError?.message ?? "unknown parse error"}`);
  }

  private buildResponse(
    payload: Record<string, unknown>,
    data: Record<string, unknown>,
    model?: string,
  ): LLMResponse {
    const usageRaw = (payload as any).usage ?? {};
    const usage: TokenUsage = {
      promptTokens: Number(usageRaw.prompt_tokens ?? 0),
      completionTokens: Number(usageRaw.completion_tokens ?? 0),
      totalTokens: Number(usageRaw.total_tokens ?? 0),
    };

    return {
      data,
      usage,
      model: String((payload as any).model ?? model ?? this.config.model),
    };
  }

  private async postWithRetries(
    messages: Array<{ role: string; content: string }>,
    schemaName: string,
    schema: Record<string, unknown>,
    model?: string,
    maxRetries = 2,
  ): Promise<Response> {
    let lastResponse: Response | null = null;
    let lastError: Error | null = null;

    for (let attempt = 0; attempt <= maxRetries; attempt++) {
      try {
        const response = await this.post(messages, schemaName, schema, model);
        lastResponse = response;

        if (response.ok) return response;

        // OpenRouter returns HTTP 400 when the upstream provider fails.
        // The provider error is transient — retry (other providers may work).
        const isProviderError = response.status === 400 &&
          (await response.clone().text().catch(() => "")).includes('provider_name');

        if (isProviderError) {
          debug.debug("openrouter", `Provider error (attempt ${attempt + 1}/${maxRetries + 1}), retrying...`);
          await new Promise((r) => setTimeout(r, 1000 * (attempt + 1)));
          continue;
        }

        if (!RETRYABLE_STATUSES.has(response.status) || attempt >= maxRetries) {
          break;
        }

        // Wait before retry
        await new Promise((r) => setTimeout(r, 250 * (attempt + 1)));
      } catch (err) {
        // Network-level failure (DNS, TLS, connection reset) — retry
        lastError = err instanceof Error ? err : new Error(String(err));
        if (attempt >= maxRetries) break;
        await new Promise((r) => setTimeout(r, 250 * (attempt + 1)));
      }
    }

    if (lastResponse) {
      const body = await lastResponse.text().catch(() => "");
      throw new Error(
        `OpenRouter request failed with HTTP ${lastResponse.status}: ${body}`,
      );
    }

    throw lastError ?? new Error("OpenRouter request failed: no response and no error recorded");
  }

  private async post(
    messages: Array<{ role: string; content: string }>,
    schemaName: string,
    schema: Record<string, unknown>,
    model?: string,
  ): Promise<Response> {
    const url = `${this.config.baseUrl!.replace(/\/$/, "")}/chat/completions`;

    const body: Record<string, unknown> = {
      model: model ?? this.config.model,
      messages,
      temperature: this.config.temperature,
      max_tokens: this.config.maxTokens,
      response_format: {
        type: "json_schema",
        json_schema: {
          name: schemaName,
          schema,
        },
      },
    };

    return fetch(url, {
      method: "POST",
      headers: {
        Authorization: `Bearer ${this.config.apiKey}`,
        "Content-Type": "application/json",
      },
      body: JSON.stringify(body),
    });
  }

}
