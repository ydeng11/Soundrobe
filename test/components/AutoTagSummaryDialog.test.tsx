// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { AutoTagSummaryDialog } from "../../src/components/AutoTagSummaryDialog";

afterEach(cleanup);

describe("AutoTagSummaryDialog", () => {
  it("shows every album outcome and closes explicitly", () => {
    const onClose = vi.fn();
    render(
      <AutoTagSummaryDialog
        summary={{
          items: [
            {
              albumPath: "/music/Recovered",
              status: "recovered",
              attempts: 2,
              message: "Applied after retry",
              reasonCode: null,
              providerAttempts: [],
              readbackRequired: true,
            },
            {
              albumPath: "/music/Needs Review",
              status: "needs_review",
              reviewId: "review-album",
              attempts: 2,
              message: "Provider unavailable",
              reasonCode: "provider_unavailable",
              providerAttempts: [
                {
                  provider: "musicbrainz",
                  status: "unavailable",
                  diagnostic: "HTTP 503",
                  retryCount: 2,
                  retryAfterSeconds: 30,
                },
              ],
              readbackRequired: false,
            },
          ],
        }}
        onClose={onClose}
      />,
    );

    expect(screen.getByRole("dialog", { name: "Auto-tag summary" })).toBeTruthy();
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Close" }));
    fireEvent.keyDown(document.activeElement!, { key: "Tab", shiftKey: true });
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Done" }));
    fireEvent.keyDown(document.activeElement!, { key: "Tab" });
    expect(document.activeElement).toBe(screen.getByRole("button", { name: "Close" }));
    expect(screen.getByText("/music/Recovered")).toBeTruthy();
    expect(screen.getByText("/music/Needs Review")).toBeTruthy();
    expect(screen.getByText("Recovered on retry")).toBeTruthy();
    expect(screen.getByText(/HTTP 503/)).toBeTruthy();
    expect(screen.getByText(/2 provider retries/)).toBeTruthy();
    window.api = { getAutoTagReview: vi.fn().mockResolvedValue({ id: "review-album", albumPath: "/music/Needs Review", outcome: "needs_review", decision: "pending", result: {}, before: { tracks: [], artworks: [], errors: [] }, after: { tracks: [], artworks: [], errors: [] }, canRevert: false, errors: [] }) } as any;
    fireEvent.click(screen.getByRole("button", { name: "Review album" }));
    expect(screen.getByRole("tab", { name: "Review" }).getAttribute("aria-selected")).toBe("true");
    fireEvent.click(screen.getByRole("tab", { name: "Results" }));
    expect(screen.getByText("/music/Recovered")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Done" }));
    expect(onClose).toHaveBeenCalledOnce();
  });
});
