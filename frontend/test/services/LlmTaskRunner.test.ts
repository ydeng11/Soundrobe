/**
 * Tests for LlmTaskRunner tool name resolution.
 *
 * The resolveToolName method is private — tested via `any` cast.
 */

import { describe, it, expect, vi } from "vitest";
import { LlmTaskRunner } from "../../electron/services/LlmTaskRunner";
import type { AssistantToolDef } from "../../electron/services/LlmTaskRunner";

const tools: AssistantToolDef[] = [
  {
    name: "library.summarize",
    description: "Summarize library",
    inputSchema: { type: "object", properties: {}, required: [] },
  },
  {
    name: "tracks.search",
    description: "Search tracks",
    inputSchema: { type: "object", properties: {}, required: [] },
  },
  {
    name: "organize_files",
    description: "Organize files",
    inputSchema: { type: "object", properties: {}, required: [] },
  },
];

function makeRunner(): LlmTaskRunner {
  return new LlmTaskRunner({ apiKey: "test-key" });
}

describe("LlmTaskRunner — resolveToolName", () => {
  it("returns toolName when present", () => {
    const runner = makeRunner();
    const result = (runner as any).resolveToolName(
      { toolName: "library.summarize", content: "some description" },
      tools,
    );
    expect(result).toBe("library.summarize");
  });

  it("falls back to content when toolName is missing", () => {
    const runner = makeRunner();
    const result = (runner as any).resolveToolName(
      { content: "organize_files", type: "tool_call" },
      tools,
    );
    expect(result).toBe("organize_files");
  });

  it("matches when content starts with a tool name", () => {
    const runner = makeRunner();
    const result = (runner as any).resolveToolName(
      { content: "library.summarize the library stats", type: "tool_call" },
      tools,
    );
    expect(result).toBe("library.summarize");
  });

  it("returns 'unknown' when content does not match any tool", () => {
    const runner = makeRunner();
    const result = (runner as any).resolveToolName(
      { content: "I need more information to proceed", type: "tool_call" },
      tools,
    );
    expect(result).toBe("unknown");
  });

  it("returns 'unknown' when both toolName and content are missing", () => {
    const runner = makeRunner();
    const result = (runner as any).resolveToolName({}, tools);
    expect(result).toBe("unknown");
  });
});

describe("LlmTaskRunner — malformed assistant tool calls", () => {
  it("reports a tool_call without a resolvable tool as incomplete instead of a final message", async () => {
    const runner = makeRunner();
    (runner as any).client = {
      completeJson: vi.fn().mockResolvedValue({
        data: {
          type: "tool_call",
          content: "Applying automatic track numbering to all tracks in the library.",
        },
        usage: { promptTokens: 10, completionTokens: 5, totalTokens: 15 },
        model: "openai/gpt-oss-120b:free",
      }),
    };

    const result = await runner.runToolLoop({
      systemPrompt: "test",
      messages: [{ role: "user", content: "number" }],
      tools,
      maxSteps: 1,
    });

    expect(result.stoppedEarly).toBe(true);
    expect(result.reason).toBe("malformed_tool_call");
    expect(result.steps.at(-1)).toMatchObject({
      type: "message",
      content: expect.stringContaining("Malformed tool call"),
    });
  });
});
