// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import React from "react";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { WebTaggingDebugPortal } from "../../src/components/WebTaggingDebugPortal";

afterEach(() => {
  cleanup();
  vi.useRealTimers();
  vi.restoreAllMocks();
});

describe("WebTaggingDebugPortal", () => {
  it("keeps automatic tagging details out of the editor until Activity is opened", async () => {
    const fetchMock = vi.spyOn(globalThis, "fetch").mockResolvedValue(
      new Response(JSON.stringify([{
        taskId: "auto-tag:/library/Album",
        status: "needs_review",
        message: "Needs review: provider_ambiguous",
        result: null,
      }]), { status: 200 }),
    );

    render(<WebTaggingDebugPortal />);

    expect(screen.queryByText("Background tagging diagnostics")).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Activity" }));

    await waitFor(() => expect(screen.getByText("Needs review: provider_ambiguous")).toBeTruthy());
    expect(fetchMock).toHaveBeenCalledWith("/api/v1/tasks", {
      method: "GET",
      credentials: "same-origin",
    });
  });

  it("shows a loading state before the first response and an empty state after it", async () => {
    let resolveFetch: (response: Response) => void = () => {};
    const fetchMock = vi.spyOn(globalThis, "fetch").mockImplementation(
      () => new Promise((resolve) => { resolveFetch = resolve; }),
    );

    render(<WebTaggingDebugPortal />);
    fireEvent.click(screen.getByRole("button", { name: "Activity" }));

    expect(screen.getByText("Loading activity…")).toBeTruthy();
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));

    resolveFetch(new Response("[]", { status: 200 }));
    await waitFor(() => expect(screen.getByText("No background activity.")).toBeTruthy());
  });

  it("shows a first-load failure and retries successfully", async () => {
    const fetchMock = vi.spyOn(globalThis, "fetch")
      .mockRejectedValueOnce(new Error("service unavailable"))
      .mockResolvedValueOnce(new Response(JSON.stringify([{
        taskId: "task-after-retry",
        status: "completed",
        progress: 1,
        total: 1,
        message: "Applied",
        result: null,
      }]), { status: 200 }));

    render(<WebTaggingDebugPortal />);
    fireEvent.click(screen.getByRole("button", { name: "Activity" }));

    await waitFor(() => expect(screen.getByText("Unable to load activity.")).toBeTruthy());
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    await waitFor(() => expect(screen.getByText("Applied")).toBeTruthy());
    expect(fetchMock).toHaveBeenCalledTimes(2);
  });

  it("retains tasks and reports a stale refresh failure", async () => {
    let poll: (() => void) | undefined;
    vi.spyOn(window, "setInterval").mockImplementation((callback, timeout) => {
      if (timeout === 5_000) {
        poll = typeof callback === "function" ? callback as () => void : undefined;
      }
      return 1;
    });
    const fetchMock = vi.spyOn(globalThis, "fetch")
      .mockResolvedValueOnce(new Response(JSON.stringify([{
        taskId: "task-stale",
        status: "running",
        progress: 1,
        total: 2,
        message: "Still working",
        result: null,
      }]), { status: 200 }))
      .mockRejectedValueOnce(new Error("service unavailable"));

    render(<WebTaggingDebugPortal />);
    fireEvent.click(screen.getByRole("button", { name: "Activity" }));
    await waitFor(() => expect(screen.getByText("Still working")).toBeTruthy());

    poll?.();
    await waitFor(() => expect(
      screen.getByText("Refresh failed; showing last known activity."),
    ).toBeTruthy());
    expect(screen.getByText("Still working")).toBeTruthy();
  });

  it("displays every supported task status without adding task actions", async () => {
    vi.spyOn(globalThis, "fetch").mockResolvedValue(new Response(JSON.stringify([
      ["running", "Running"],
      ["completed", "Completed"],
      ["needs_review", "Needs review"],
      ["failed", "Failed"],
      ["cancelled", "Cancelled"],
    ].map(([status, message], index) => ({
      taskId: `task-${index}`,
      status,
      progress: 1,
      total: 1,
      message,
      result: null,
    }))), { status: 200 }));

    render(<WebTaggingDebugPortal />);
    fireEvent.click(screen.getByRole("button", { name: "Activity" }));

    await waitFor(() => {
      expect(screen.getByText("running")).toBeTruthy();
      expect(screen.getByText("completed")).toBeTruthy();
      expect(screen.getByText("needs review")).toBeTruthy();
      expect(screen.getByText("failed")).toBeTruthy();
      expect(screen.getByText("cancelled")).toBeTruthy();
    });
    expect(screen.queryByRole("button", { name: /review|cancel/i })).toBeNull();
  });

  it("polls only while Activity is displayed and cleans up on close", async () => {
    let poll: (() => void) | undefined;
    vi.spyOn(window, "setInterval").mockImplementation((callback, timeout) => {
      if (timeout === 5_000) {
        poll = typeof callback === "function" ? callback as () => void : undefined;
      }
      return 1;
    });
    const clearIntervalMock = vi.spyOn(window, "clearInterval");
    const fetchMock = vi.spyOn(globalThis, "fetch").mockResolvedValue(
      new Response("[]", { status: 200 }),
    );

    render(<WebTaggingDebugPortal />);
    fireEvent.click(screen.getByRole("button", { name: "Activity" }));
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(1));

    poll?.();
    await waitFor(() => expect(fetchMock).toHaveBeenCalledTimes(2));

    fireEvent.click(screen.getByRole("button", { name: "Close" }));
    expect(clearIntervalMock).toHaveBeenCalledWith(1);
  });
});
