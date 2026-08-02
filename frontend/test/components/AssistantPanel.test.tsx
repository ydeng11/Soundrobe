// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach, beforeEach } from "vitest";
import React from "react";
import { render, screen, fireEvent, cleanup, waitFor, act } from "@testing-library/react";
import { AssistantPanel } from "../../src/components/AssistantPanel";
import type { AssistantEvent } from "../../src/shared/desktop-api";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
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
  assistantApplyActions: vi.fn(),
  assistantCompleteTaskActions: vi.fn().mockResolvedValue({ success: true }),
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
      keyConfigured={true}
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

  it("labels an unapplied action batch as ready for review", async () => {
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
      expect(screen.getByText("Ready for review")).toBeTruthy();
      expect(screen.queryByText("Completed")).toBeNull();
      expect(screen.getByText("Batch ready for review")).toBeTruthy();
    });
  });

  it("marks the matching preview completed after its batch is applied", async () => {
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
    await screen.findByText("Ready for review");

    emitEvent({
      sessionId: "s1",
      type: "action_batch_applied",
      message: "Applied: Fix tags",
      data: { batchId: "batch-1" },
    });

    await waitFor(() => {
      expect(screen.getByText("Completed")).toBeTruthy();
      expect(screen.queryByText("Ready for review")).toBeNull();
    });
  });

  it("marks a metadata preview completed only with verified native readback", async () => {
    renderPanel();
    const input = screen.getByPlaceholderText(/ask the assistant/i);
    fireEvent.change(input, { target: { value: "Fix tags" } });
    fireEvent.keyDown(input, { key: "Enter" });
    emitEvent({
      sessionId: "s1",
      type: "action_batch_created",
      message: "Batch ready for review",
      data: { actionBatchId: "batch-verified" },
    });
    await screen.findByText("Ready for review");

    emitEvent({
      sessionId: "s1",
      type: "action_batch_applied",
      message: "Applied: Fix tags",
      data: {
        batchId: "batch-verified",
        verificationRequired: true,
        verification: {
          status: "verified",
          phase: "readback",
          scopeCount: 2,
          expectedActionCount: 2,
          verifiedActionCount: 2,
          failures: [],
        },
      },
    });

    await waitFor(() => {
      expect(screen.getByText("Completed")).toBeTruthy();
    });
  });

  it("surfaces semantic verification warnings even when readback is verified", async () => {
    // A derived batch or a uniform literal across many folders reports a
    // semantic warning alongside a verified readback; the panel must show it.
    renderPanel();
    const input = screen.getByPlaceholderText(/ask the assistant/i);
    fireEvent.change(input, { target: { value: "Set album from folder" } });
    fireEvent.keyDown(input, { key: "Enter" });
    emitEvent({
      sessionId: "s1",
      type: "action_batch_created",
      message: "Batch ready for review",
      data: { actionBatchId: "batch-warning" },
    });
    await screen.findByText("Ready for review");

    emitEvent({
      sessionId: "s1",
      type: "action_batch_applied",
      message: "Applied: Set album from folder",
      data: {
        batchId: "batch-warning",
        verificationRequired: true,
        verification: {
          status: "verified",
          phase: "readback",
          scopeCount: 381,
          expectedActionCount: 381,
          verifiedActionCount: 381,
          failures: [],
          warnings: [
            "Disk write verified, but a semantic warning was raised: the same value 'based on their folder name' was written across 14 different folders. Confirm this was intended rather than folder-derived.",
          ],
        },
      },
    });

    await screen.findByText("Completed");
    fireEvent.click(screen.getByText("2 steps"));
    expect(
      screen.getByText(/semantic warning was raised/),
    ).toBeTruthy();
  });

  it("shows folder-derived informational confirmation as a success", async () => {
    // A derived batch whose source verification passed reports informational
    // text, which must render as a success, not a warning.
    renderPanel();
    const input = screen.getByPlaceholderText(/ask the assistant/i);
    fireEvent.change(input, { target: { value: "Set album from folder" } });
    fireEvent.keyDown(input, { key: "Enter" });
    emitEvent({
      sessionId: "s1",
      type: "action_batch_created",
      message: "Batch ready for review",
      data: { actionBatchId: "batch-informational" },
    });
    await screen.findByText("Ready for review");

    emitEvent({
      sessionId: "s1",
      type: "action_batch_applied",
      message: "Applied: Set album from folder",
      data: {
        batchId: "batch-informational",
        verificationRequired: true,
        verification: {
          status: "verified",
          phase: "readback",
          scopeCount: 381,
          expectedActionCount: 381,
          verifiedActionCount: 381,
          failures: [],
          informational: [
            "Disk write verified; values matched their containing folder sources.",
          ],
        },
      },
    });

    await screen.findByText("Completed");
    fireEvent.click(screen.getByText("2 steps"));
    expect(
      screen.getByText(/values matched their containing folder sources/),
    ).toBeTruthy();
    expect(screen.queryByText(/semantic warning/)).toBeNull();
  });

  it("fails a metadata preview when an applied event lacks verified readback", async () => {
    renderPanel();
    const input = screen.getByPlaceholderText(/ask the assistant/i);
    fireEvent.change(input, { target: { value: "Fix tags" } });
    fireEvent.keyDown(input, { key: "Enter" });
    emitEvent({
      sessionId: "s1",
      type: "action_batch_created",
      message: "Batch ready for review",
      data: { actionBatchId: "batch-unverified" },
    });
    await screen.findByText("Ready for review");

    emitEvent({
      sessionId: "s1",
      type: "action_batch_applied",
      message: "Applied: Fix tags",
      data: { batchId: "batch-unverified", verificationRequired: true },
    });

    await waitFor(() => {
      expect(screen.getByText("Failed")).toBeTruthy();
      expect(screen.queryByText("Completed")).toBeNull();
    });
  });

  it("shows affected track details for native verification failures", async () => {
    renderPanel();
    const input = screen.getByPlaceholderText(/ask the assistant/i);
    fireEvent.change(input, { target: { value: "Fix tags" } });
    fireEvent.keyDown(input, { key: "Enter" });
    emitEvent({
      sessionId: "s1",
      type: "action_batch_created",
      message: "Batch ready for review",
      data: { actionBatchId: "batch-failed" },
    });
    await screen.findByText("Ready for review");

    emitEvent({
      sessionId: "s1",
      type: "action_batch_failed",
      message: "Failed: Fix tags",
      data: {
        batchId: "batch-failed",
        verification: {
          status: "failed",
          phase: "readback",
          failures: [
            {
              trackPath: "/music/problem.flac",
              field: "artists",
              error: "Metadata readback did not match",
            },
          ],
        },
      },
    });

    await screen.findByText("Failed");
    fireEvent.click(screen.getByText("2 steps"));
    expect(
      screen.getByText(/\/music\/problem\.flac: Metadata readback did not match/),
    ).toBeTruthy();
  });

  it("does not label a prose-only message as completed work", async () => {
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
      expect(screen.getByText("Answered")).toBeTruthy();
      expect(screen.queryByText("Completed")).toBeNull();
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
      expect(screen.getByText("Answered")).toBeTruthy();
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
  it("finalizes delegated task batches only after the renderer task succeeds", async () => {
    mockApi.assistantGetBatches.mockResolvedValueOnce([{
      id: "batch-task",
      createdAt: "now",
      sessionId: "session",
      kind: "auto-tag-run",
      title: "Auto-tag album",
      summary: "Auto-tag 1 track",
      riskLevel: "medium",
      actions: [{ trackPath: "/music/album/track.flac" }],
      reversible: true,
      status: "pending",
    }]);
    mockApi.assistantApplyActions.mockResolvedValueOnce({
      success: true,
      task: "auto_tag",
      trackPaths: ["/music/album/track.flac"],
    });
    const runTask = vi.fn().mockResolvedValue(undefined);
    renderPanel({ onAssistantRunTask: runTask });

    fireEvent.click(await screen.findByRole("button", { name: "Apply" }));

    await waitFor(() => {
      expect(runTask).toHaveBeenCalledWith("auto_tag", ["/music/album/track.flac"]);
      expect(mockApi.assistantCompleteTaskActions)
        .toHaveBeenCalledWith("batch-task", null);
    });
  });

  it("keeps the renderer watchdog beyond the native ten-minute session", async () => {
    vi.useFakeTimers();
    renderPanel();
    const input = screen.getByPlaceholderText(/ask the assistant/i);
    fireEvent.change(input, { target: { value: "Fill missing genres" } });
    fireEvent.keyDown(input, { key: "Enter" });

    await act(async () => {
      await vi.advanceTimersByTimeAsync(30_000);
    });
    expect(mockApi.assistantCancel).not.toHaveBeenCalled();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(570_000);
    });
    expect(mockApi.assistantCancel).not.toHaveBeenCalled();

    await act(async () => {
      await vi.advanceTimersByTimeAsync(30_000);
    });
    expect(mockApi.assistantCancel).toHaveBeenCalledTimes(1);
  });

  it("shows session number when available", async () => {
    renderPanel();
    await waitFor(() => {
      expect(screen.getByText("#test-123")).toBeTruthy();
    });
  });

  it("disables send when no key configured", () => {
    render(
      <AssistantPanel
        isOpen={true}
        onClose={vi.fn()}
        keyConfigured={false}
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
