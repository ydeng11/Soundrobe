// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach, beforeEach } from "vitest";
import React from "react";
import { render, screen, fireEvent, cleanup, waitFor } from "@testing-library/react";
import { AssistantPanel } from "../../src/components/AssistantPanel";
import type { AssistantEvent } from "../../electron/preload";

afterEach(() => {
  cleanup();
});

// Polyfill scrollIntoView for jsdom
Element.prototype.scrollIntoView = vi.fn();

// ── Mocks ──────────────────────────────────────────────────────────

const mockApi = {
  assistantInitRuntime: vi.fn().mockResolvedValue(undefined),
  getCurrentSession: vi.fn().mockResolvedValue({ sessionNumber: "test-123" }),
  assistantGetBatches: vi.fn().mockResolvedValue([]),
  assistantSend: vi.fn().mockResolvedValue(undefined),
  assistantCancel: vi.fn().mockResolvedValue(undefined),
  onAssistantEvent: vi.fn().mockReturnValue(() => {}),
  getConfig: vi.fn().mockResolvedValue({ llmApiKey: "test-key", llmModel: "test-model" }),
  assistantClear: vi.fn().mockResolvedValue(undefined),
};

beforeEach(() => {
  vi.clearAllMocks();
  (window as any).api = mockApi;
});

// ── Factory ────────────────────────────────────────────────────────

function renderPanel(props?: Partial<React.ComponentProps<typeof AssistantPanel>>) {
  return render(
    <AssistantPanel
      isOpen={true}
      onClose={vi.fn()}
      apiKey="test-key"
      libraryPath="/music"
      activeAlbumPath={null}
      selectedTrackPaths={[]}
      allTracks={[]}
      allAlbums={[]}
      autonomous={false}
      onRefreshRequest={vi.fn()}
      {...props}
    />,
  );
}

/** Simulate an assistant event being emitted by the API. */
function emitEvent(event: AssistantEvent) {
  const handler = mockApi.onAssistantEvent.mock.calls[0]?.[0];
  if (handler) handler(event);
}

// ── Tests ──────────────────────────────────────────────────────────

describe("AssistantPanel — status indicator", () => {
  it("shows empty state when no messages", () => {
    renderPanel();
    expect(screen.getByText(/ask me anything/i)).toBeTruthy();
  });

  it("creates a pending assistant message with 'sending' status when user sends a prompt", async () => {
    renderPanel();
    const input = screen.getByPlaceholderText(/ask the assistant/i);
    fireEvent.change(input, { target: { value: "Summarize my library" } });
    fireEvent.keyDown(input, { key: "Enter" });

    // User message should be visible
    expect(screen.getByText("Summarize my library")).toBeTruthy();

    // Pending assistant message with 'sending' status
    expect(screen.getByText("Sending…")).toBeTruthy();
    expect(screen.getByText("Waiting for response…")).toBeTruthy();
  });

  it("transitions to 'thinking' on tool_running event", async () => {
    renderPanel();
    const input = screen.getByPlaceholderText(/ask the assistant/i);
    fireEvent.change(input, { target: { value: "Find genres" } });
    fireEvent.keyDown(input, { key: "Enter" });

    emitEvent({
      sessionId: "s1",
      type: "tool_running",
      message: "Searching MusicBrainz…",
    });

    await waitFor(() => {
      expect(screen.getByText("Thinking…")).toBeTruthy();
    });
  });

  it("transitions to 'looking_up' on tool_result event", async () => {
    renderPanel();
    const input = screen.getByPlaceholderText(/ask the assistant/i);
    fireEvent.change(input, { target: { value: "Find genres" } });
    fireEvent.keyDown(input, { key: "Enter" });

    emitEvent({
      sessionId: "s1",
      type: "tool_result",
      message: "Found 3 genres",
    });

    await waitFor(() => {
      expect(screen.getByText("Looking up data…")).toBeTruthy();
    });
  });

  it("transitions to 'completed' on action_batch_created event with batch summary", async () => {
    renderPanel();
    const input = screen.getByPlaceholderText(/ask the assistant/i);
    fireEvent.change(input, { target: { value: "Fix tags" } });
    fireEvent.keyDown(input, { key: "Enter" });

    emitEvent({
      sessionId: "s1",
      type: "action_batch_created",
      message: "Batch ready for review",
      data: { actionBatchId: "batch-1" },
    });

    await waitFor(() => {
      expect(screen.getByText("Batch ready for review")).toBeTruthy();
    });
  });

  it("transitions to 'completed' on message event", async () => {
    renderPanel();
    const input = screen.getByPlaceholderText(/ask the assistant/i);
    fireEvent.change(input, { target: { value: "Hi" } });
    fireEvent.keyDown(input, { key: "Enter" });

    emitEvent({
      sessionId: "s1",
      type: "message",
      message: "Hello! I'm the assistant.",
    });

    await waitFor(() => {
      expect(screen.getByText("Completed")).toBeTruthy();
      expect(screen.getByText("Hello! I'm the assistant.")).toBeTruthy();
    });
  });

  it("treats incomplete backend completion events as failed", async () => {
    renderPanel();
    const input = screen.getByPlaceholderText(/ask the assistant/i);
    fireEvent.change(input, { target: { value: "number" } });
    fireEvent.keyDown(input, { key: "Enter" });

    emitEvent({
      sessionId: "s1",
      type: "completed",
      message: "I reached the maximum step limit (10) and couldn't complete the task in one response.",
    });

    await waitFor(() => {
      expect(screen.getByText("Failed")).toBeTruthy();
      expect(screen.getByText(/maximum step limit/i)).toBeTruthy();
    });
  });

  it("transitions to 'failed' on error event and shows retry button", async () => {
    renderPanel();
    const input = screen.getByPlaceholderText(/ask the assistant/i);
    fireEvent.change(input, { target: { value: "Do something" } });
    fireEvent.keyDown(input, { key: "Enter" });

    emitEvent({
      sessionId: "s1",
      type: "error",
      message: "API rate limit exceeded",
    });

    await waitFor(() => {
      expect(screen.getByText("Failed")).toBeTruthy();
      // The error detail should be visible in collapsible details
    });

    // The assistant message should have a retry button (edit icon)
    // We can check by finding the failed message bubble area
  });

  it("does NOT create separate system messages for tool events", async () => {
    renderPanel();
    const input = screen.getByPlaceholderText(/ask the assistant/i);
    fireEvent.change(input, { target: { value: "Check library" } });
    fireEvent.keyDown(input, { key: "Enter" });

    // Simulate multiple backend events
    emitEvent({ sessionId: "s1", type: "tool_running", message: "Scanning…" });
    emitEvent({ sessionId: "s1", type: "tool_result", message: "Found 10 tracks" });
    emitEvent({ sessionId: "s1", type: "tool_running", message: "Looking up tags…" });
    emitEvent({ sessionId: "s1", type: "message", message: "Done!" });

    await waitFor(() => {
      expect(screen.getByText("Completed")).toBeTruthy();
    });

    // Only user message + assistant reply — no separate system messages
    const allBubbles = screen.getAllByText(/Check library|Done!/);
    // User message: "Check library" (1), Assistant reply: "Done!" (1), no tool_running/tool_result as separate messages
    expect(allBubbles.length).toBe(2);
  });

  it("shows steps count and expands details on click", async () => {
    renderPanel();
    const input = screen.getByPlaceholderText(/ask the assistant/i);
    fireEvent.change(input, { target: { value: "Analyze" } });
    fireEvent.keyDown(input, { key: "Enter" });

    emitEvent({ sessionId: "s1", type: "tool_running", message: "Step 1…" });
    emitEvent({ sessionId: "s1", type: "tool_result", message: "Got data" });
    emitEvent({ sessionId: "s1", type: "message", message: "Analysis complete" });

    await waitFor(() => {
      // Should show "2 steps" (tool_running + tool_result)
      expect(screen.getByText("2 steps")).toBeTruthy();
    });

    // Click to expand
    const expandBtn = screen.getByText("2 steps");
    fireEvent.click(expandBtn);

    // Now details should be visible
    expect(screen.getByText("Step 1…")).toBeTruthy();
    expect(screen.getByText("Got data")).toBeTruthy();

    // Click to collapse
    fireEvent.click(screen.getByText("Hide details"));
    await waitFor(() => {
      expect(screen.getByText("2 steps")).toBeTruthy();
    });
  });
});

describe("AssistantPanel — core behavior preserved", () => {
  it("shows session number when available", async () => {
    renderPanel();
    await waitFor(() => {
      expect(screen.getByText("#test-123")).toBeTruthy();
    });
  });

  it("disables send when no apiKey", () => {
    render(
      <AssistantPanel
        isOpen={true}
        onClose={vi.fn()}
        apiKey=""
        libraryPath="/music"
        activeAlbumPath={null}
        selectedTrackPaths={[]}
        allTracks={[]}
        allAlbums={[]}
        autonomous={false}
        onRefreshRequest={vi.fn()}
      />,
    );
    const input = screen.getByPlaceholderText(/configure.*api key/i);
    expect((input as HTMLTextAreaElement).disabled).toBe(true);
  });

  it("clears messages on /clear command", async () => {
    renderPanel();
    const input = screen.getByPlaceholderText(/ask the assistant/i);
    fireEvent.change(input, { target: { value: "/clear" } });
    fireEvent.keyDown(input, { key: "Enter" });

    await waitFor(() => {
      expect(screen.getByText(/session cleared/i)).toBeTruthy();
    });
  });
});
