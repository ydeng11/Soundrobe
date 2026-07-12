import { describe, it, expect, vi, beforeAll, afterAll, afterEach } from "vitest";
import {
  OpenRouterClient,
  estimateCost,
  formatCost,
} from "../../electron/handlers/openrouter";

describe("OpenRouterClient", () => {
  const originalFetch = globalThis.fetch;

  afterAll(() => {
    globalThis.fetch = originalFetch;
  });

  it("completes JSON with valid response", async () => {
    globalThis.fetch = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        choices: [
          {
            message: {
              content: JSON.stringify({
                selectedIndex: 0,
                confidence: 0.95,
                reason: "Best match",
              }),
            },
          },
        ],
        usage: {
          prompt_tokens: 100,
          completion_tokens: 50,
          total_tokens: 150,
        },
        model: "deepseek/deepseek-chat",
      }),
    });

    const client = new OpenRouterClient({
      apiKey: "test-key",
    });

    const result = await client.completeJson(
      [{ role: "user", content: "test" }],
      "test-schema",
      { type: "object", properties: {} },
    );

    expect(result.data.selectedIndex).toBe(0);
    expect(result.data.confidence).toBe(0.95);
    expect(result.usage.promptTokens).toBe(100);
    expect(result.usage.completionTokens).toBe(50);
    expect(result.usage.totalTokens).toBe(150);
  });

  it("throws on natural language text by default", async () => {
    globalThis.fetch = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        choices: [{ message: { content: "not json" } }],
        usage: { prompt_tokens: 10, completion_tokens: 5, total_tokens: 15 },
        model: "test-model",
      }),
    });

    const client = new OpenRouterClient({ apiKey: "test" });
    await expect(
      client.completeJson([{ role: "user", content: "x" }], "s", {}),
    ).rejects.toThrow("LLM returned non-JSON content");
  });

  it("wraps natural language text as a message when explicitly allowed", async () => {
    globalThis.fetch = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        choices: [{ message: { content: "not json" } }],
        usage: { prompt_tokens: 10, completion_tokens: 5, total_tokens: 15 },
        model: "test-model",
      }),
    });

    const client = new OpenRouterClient({ apiKey: "test" });
    const result = await client.completeJson(
      [{ role: "user", content: "x" }],
      "s",
      {},
      undefined,
      { allowMessageFallback: true },
    );
    expect(result.data).toEqual({ type: "message", content: "not json" });
    expect(result.usage.promptTokens).toBe(10);
  });

  it("retries once when the first response has malformed JSON", async () => {
    let attempts = 0;
    globalThis.fetch = vi.fn().mockImplementation(async () => {
      attempts++;
      return {
        ok: true,
        json: async () => ({
          choices: [
            {
              message: {
                content: attempts === 1 ? '{"tracks":[{"message":"cut' : '{"tracks":[]}',
              },
            },
          ],
          usage: {},
          model: "test-model",
        }),
      };
    });

    const client = new OpenRouterClient({ apiKey: "test" });
    const result = await client.completeJson(
      [{ role: "user", content: "x" }],
      "s",
      {},
    );

    expect(result.data.tracks).toEqual([]);
    expect(attempts).toBe(2);
  });

  it("throws on HTTP error", async () => {
    globalThis.fetch = vi.fn().mockResolvedValue({
      ok: false,
      status: 401,
      text: async () => "Unauthorized",
    });

    const client = new OpenRouterClient({ apiKey: "bad-key" });
    await expect(
      client.completeJson([{ role: "user", content: "x" }], "s", {}),
    ).rejects.toThrow("HTTP 401");
  });

  it("retries on 429 then succeeds", async () => {
    let attempts = 0;
    globalThis.fetch = vi.fn().mockImplementation(async () => {
      attempts++;
      if (attempts <= 1) {
        return { ok: false, status: 429, text: async () => "Rate limited" };
      }
      return {
        ok: true,
        json: async () => ({
          choices: [{ message: { content: "{}" } }],
          usage: {},
          model: "test-model",
        }),
      };
    });

    const client = new OpenRouterClient({ apiKey: "test" });
    const result = await client.completeJson(
      [{ role: "user", content: "x" }],
      "s",
      {},
    );
    expect(result.model).toBe("test-model");
    expect(attempts).toBe(2);
  });

  it("throws after exhausting retries", async () => {
    globalThis.fetch = vi.fn().mockResolvedValue({
      ok: false,
      status: 503,
      text: async () => "Service unavailable",
    });

    const client = new OpenRouterClient({ apiKey: "test" });
    await expect(
      client.completeJson([{ role: "user", content: "x" }], "s", {}),
    ).rejects.toThrow("HTTP 503");
  });

  it("throws on empty choices", async () => {
    globalThis.fetch = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({ choices: [], usage: {} }),
    });

    const client = new OpenRouterClient({ apiKey: "test" });
    await expect(
      client.completeJson([{ role: "user", content: "x" }], "s", {}),
    ).rejects.toThrow("did not include choices");
  });

  it("retries once then throws a diagnostic error on repeated empty message content", async () => {
    let attempts = 0;
    globalThis.fetch = vi.fn().mockImplementation(async () => {
      attempts++;
      return {
        ok: true,
        json: async () => ({
          choices: [{ message: { content: "" } }],
          usage: { prompt_tokens: 5, completion_tokens: 3, reasoning_tokens: 3, total_tokens: 8 },
          model: "reasoning-model",
        }),
      };
    });

    const client = new OpenRouterClient({ apiKey: "test", model: "reasoning-model" });
    // Empty structured content is retried once with reasoning disabled; if it is
    // still empty, the error must include model, finish_reason, and token counts.
    await expect(
      client.completeJson([{ role: "user", content: "x" }], "s", {}),
    ).rejects.toThrow(/empty message content after retry.*model=reasoning-model.*reasoning_tokens=3/);
    // Exactly one repair attempt (first empty → retry, second empty → fail).
    expect(attempts).toBe(2);
  });

  it("wraps empty message content when explicitly allowed", async () => {
    globalThis.fetch = vi.fn().mockResolvedValue({
      ok: true,
      json: async () => ({
        choices: [{ message: { content: "" } }],
        usage: { prompt_tokens: 5, completion_tokens: 3, total_tokens: 8 },
        model: "test-model",
      }),
    });

    const client = new OpenRouterClient({ apiKey: "test" });
    const result = await client.completeJson(
      [{ role: "user", content: "x" }],
      "s",
      {},
      undefined,
      { allowMessageFallback: true },
    );
    expect(result.data).toEqual({ type: "message", content: "" });
    expect(result.model).toBe("test-model");
  });

  it("includes a caller-provided reasoning control in the request body", async () => {
    let captured: any;
    globalThis.fetch = vi.fn().mockImplementation(async (_url: string, init?: { body?: string }) => {
      captured = JSON.parse(String(init?.body));
      return {
        ok: true,
        json: async () => ({
          choices: [{ message: { content: JSON.stringify({ ok: true }) } }],
          usage: {},
          model: "test-model",
        }),
      };
    });

    const client = new OpenRouterClient({ apiKey: "test" });
    await client.completeJson(
      [{ role: "user", content: "x" }],
      "s",
      { type: "object", properties: {} },
      undefined,
      { reasoning: { max_tokens: 256 } },
    );
    expect(captured.reasoning).toEqual({ max_tokens: 256 });
  });

  it("omits the reasoning control when the caller provides none", async () => {
    let captured: any;
    globalThis.fetch = vi.fn().mockImplementation(async (_url: string, init?: { body?: string }) => {
      captured = JSON.parse(String(init?.body));
      return {
        ok: true,
        json: async () => ({
          choices: [{ message: { content: JSON.stringify({ ok: true }) } }],
          usage: {},
          model: "test-model",
        }),
      };
    });

    const client = new OpenRouterClient({ apiKey: "test" });
    await client.completeJson(
      [{ role: "user", content: "x" }],
      "s",
      { type: "object", properties: {} },
    );
    expect(captured.reasoning).toBeUndefined();
  });

  it("retries once with reasoning disabled when content is empty, then succeeds", async () => {
    const bodies: any[] = [];
    let attempts = 0;
    globalThis.fetch = vi.fn().mockImplementation(async (_url: string, init?: { body?: string }) => {
      attempts++;
      bodies.push(JSON.parse(String(init?.body)));
      if (attempts === 1) {
        return { ok: true, json: async () => ({ choices: [{ message: { content: "" } }], usage: {}, model: "test-model" }) };
      }
      return {
        ok: true,
        json: async () => ({
          choices: [{ message: { content: JSON.stringify({ ok: true }) } }],
          usage: {},
          model: "test-model",
        }),
      };
    });

    const client = new OpenRouterClient({ apiKey: "test" });
    const result = await client.completeJson(
      [{ role: "user", content: "x" }],
      "s",
      { type: "object", properties: {} },
      undefined,
      { reasoning: { max_tokens: 256 } },
    );
    expect(result.data.ok).toBe(true);
    // First attempt uses the caller's cap; the repair attempt disables reasoning
    // so the model is forced to emit its answer into message.content.
    expect(attempts).toBe(2);
    expect(bodies[0].reasoning).toEqual({ max_tokens: 256 });
    expect(bodies[1].reasoning).toEqual({ enabled: false });
  });
});

describe("OpenRouterClient — timeout & abort", () => {
  const originalFetch = globalThis.fetch;

  afterEach(() => {
    globalThis.fetch = originalFetch;
  });

  // Simulates a real fetch that rejects when its AbortSignal fires.
  function hangingFetch(): ReturnType<typeof vi.fn> {
    return vi.fn().mockImplementation((_url: string, init?: { signal?: AbortSignal }) =>
      new Promise((_resolve, reject) => {
        const signal = init?.signal;
        const onAbort = () => reject(new DOMException("Aborted", "AbortError"));
        if (signal?.aborted) {
          onAbort();
          return;
        }
        signal?.addEventListener("abort", onAbort);
      }),
    );
  }

  it("aborts a hanging request within timeoutMs", async () => {
    globalThis.fetch = hangingFetch();
    const client = new OpenRouterClient({ apiKey: "test" });
    const start = Date.now();
    await expect(
      client.completeJson([{ role: "user", content: "x" }], "s", {}, undefined, {
        timeoutMs: 50,
      }),
    ).rejects.toThrow(/timed out/i);
    expect(Date.now() - start).toBeLessThan(1500);
  });

  it("propagates caller AbortSignal as an abort error (not a timeout)", async () => {
    globalThis.fetch = hangingFetch();
    const client = new OpenRouterClient({ apiKey: "test" });
    const controller = new AbortController();
    const promise = client.completeJson([{ role: "user", content: "x" }], "s", {}, undefined, {
      signal: controller.signal,
      timeoutMs: 5000,
    });
    controller.abort();
    await expect(promise).rejects.toThrow(/abort/i);
  });

  it("stops retrying once the deadline is exhausted", async () => {
    let attempts = 0;
    globalThis.fetch = vi.fn().mockImplementation(async () => {
      attempts++;
      return { ok: false, status: 429, text: async () => "Rate limited" };
    });
    const client = new OpenRouterClient({ apiKey: "test" });
    await expect(
      client.completeJson([{ role: "user", content: "x" }], "s", {}, undefined, {
        timeoutMs: 60,
      }),
    ).rejects.toThrow();
    // Without a deadline, 3 attempts (1 + 2 retries) would fire. With a 60ms
    // deadline and 250ms+ backoff between retries, only the first attempt can
    // run before the budget is spent.
    expect(attempts).toBeLessThanOrEqual(2);
  });

  it("respects a single total deadline across JSON-repair attempts", async () => {
    let attempts = 0;
    globalThis.fetch = vi.fn().mockImplementation(async (_url: string, init?: { signal?: AbortSignal }) => {
      attempts++;
      if (attempts === 1) {
        // Truncated JSON → parse error → completeJson retries.
        return {
          ok: true,
          json: async () => ({
            choices: [{ message: { content: '{"tracks":[{"message":"cut' } }],
            usage: {},
            model: "test-model",
          }),
        };
      }
      // Second attempt hangs until the deadline aborts it.
      return new Promise((_resolve, reject) => {
        const signal = init?.signal;
        const onAbort = () => reject(new DOMException("Aborted", "AbortError"));
        if (signal?.aborted) {
          onAbort();
          return;
        }
        signal?.addEventListener("abort", onAbort);
      });
    });
    const client = new OpenRouterClient({ apiKey: "test" });
    const start = Date.now();
    await expect(
      client.completeJson([{ role: "user", content: "x" }], "s", {}, undefined, {
        timeoutMs: 80,
      }),
    ).rejects.toThrow(/timed out|abort/i);
    expect(Date.now() - start).toBeLessThan(1500);
    expect(attempts).toBe(2);
  });

  it("does not wait past the deadline before giving up on retries", async () => {
    let attempts = 0;
    globalThis.fetch = vi.fn().mockImplementation(async () => {
      attempts++;
      return { ok: false, status: 429, text: async () => "Rate limited" };
    });
    const client = new OpenRouterClient({ apiKey: "test" });
    const start = Date.now();
    await expect(
      client.completeJson([{ role: "user", content: "x" }], "s", {}, undefined, {
        timeoutMs: 60,
      }),
    ).rejects.toThrow();
    // The first 429 is immediate; the 250ms retry backoff would exceed the 60ms
    // deadline, so we must break immediately rather than sleeping 250ms.
    expect(Date.now() - start).toBeLessThan(200);
    expect(attempts).toBe(1);
  });

  it("interrupts retry backoff when the caller aborts", async () => {
    let attempts = 0;
    globalThis.fetch = vi.fn().mockImplementation(async (_url: string, init?: { signal?: AbortSignal }) => {
      attempts++;
      if (init?.signal?.aborted) {
        throw new DOMException("Aborted", "AbortError");
      }
      return { ok: false, status: 429, text: async () => "Rate limited" };
    });
    const client = new OpenRouterClient({ apiKey: "test" });
    const controller = new AbortController();
    const start = Date.now();
    const promise = client.completeJson([{ role: "user", content: "x" }], "s", {}, undefined, {
      signal: controller.signal,
      timeoutMs: 5000,
    });
    // Abort during the 250ms retry backoff.
    setTimeout(() => controller.abort(), 20);
    await expect(promise).rejects.toThrow(/abort/i);
    // Backoff is 250ms; abort at ~20ms means we finish well before 250ms.
    expect(Date.now() - start).toBeLessThan(200);
  });

  it("surfaces a caller abort even when message content is empty", async () => {
    globalThis.fetch = hangingFetch();
    const client = new OpenRouterClient({ apiKey: "test" });
    const controller = new AbortController();
    const promise = client.completeJson([{ role: "user", content: "x" }], "s", {}, undefined, {
      signal: controller.signal,
      timeoutMs: 5000,
    });
    controller.abort();
    await expect(promise).rejects.toThrow(/abort/i);
  });
});

describe("estimateCost / formatCost", () => {
  it("estimates cost for known model", () => {
    const cost = estimateCost("deepseek/deepseek-chat", {
      promptTokens: 1000,
      completionTokens: 500,
      totalTokens: 1500,
    });
    expect(cost).toBeCloseTo(0.00014 + 0.00014, 5);
  });

  it("returns 0 for free model", () => {
    const cost = estimateCost("deepseek/deepseek-v4-flash:free", {
      promptTokens: 1000,
      completionTokens: 500,
      totalTokens: 1500,
    });
    expect(cost).toBe(0);
  });

  it("uses default rates for unknown model", () => {
    const cost = estimateCost("unknown-model", {
      promptTokens: 1000,
      completionTokens: 500,
      totalTokens: 1500,
    });
    expect(cost).toBeGreaterThan(0);
  });

  it("formats small costs", () => {
    expect(formatCost(0.0005)).toBe("$0.50/1K");
  });

  it("formats medium costs", () => {
    expect(formatCost(0.003)).toBe("$0.0030");
  });
});
