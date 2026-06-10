import { describe, it, expect, vi, beforeAll, afterAll } from "vitest";
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

  it("wraps natural language text as a message when JSON parsing fails", async () => {
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
      "s", {},
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

  it("wraps empty message content as a message rather than throwing", async () => {
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
      "s", {},
    );
    expect(result.data).toEqual({ type: "message", content: "" });
    expect(result.model).toBe("test-model");
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
