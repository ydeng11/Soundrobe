// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import React from "react";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { WebTaggingDebugPortal } from "../../src/components/WebTaggingDebugPortal";

afterEach(() => {
  cleanup();
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
    expect(fetchMock).toHaveBeenCalledWith("/api/v1/tasks", { credentials: "same-origin" });
  });
});
