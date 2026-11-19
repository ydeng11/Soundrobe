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

  it("removes a resolved interrupted-write row without changing session undo", async () => {
    const onChanged = vi.fn();
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
    window.api = {
      getAutoTagReview: vi.fn().mockResolvedValue({
        id: "run", albumPath: "/music/album", outcome: "needs_review", decision: "pending",
        result: null, before: { tracks: [], artworks: [], errors: [] },
        after: { tracks: [], artworks: [], errors: [] }, canRevert: false,
        errors: ["Interrupted write"], recovered: true, recoveryRequired: true,
        recoveryFiles: [{ path: "/music/album/1.flac", currentSha256: "a".repeat(64) }],
      }),
      keepAutoTagRecoveryFile: vi.fn().mockResolvedValue(null),
    } as any;
    render(<AutoTagSummaryDialog onClose={vi.fn()} onChanged={onChanged} summary={{ items: [{
      albumPath: "/music/album", reviewId: "run", status: "needs_review", attempts: 1,
      message: "Interrupted write needs recovery", reasonCode: null, providerAttempts: [], readbackRequired: false,
    }] }} />);
    fireEvent.click(screen.getByRole("button", { name: "Review album" }));
    fireEvent.click(await screen.findByRole("button", { name: "Keep current file" }));
    expect(await screen.findByRole("status")).toBeTruthy();
    fireEvent.click(screen.getByRole("tab", { name: "Results" }));
    expect(screen.queryByText("/music/album")).toBeNull();
    expect(onChanged).not.toHaveBeenCalled();
    confirm.mockRestore();
  });
});
