/**
 * OpenRouter API client for LLM chat completions.
 * Ported from Python auto_tagger.llm.client + auto_tagger.llm.cost.
 *
 * Pure fetch() — no external dependencies.
 */

import debug from "./debug";

const OPENROUTER_BASE = "https://openrouter.ai/api/v1";
const RETRYABLE_STATUSES = new Set([429, 500, 502, 503, 504]);

/** Default total deadline (covering retries) for a single LLM call. */
const DEFAULT_LLM_TIMEOUT_MS = 30_000;

/** Sleep that resolves early if the signal aborts. Resolves immediately for ms <= 0. */
function abortableSleep(ms: number, signal?: AbortSignal): Promise<void> {
  return new Promise((resolve) => {
    if (ms <= 0 || signal?.aborted) {
      resolve();
      return;
    }
    let timer: ReturnType<typeof setTimeout> | undefined;
    const onAbort = () => {
      if (timer) clearTimeout(timer);
      resolve();
    };
    timer = setTimeout(() => {
      signal?.removeEventListener("abort", onAbort);
      resolve();
    }, ms);
    signal?.addEventListener("abort", onAbort, { once: true });
  });
}

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

export interface CompleteJsonOptions {
  allowMessageFallback?: boolean;
  /** Caller AbortSignal (e.g. task cancellation). Aborts are surfaced as abort errors. */
  signal?: AbortSignal;
  /** Total deadline in ms covering all retries within one postWithRetries call. Default: 30s. */
  timeoutMs?: number;
  /**
   * Per-call OpenRouter `reasoning` control, e.g. `{ max_tokens: 256 }` to cap
   * chain-of-thought or `{ enabled: false }` to disable it. Only included in the
   * request body when provided, so unrelated callers (audit, assistant) are
   * unaffected.
   */
  reasoning?: Record<string, unknown>;
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
   *
   * Structured callers fail loud on malformed content by default.
   * Assistant chat/tool-loop callers may opt in to natural-language fallback.
   */
  async completeJson(
    messages: Array<{ role: string; content: string }>,
    schemaName: string,
    schema: Record<string, unknown>,
    model?: string,
    options: CompleteJsonOptions = {},
  ): Promise<LLMResponse> {
    let payload: Record<string, unknown> | null = null;
    let content = "";
    let parseError: Error | null = null;
    // One total deadline shared across both JSON-repair attempts and all retries.
    const totalBudget = options.timeoutMs ?? DEFAULT_LLM_TIMEOUT_MS;
    const deadline = Date.now() + totalBudget;

    for (let attempt = 0; attempt < 2; attempt++) {
      const remaining = deadline - Date.now();
      if (remaining <= 0) break;

      // If the previous attempt was truncated (finish_reason=length), double
      // the token budget so the model can complete its JSON output.
      const maxTokensOverride =
        attempt > 0 && this.getFinishReason(payload) === "length"
          ? (this.config.maxTokens ?? 4096) * 2
          : undefined;

      // On a repair attempt, disable reasoning so the model emits its answer
      // directly into message.content instead of only chain-of-thought.
      const reasoningForAttempt =
        attempt === 0 ? options.reasoning : { enabled: false };

      const response = await this.postWithRetries(
        messages, schemaName, schema, model, 2, maxTokensOverride,
        options.signal, remaining, reasoningForAttempt,
      );
      const responsePayload = await response.json() as Record<string, unknown>;
      payload = responsePayload;

      const choices = (responsePayload as any).choices ?? [];
      if (choices.length === 0) {
        // Include response body for debugging — common causes:
        // - Model returned empty response
        // - API error wrapped in 200 (e.g. rate limit with body error)
        const preview = JSON.stringify(responsePayload).slice(0, 200);
        throw new Error(`OpenRouter response did not include choices: ${preview}`);
      }
      content = choices[0]?.message?.content ?? "";
      if (!content) {
        if (options.allowMessageFallback) {
          break;
        }
        // Reasoning models may emit only chain-of-thought and exhaust the token
        // budget before writing the answer, leaving message.content empty. Retry
        // once with reasoning disabled to prioritize a content response. Never
        // treat message.reasoning as the answer.
        if (attempt === 0) {
          debug.debug("openrouter", "Empty message.content (reasoning model?) — retrying with reasoning disabled");
          continue;
        }
        const fr = this.getFinishReason(payload);
        const usage = ((payload ?? {}) as any).usage ?? {};
        const detail = `model=${model ?? this.config.model} finish_reason=${fr ?? "?"} completion_tokens=${usage.completion_tokens ?? "?"} reasoning_tokens=${usage.reasoning_tokens ?? "?"}`;
        throw new Error(`OpenRouter returned empty message content after retry (${detail})`);
      }
      content = String(content);

      const trimmed = content.trim();
      if (!trimmed.startsWith("{") && !trimmed.startsWith("[")) {
        if (options.allowMessageFallback) {
          break;
        }
        // Model may have prefixed JSON with reasoning text. Try to extract
        // the first JSON object from the response before giving up.
        const jsonStart = trimmed.indexOf("{");
        const jsonEnd = trimmed.lastIndexOf("}");
        if (jsonStart >= 0 && jsonEnd > jsonStart) {
          const extracted = trimmed.slice(jsonStart, jsonEnd + 1);
          try {
            const data = JSON.parse(extracted) as Record<string, unknown>;
            return this.buildResponse(responsePayload, data, model);
          } catch {
            // Extracted JSON is still malformed — fall through to error
          }
        }
        throw new Error(`LLM returned non-JSON content: ${trimmed.slice(0, 120)}`);
      }

      try {
        const data = JSON.parse(content) as Record<string, unknown>;
        return this.buildResponse(responsePayload, data, model);
      } catch (err) {
        parseError = err as Error;
      }
    }

    const trimmed = content.trim();
    if (options.allowMessageFallback && (!trimmed || !trimmed.startsWith("{") && !trimmed.startsWith("["))) {
      const wrapped = { type: "message", content: trimmed };
      return this.buildResponse(payload ?? { usage: {}, model: model ?? this.config.model }, wrapped, model);
    }

    const finishReason = this.getFinishReason(payload);
    const reason = finishReason ? ` (finish_reason=${String(finishReason)})` : "";
    throw new Error(`LLM returned malformed JSON${reason}: ${parseError?.message ?? "unknown parse error"}`);
  }

  /** Extract finish_reason from an OpenRouter response payload, or null. */
  private getFinishReason(payload: Record<string, unknown> | null): string | null {
    const choices = ((payload ?? {}) as any).choices ?? [];
    return choices[0]?.finish_reason ?? null;
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
    maxTokensOverride?: number,
    signal?: AbortSignal,
    timeoutMs?: number,
    reasoning?: Record<string, unknown>,
  ): Promise<Response> {
    let lastResponse: Response | null = null;
    let lastError: Error | null = null;
    const totalBudget = timeoutMs ?? DEFAULT_LLM_TIMEOUT_MS;
    const deadline = Date.now() + totalBudget;

    for (let attempt = 0; attempt <= maxRetries; attempt++) {
      const remaining = deadline - Date.now();
      if (remaining <= 0) {
        // Budget exhausted before this attempt could start.
        if (!lastResponse && !lastError) {
          lastError = new Error(`LLM request timed out after ${totalBudget}ms`);
        }
        break;
      }
      try {
        const response = await this.post(messages, schemaName, schema, model, maxTokensOverride, signal, remaining, reasoning);
        lastResponse = response;

        if (response.ok) return response;

        // OpenRouter returns HTTP 400 when the upstream provider fails.
        // The provider error is transient — retry (other providers may work).
        const isProviderError = response.status === 400 &&
          (await response.clone().text().catch(() => "")).includes('provider_name');

        if (isProviderError) {
          debug.debug("openrouter", `Provider error (attempt ${attempt + 1}/${maxRetries + 1}), retrying...`);
          const backoff = 1000 * (attempt + 1);
          if (Date.now() + backoff >= deadline) break;
          await abortableSleep(backoff, signal);
          continue;
        }

        if (!RETRYABLE_STATUSES.has(response.status) || attempt >= maxRetries) {
          break;
        }

        // Wait before retry — but not past the deadline, and interruptible by abort.
        const retryBackoff = 250 * (attempt + 1);
        if (Date.now() + retryBackoff >= deadline) break;
        await abortableSleep(retryBackoff, signal);
      } catch (err) {
        // Network-level failure (DNS, TLS, connection reset) — retry.
        // Timeouts and caller aborts are not retried.
        lastError = err instanceof Error ? err : new Error(String(err));
        if (/timed out|abort/i.test(lastError.message)) {
          // An abort/timeout must surface as-is, not as a stale prior response.
          lastResponse = null;
          break;
        }
        if (attempt >= maxRetries) break;
        const retryBackoff = 250 * (attempt + 1);
        if (Date.now() + retryBackoff >= deadline) break;
        await abortableSleep(retryBackoff, signal);
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
    maxTokensOverride?: number,
    signal?: AbortSignal,
    timeoutMs?: number,
    reasoning?: Record<string, unknown>,
  ): Promise<Response> {
    const url = `${this.config.baseUrl!.replace(/\/$/, "")}/chat/completions`;

    const body: Record<string, unknown> = {
      model: model ?? this.config.model,
      messages,
      temperature: this.config.temperature,
      max_tokens: maxTokensOverride ?? this.config.maxTokens,
      response_format: {
        type: "json_schema",
        json_schema: {
          name: schemaName,
          schema,
        },
      },
    };
    // Per-call reasoning control (cap or disable chain-of-thought). Only included
    // when the caller opts in; unrelated callers are untouched.
    if (reasoning) body.reasoning = reasoning;

    // Combine the caller's AbortSignal with a per-request timeout into a single
    // controller. A timeout is reported distinctly from a caller abort so that
    // postWithRetries can decide not to retry on either.
    const controller = new AbortController();
    let timedOut = false;
    const onCallerAbort = () => controller.abort();
    if (signal) {
      if (signal.aborted) controller.abort();
      else signal.addEventListener("abort", onCallerAbort, { once: true });
    }
    const timer = timeoutMs
      ? setTimeout(() => {
          timedOut = true;
          controller.abort();
        }, timeoutMs)
      : undefined;

    try {
      return await fetch(url, {
        method: "POST",
        headers: {
          Authorization: `Bearer ${this.config.apiKey}`,
          "Content-Type": "application/json",
        },
        body: JSON.stringify(body),
        signal: controller.signal,
      });
    } catch (err) {
      if (timedOut) {
        throw new Error(`LLM request timed out after ${timeoutMs}ms`);
      }
      // Caller-initiated abort: surface the abort error as-is.
      throw err instanceof Error ? err : new Error(String(err));
    } finally {
      if (timer) clearTimeout(timer);
      if (signal) signal.removeEventListener("abort", onCallerAbort);
    }
  }

}
