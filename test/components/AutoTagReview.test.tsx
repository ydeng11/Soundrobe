// @vitest-environment jsdom
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { AutoTagReview } from "../../src/components/AutoTagReview";

afterEach(cleanup);
const review = {
  id: "run",
  albumPath: "/music/album",
  outcome: "needs_review",
  decision: "pending",
  result: {
    reasonCode: "provider_unavailable",
    providerAttempts: [
      {
        provider: "musicbrainz",
        status: "unavailable",
        diagnostic: "HTTP 503",
      },
    ],
  },
  before: {
    tracks: [
      {
        path: "/music/album/1.flac",
        title: "Original",
        lyrics: { plainLyrics: "words" },
        extraTags: [{ key: "LABEL", value: "Independent" }],
      },
    ],
    artworks: [],
    errors: [],
  },
  after: {
    tracks: [
      {
        path: "/music/album/1.flac",
        title: "Original",
        lyrics: { plainLyrics: "words" },
        extraTags: [{ key: "LABEL", value: "Independent" }],
      },
    ],
    artworks: [],
    errors: [],
  },
  canRevert: false,
  errors: [],
};
it("explains skipped albums and keeps current metadata without offering a fictitious revert", async () => {
  window.api = {
    getAutoTagReview: vi.fn().mockResolvedValue(review),
    markAutoTagReviewed: vi
      .fn()
      .mockResolvedValue({ ...review, decision: "kept" }),
  } as any;
  render(
    <AutoTagReview
      reviewId="run"
      onRetry={vi.fn()}
      onSearch={vi.fn()}
      onChanged={vi.fn()}
      busy={false}
    />,
  );
  expect(await screen.findByText(/No changes written/)).toBeTruthy();
  expect(screen.queryByRole("button", { name: "Revert album" })).toBeNull();
  expect(screen.getByText(/HTTP 503/)).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "All metadata" }));
  expect(screen.getAllByText(/Independent/).length).toBeGreaterThan(0);
  fireEvent.click(
    screen.getByRole("button", { name: "Keep current metadata" }),
  );
  await waitFor(() =>
    expect(window.api.markAutoTagReviewed).toHaveBeenCalledWith("run"),
  );
});
it("compares applied values and preserves undo after keeping", async () => {
  const applied = {
    ...review,
    outcome: "applied",
    canRevert: true,
    after: {
      ...review.after,
      tracks: [{ ...review.after.tracks[0], title: "Corrected" }],
    },
  };
  window.api = {
    getAutoTagReview: vi.fn().mockResolvedValue(applied),
    markAutoTagReviewed: vi
      .fn()
      .mockResolvedValue({ ...applied, decision: "kept" }),
  } as any;
  render(
    <AutoTagReview
      reviewId="run"
      onRetry={vi.fn()}
      onSearch={vi.fn()}
      onChanged={vi.fn()}
      busy={false}
    />,
  );
  expect(await screen.findByText("Corrected")).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "Keep changes" }));
  await waitFor(() =>
    expect(screen.getByRole("button", { name: "Revert album" })).toBeTruthy(),
  );
});

it("loads artwork only when the gallery opens and supports an enlarged preview", async () => {
  const artwork = {
    id: "cover",
    label: "cover.jpg",
    source: "external",
    width: 32,
    height: 32,
    error: null,
  };
  window.api = {
    getAutoTagReview: vi
      .fn()
      .mockResolvedValue({
        ...review,
        after: { ...review.after, artworks: [artwork] },
      }),
    getAutoTagReviewArtwork: vi
      .fn()
      .mockResolvedValue("data:image/png;base64,AA=="),
  } as any;
  render(
    <AutoTagReview
      reviewId="run"
      onRetry={vi.fn()}
      onSearch={vi.fn()}
      onChanged={vi.fn()}
      busy={false}
    />,
  );
  await screen.findByText(/No changes written/);
  expect(window.api.getAutoTagReviewArtwork).not.toHaveBeenCalled();
  fireEvent.click(screen.getByText(/Artwork — before and after/));
  await screen.findByRole("img", { name: "cover.jpg" });
  expect(window.api.getAutoTagReviewArtwork).toHaveBeenCalledWith(
    "run",
    "cover",
  );
  fireEvent.click(screen.getByRole("button", { name: "Enlarge cover.jpg" }));
  expect(screen.getByRole("dialog", { name: "Artwork preview" })).toBeTruthy();
  const closeArtwork = screen.getByRole("button", { name: "Close artwork" });
  expect(fireEvent.keyDown(closeArtwork, { key: "Tab" })).toBe(false);
  expect(document.activeElement).toBe(closeArtwork);
  expect(fireEvent.keyDown(closeArtwork, { key: "Tab", shiftKey: true })).toBe(false);
  fireEvent.keyDown(closeArtwork, { key: "Escape" });
  expect(document.activeElement).toBe(screen.getByRole("button", { name: "Enlarge cover.jpg" }));
  expect(screen.queryByRole("dialog", { name: "Artwork preview" })).toBeNull();
});

it("keeps a failed restore retryable and reports its error", async () => {
  const applied = { ...review, outcome: "applied", canRevert: true };
  window.api = {
    getAutoTagReview: vi.fn().mockResolvedValue(applied),
    revertAutoTagReview: vi
      .fn()
      .mockRejectedValue(new Error("Later changes prevent restoration")),
  } as any;
  const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
  const changed = vi.fn();
  render(
    <AutoTagReview
      reviewId="run"
      onRetry={vi.fn()}
      onSearch={vi.fn()}
      onChanged={changed}
      busy={false}
    />,
  );
  fireEvent.click(await screen.findByRole("button", { name: "Revert album" }));
  expect(await screen.findByRole("alert")).toHaveProperty(
    "textContent",
    "Error: Later changes prevent restoration",
  );
  expect(changed).not.toHaveBeenCalled();
  expect(
    (screen.getByRole("button", { name: "Revert album" }) as HTMLButtonElement)
      .disabled,
  ).toBe(false);
  confirm.mockRestore();
});
