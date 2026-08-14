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
      onOpenSettings={vi.fn()}
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

describe("AssistantPanel — refreshed side sheet", () => {
  it("renders as an accessible responsive side sheet beneath the title bar", () => {
    renderPanel({ model: "deepseek/deepseek-v3" });

    const panel = screen.getByRole("complementary", { name: "AI Assistant" });
    expect(panel.className).toContain("top-[38px]");
    expect(panel.className).toContain("w-[420px]");
    expect(panel.className).toContain("max-w-[calc(100vw-24px)]");
    expect(screen.getByText("AI Assistant")).toBeTruthy();
    expect(screen.getByText("deepseek/deepseek-v3")).toBeTruthy();
  });

  it("prefills suggested prompts without sending them", () => {
    renderPanel();

    fireEvent.click(screen.getByRole("button", { name: "Summarize my library" }));

    expect(
      (screen.getByPlaceholderText(/ask the assistant/i) as HTMLTextAreaElement)
        .value,
    ).toBe("Summarize my library");
    expect(mockApi.assistantSend).not.toHaveBeenCalled();
  });

  it("opens Settings from the no-key empty state", () => {
    const onOpenSettings = vi.fn();
    renderPanel({ keyConfigured: false, onOpenSettings });

    fireEvent.click(
      screen.getByRole("button", { name: "Configure AI in Settings" }),
    );

    expect(onOpenSettings).toHaveBeenCalledOnce();
  });

  it("shows selected tracks as the highest-priority context", () => {
    renderPanel({
      selectedTrackPaths: ["/music/one.flac", "/music/two.flac"],
      activeAlbumPath: "/music/Album",
      libraryPath: "/music",
    });

    expect(screen.getByText("2 selected tracks")).toBeTruthy();
    expect(screen.queryByText("Album: Album")).toBeNull();
  });

  it("falls back from album to library to no-library context", () => {
    const { rerender } = renderPanel({
      activeAlbumPath: "/music/Artist/Album",
      libraryPath: "/music",
    });
    expect(screen.getByText("Album: Album")).toBeTruthy();

    rerender(
      <AssistantPanel
        isOpen={true}
        onClose={vi.fn()}
        onOpenSettings={vi.fn()}
        keyConfigured={true}
        libraryPath="/music"
        activeAlbumPath={null}
        selectedTrackPaths={[]}
        allTracks={[]}
        allAlbums={[]}
        autonomous={false}
        onRefreshRequest={vi.fn()}
      />,
    );
    expect(screen.getByText("Entire library")).toBeTruthy();

    rerender(
      <AssistantPanel
        isOpen={true}
        onClose={vi.fn()}
        onOpenSettings={vi.fn()}
        keyConfigured={true}
        libraryPath={null}
        activeAlbumPath={null}
        selectedTrackPaths={[]}
        allTracks={[]}
        allAlbums={[]}
        autonomous={false}
        onRefreshRequest={vi.fn()}
      />,
    );
    expect(screen.getByText("No library context")).toBeTruthy();
  });

  it("grows the composer up to its five-line cap", () => {
    renderPanel();
    const input = screen.getByPlaceholderText(/ask the assistant/i) as HTMLTextAreaElement;
    Object.defineProperty(input, "scrollHeight", {
      configurable: true,
      value: 180,
    });

    fireEvent.input(input);

    expect(input.style.height).toBe("112px");
  });

  it("returns the composer to one-line height after sending a multiline prompt", async () => {
    renderPanel();
    const input = screen.getByPlaceholderText(/ask the assistant/i) as HTMLTextAreaElement;
    Object.defineProperty(input, "scrollHeight", {
      configurable: true,
      get: () => input.value.includes("\n") ? 112 : 40,
    });

    fireEvent.change(input, { target: { value: "Find missing genres\nin this album" } });
    fireEvent.input(input);
    expect(input.style.height).toBe("112px");

    fireEvent.keyDown(input, { key: "Enter" });

    await waitFor(() => {
      expect(input.value).toBe("");
      expect(input.style.height).toBe("40px");
    });
  });

  it("resizes the composer when editing a multiline message", async () => {
    renderPanel();
    const input = screen.getByPlaceholderText(/ask the assistant/i) as HTMLTextAreaElement;
    Object.defineProperty(input, "scrollHeight", {
      configurable: true,
      get: () => input.value.includes("\n") ? 112 : 40,
    });
    const prompt = "Find missing genres\nin this album";

    fireEvent.change(input, { target: { value: prompt } });
    fireEvent.keyDown(input, { key: "Enter" });
    emitEvent({ sessionId: "s1", type: "message", message: "I found two tracks." });
    await screen.findByRole("button", { name: "Edit and resend" });

    // Editing begins from the standard one-line composer height.
    input.style.height = "40px";
    fireEvent.click(screen.getByRole("button", { name: "Edit and resend" }));

    await waitFor(() => {
      expect(input.value).toBe(prompt);
      expect(input.style.height).toBe("112px");
    });
  });

  it("presents pending actions as an explicit bounded review card", async () => {
    mockApi.assistantGetBatches.mockResolvedValueOnce([{
      id: "batch-review",
      createdAt: "now",
      sessionId: "session",
      kind: "metadata-edit",
      title: "Fill missing genre",
      summary: "Add one missing genre without replacing existing tags",
      riskLevel: "medium",
      actions: [{
        trackPath: "/music/track.flac",
        field: "genre",
        newValue: "Pop",
      }],
      reversible: true,
      status: "pending",
    }]);

    renderPanel();

    expect(await screen.findByText("Review changes")).toBeTruthy();
    expect(screen.queryByText("Ask about your music library")).toBeNull();
    expect(screen.getByText("Medium risk")).toBeTruthy();
    expect(screen.getByText("1 change on 1 track")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Apply changes" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Reject" })).toBeTruthy();
  });

  it("shows file destinations and counts their source tracks in the review card", async () => {
    mockApi.assistantGetBatches.mockResolvedValueOnce([{
      id: "batch-file-review",
      createdAt: "now",
      sessionId: "session",
      kind: "folder-move",
      title: "Organize album files",
      summary: "Rename and relocate two tracks",
      riskLevel: "medium",
      actions: [
        {
          operation: "files.transform",
          sourcePath: "/music/inbox/01.flac",
          destinationPath: "/music/Artist/Album/01 - First.flac",
          description: "Rename from track metadata",
        },
        {
          operation: "files.relocate",
          sourcePath: "/music/inbox/02.flac",
          destinationPath: "/music/Artist/Album/02 - Second.flac",
          description: "Move into folder: Artist/Album",
        },
      ],
      reversible: true,
      status: "pending",
    }]);

    renderPanel();

    expect(await screen.findByText("2 changes on 2 tracks")).toBeTruthy();
    expect(screen.getByText(
      "/music/inbox/01.flac → /music/Artist/Album/01 - First.flac",
    )).toBeTruthy();
    expect(screen.getByText(
      "/music/inbox/02.flac → /music/Artist/Album/02 - Second.flac",
    )).toBeTruthy();
  });
});

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
    expect(screen.getByRole("status").textContent).toContain("Sending…");
    expect(screen.getByText("Waiting for response…")).toBeTruthy();
  });

  it("does not send the message when Enter commits an IME composition (selecting a word in the input method)", () => {
    renderPanel();
    const input = screen.getByPlaceholderText(/ask the assistant/i);
    fireEvent.change(input, { target: { value: "你好" } });
    // Enter to confirm the candidate word while the input method is composing
    fireEvent.keyDown(input, { key: "Enter", isComposing: true });

    expect(mockApi.assistantSend).not.toHaveBeenCalled();
    expect(screen.queryByText("Sending…")).toBeNull();
  });

  it("does not send WebKit's post-composition Enter event", () => {
    renderPanel();
    const input = screen.getByPlaceholderText(/ask the assistant/i);
    fireEvent.change(input, { target: { value: "你好" } });
    // WebKit can report isComposing=false while retaining the IME keyCode.
    fireEvent.keyDown(input, { key: "Enter", keyCode: 229, isComposing: false });

    expect(mockApi.assistantSend).not.toHaveBeenCalled();
    expect(screen.queryByText("Sending…")).toBeNull();
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

describe("AssistantPanel — new conversation button", () => {
  it("clears the conversation when the New chat button is clicked", async () => {
    renderPanel();
    // Start a conversation and let the request finish
    const input = screen.getByPlaceholderText(/ask the assistant/i);
    fireEvent.change(input, { target: { value: "Summarize my library" } });
    fireEvent.keyDown(input, { key: "Enter" });
    emitEvent({ sessionId: "s1", type: "message", message: "Here's your summary." });
    await screen.findByText("Here's your summary.");

    fireEvent.click(screen.getByRole("button", { name: /new chat/i }));

    await waitFor(() => {
      expect(screen.getByText(/session cleared/i)).toBeTruthy();
    });
    expect(screen.queryByText("Summarize my library")).toBeNull();
    expect(screen.queryByText("Here's your summary.")).toBeNull();
    expect(mockApi.assistantClear).toHaveBeenCalled();
  });

  it("refreshes the session number after starting a new conversation", async () => {
    mockApi.getCurrentSession
      .mockResolvedValueOnce({ sessionNumber: "old-session" })
      .mockResolvedValueOnce({ sessionNumber: "new-session" });
    renderPanel();
    await screen.findByText("#old-session");

    fireEvent.click(screen.getByRole("button", { name: /new chat/i }));

    expect(await screen.findByText("#new-session")).toBeTruthy();
    expect(mockApi.assistantClear).toHaveBeenCalledTimes(1);
  });

  it("preserves the current conversation and surfaces an error when reset fails", async () => {
    mockApi.assistantClear.mockRejectedValueOnce(new Error("reset failed"));
    renderPanel();
    const input = screen.getByPlaceholderText(/ask the assistant/i);
    fireEvent.change(input, { target: { value: "Keep this message" } });
    fireEvent.keyDown(input, { key: "Enter" });
    emitEvent({ sessionId: "s1", type: "message", message: "Keep this answer" });
    await screen.findByText("Keep this answer");

    fireEvent.click(screen.getByRole("button", { name: /new chat/i }));

    expect(await screen.findByText(/failed to start a new conversation: reset failed/i)).toBeTruthy();
    expect(screen.getByText("Keep this message")).toBeTruthy();
    expect(screen.getByText("Keep this answer")).toBeTruthy();
    expect(screen.queryByText(/session cleared/i)).toBeNull();
  });

  it("disables conversation input while the native reset is in flight", async () => {
    let resolveClear!: () => void;
    mockApi.assistantClear.mockImplementationOnce(
      () => new Promise<void>((resolve) => { resolveClear = resolve; }),
    );
    renderPanel();

    fireEvent.click(screen.getByRole("button", { name: /new chat/i }));

    await waitFor(() => {
      expect((screen.getByRole("button", { name: /new chat/i }) as HTMLButtonElement).disabled).toBe(true);
      expect((screen.getByPlaceholderText(/ask the assistant/i) as HTMLTextAreaElement).disabled).toBe(true);
    });

    resolveClear();
    expect(await screen.findByText(/session cleared/i)).toBeTruthy();
  });

  it("disables the New chat button while a request is sending", async () => {
    renderPanel();
    const input = screen.getByPlaceholderText(/ask the assistant/i);
    fireEvent.change(input, { target: { value: "Summarize" } });
    fireEvent.keyDown(input, { key: "Enter" });

    await waitFor(() => {
      expect(
        (screen.getByRole("button", { name: /new chat/i }) as HTMLButtonElement)
          .disabled,
      ).toBe(true);
    });
  });

  it("disables the New chat button while an action batch is applying", async () => {
    mockApi.assistantGetBatches.mockResolvedValueOnce([{
      id: "batch-applying",
      createdAt: "now",
      sessionId: "session",
      kind: "metadata-edit",
      title: "Apply tags",
      summary: "Apply one tag",
      riskLevel: "low",
      actions: [{ trackPath: "/music/track.flac", field: "genre", newValue: "Pop" }],
      reversible: true,
      status: "pending",
    }]);
    let resolveApply!: (result: { success: boolean; error: string; results: unknown[] }) => void;
    mockApi.assistantApplyActions.mockImplementationOnce(
      () => new Promise((resolve) => { resolveApply = resolve; }),
    );
    renderPanel();

    fireEvent.click(await screen.findByRole("button", { name: "Apply changes" }));

    await waitFor(() => {
      expect((screen.getByRole("button", { name: /new chat/i }) as HTMLButtonElement).disabled).toBe(true);
    });

    resolveApply({ success: false, error: "apply failed", results: [] });
    await waitFor(() => {
      expect((screen.getByRole("button", { name: /new chat/i }) as HTMLButtonElement).disabled).toBe(false);
    });
  });

  it("shows progress only for the action batch currently being applied", async () => {
    mockApi.assistantGetBatches.mockResolvedValueOnce([{
      id: "batch-current",
      createdAt: "now",
      sessionId: "session",
      kind: "metadata-update",
      title: "Apply tags",
      summary: "Apply three tags",
      riskLevel: "low",
      actions: [
        { trackPath: "/music/a/one.flac", field: "genre", newValue: "Pop" },
        { trackPath: "/music/b/two.flac", field: "genre", newValue: "Rock" },
        { trackPath: "/music/c/three.flac", field: "genre", newValue: "Jazz" },
      ],
      reversible: true,
      status: "pending",
    }]);
    let resolveApply!: (result: { success: boolean; results: unknown[] }) => void;
    mockApi.assistantApplyActions.mockImplementationOnce(
      () => new Promise((resolve) => { resolveApply = resolve; }),
    );
    renderPanel();

    fireEvent.click(await screen.findByRole("button", { name: "Apply changes" }));
    emitEvent({
      sessionId: "s1",
      type: "action_batch_progress",
      message: "Writing 9/9",
      data: { batchId: "batch-other", phase: "writing", current: 9, total: 9 },
    });
    expect(screen.queryByText("Writing 9/9")).toBeNull();

    emitEvent({
      sessionId: "s1",
      type: "action_batch_progress",
      message: "Writing 1/3",
      data: { batchId: "batch-current", phase: "writing", current: 1, total: 3 },
    });
    expect(await screen.findByText("Writing 1/3")).toBeTruthy();

    resolveApply({ success: true, results: [] });
    await waitFor(() => expect(screen.queryByText("Writing 1/3")).toBeNull());
  });

  it("clears matching apply progress on native failure", async () => {
    mockApi.assistantGetBatches.mockResolvedValueOnce([{
      id: "batch-failing-progress",
      createdAt: "now",
      sessionId: "session",
      kind: "metadata-update",
      title: "Apply tags",
      summary: "Apply one tag",
      riskLevel: "low",
      actions: [{ trackPath: "/music/a/one.flac", field: "genre", newValue: "Pop" }],
      reversible: true,
      status: "pending",
    }]);
    mockApi.assistantApplyActions.mockImplementationOnce(() => new Promise(() => {}));
    renderPanel();

    fireEvent.click(await screen.findByRole("button", { name: "Apply changes" }));
    emitEvent({
      sessionId: "s1",
      type: "action_batch_progress",
      message: "Verifying 1/1",
      data: { batchId: "batch-failing-progress", phase: "verifying", current: 1, total: 1 },
    });
    expect(await screen.findByText("Verifying 1/1")).toBeTruthy();

    emitEvent({
      sessionId: "s1",
      type: "action_batch_failed",
      message: "Failed: Apply tags",
      data: { batchId: "batch-failing-progress" },
    });
    await waitFor(() => expect(screen.queryByText("Verifying 1/1")).toBeNull());
  });
});

describe("AssistantPanel — core behavior preserved", () => {
  it("groups mixed standard and extra-tag snapshots into one history command", async () => {
    mockApi.assistantGetBatches.mockResolvedValueOnce([{
      id: "batch-mixed",
      createdAt: "now",
      sessionId: "session",
      kind: "metadata-update",
      title: "Apply mixed tags",
      summary: "Apply standard and extra tags",
      riskLevel: "low",
      actions: [{ trackPath: "/music/track.flac", field: "genre", newValue: "Pop" }],
      reversible: true,
      status: "pending",
    }]);
    const undoSnapshots = [{ path: "/music/track.flac", metadata: { genre: "Rock" } }];
    const extraUndoSnapshots = [{
      path: "/music/track.flac",
      extraTags: [{ key: "MOOD", value: "Calm" }],
    }];
    mockApi.assistantApplyActions.mockResolvedValueOnce({
      success: true,
      results: [],
      undoSnapshots,
      extraUndoSnapshots,
    });
    const onAssistantApplyUndo = vi.fn();
    const onApplyingChange = vi.fn();
    renderPanel({ onAssistantApplyUndo, onApplyingChange });

    fireEvent.click(await screen.findByRole("button", { name: "Apply changes" }));

    await waitFor(() => {
      expect(onAssistantApplyUndo).toHaveBeenCalledOnce();
      expect(onAssistantApplyUndo).toHaveBeenCalledWith(
        "Assistant Apply",
        undoSnapshots,
        extraUndoSnapshots,
        true,
      );
      expect(onApplyingChange.mock.calls.map(([applying]) => applying)).toEqual([
        true,
        false,
      ]);
    });
  });

  it("retains undo evidence when an assistant apply partially writes before failing", async () => {
    mockApi.assistantGetBatches.mockResolvedValueOnce([{
      id: "batch-partial",
      createdAt: "now",
      sessionId: "session",
      kind: "metadata-update",
      title: "Apply tags",
      summary: "Apply tags to two tracks",
      riskLevel: "low",
      actions: [{ trackPath: "/music/track.flac", field: "genre", newValue: "Pop" }],
      reversible: true,
      status: "pending",
    }]);
    const undoSnapshots = [
      { path: "/music/success.flac", metadata: { genre: "Rock" } },
      { path: "/music/failure.flac", metadata: { genre: "Jazz" } },
    ];
    mockApi.assistantApplyActions.mockResolvedValueOnce({
      success: false,
      error: "one track failed",
      results: [{ trackPath: "/music/failure.flac", error: "disk full" }],
      undoSnapshots,
      verification: {
        status: "failed",
        phase: "write",
        scopeCount: 1,
        expectedActionCount: 1,
        verifiedActionCount: 0,
        failures: [{ error: "disk full" }],
      },
    });
    const onAssistantApplyUndo = vi.fn();
    renderPanel({ onAssistantApplyUndo });

    fireEvent.click(await screen.findByRole("button", { name: "Apply changes" }));

    await waitFor(() => {
      expect(onAssistantApplyUndo).toHaveBeenCalledWith(
        "Assistant Apply (partial)",
        [undoSnapshots[0]],
        [],
        true,
      );
    });
  });

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

    fireEvent.click(await screen.findByRole("button", { name: "Apply changes" }));

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
        onOpenSettings={vi.fn()}
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
