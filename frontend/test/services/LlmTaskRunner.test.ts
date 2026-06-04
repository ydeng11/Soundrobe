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
