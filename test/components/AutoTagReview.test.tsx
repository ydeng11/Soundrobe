// @vitest-environment jsdom
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { AutoTagReview } from "../../src/components/AutoTagReview";

afterEach(cleanup);

it.each([
  { original: null, inferred: "Pop", written: "Pop" },
  { original: "  ", inferred: "Pop", written: "Pop" },
  { original: null, inferred: "搖滾", written: "摇滚" },
])("labels inferred genre per file after display conversion or keeping: $original → $written", async ({ original, inferred, written }) => {
  const applied = {
    ...review,
    outcome: "needs_review",
    canRevert: true,
    result: { written: 2, candidate: { source: "musicbrainz", genre: written, assisted_genre: {
      value: inferred, model: "scripted/genre-v1", input_sha256: "a".repeat(64),
    } } },
    before: { ...review.before, tracks: [
      { path: "/music/album/1.flac", title: "One", genre: original },
      { path: "/music/album/2.flac", title: "Two", genre: "User genre" },
    ] },
    after: { ...review.after, tracks: [
      { path: "/music/album/1.flac", title: "One", genre: written },
      { path: "/music/album/2.flac", title: "Two", genre: "User genre" },
    ] },
  };
  window.api = {
    getAutoTagReview: vi.fn().mockResolvedValue(applied),
    markAutoTagReviewed: vi.fn().mockResolvedValue({ ...applied, decision: "kept" }),
  } as any;
  render(<AutoTagReview reviewId="run" onRetry={vi.fn()} onSearch={vi.fn()} onChanged={vi.fn()} busy={false} />);
  expect(await screen.findByText("AI-inferred · unverified")).toBeTruthy();
  expect(screen.queryByText(/No changes written/)).toBeNull();
  expect(screen.getByText(/Keeping changes acknowledges review, not verification/)).toBeTruthy();
  fireEvent.click(screen.getByRole("button", { name: "All metadata" }));
  expect(screen.getAllByText("AI-inferred · unverified")).toHaveLength(1);
  fireEvent.click(screen.getByRole("button", { name: "Keep changes" }));
  await waitFor(() => expect(window.api.markAutoTagReviewed).toHaveBeenCalledWith("run"));
  expect(screen.getByText("AI-inferred · unverified")).toBeTruthy();
});

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

it("uses stored field reviews to highlight only the written physical file", async () => {
  const paths = ["/music/album/1.flac", "/music/album/2.flac"];
  const affected = {
    ...review,
    outcome: "applied",
    result: { candidate: { genre: "Dream pop", assisted_genre: { value: "Dream pop" } } },
    before: { ...review.before, tracks: paths.map((path) => ({ path, genre: null })) },
    after: { ...review.after, tracks: paths.map((path) => ({ path, genre: "Dream pop" })) },
    fieldReviews: [{
      path: paths[0],
      field: "genre",
      before: null,
      after: "Dream pop",
      source: "Discogs release notes",
      state: "open",
    }],
  };
  window.api = {
    getAutoTagReview: vi.fn().mockResolvedValue(affected),
    markAutoTagReviewed: vi.fn().mockResolvedValue({
      ...affected,
      decision: "kept",
      fieldReviews: [{ ...affected.fieldReviews[0], state: "acknowledged" }],
    }),
  } as any;
  render(<AutoTagReview reviewId="run" onRetry={vi.fn()} onSearch={vi.fn()} onChanged={vi.fn()} busy={false} />);

  const firstTrack = (await screen.findByText(/1\.flac · 1 changed field/)).closest("details")!;
  const secondTrack = screen.getByText(/2\.flac · 1 changed field/).closest("details")!;
  expect(within(firstTrack).getByText("AI-assisted · review pending")).toBeTruthy();
  expect(within(firstTrack).getByText("Discogs release notes")).toBeTruthy();
  expect(within(secondTrack).queryByText(/AI-assisted|AI-inferred/)).toBeNull();
  fireEvent.click(screen.getByRole("button", { name: "Keep changes" }));
  expect(await within(firstTrack).findByText("AI-assisted · reviewed")).toBeTruthy();
});

it("confirms keeping the reviewed current file when resolving an interrupted write", async () => {
  const keepRecoveryFile = vi.fn().mockResolvedValue(null);
  const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
  window.api = {
    getAutoTagReview: vi.fn().mockResolvedValue({
      ...review,
      recovered: true,
      recoveryRequired: true,
      recoveryFiles: [{ path: "/music/album/1.flac", currentSha256: "a".repeat(64) }],
      fieldReviews: [],
      result: null,
      before: { ...review.before, tracks: [] },
      after: { ...review.after, tracks: [] },
      errors: ["Could not verify the interrupted write to /music/album/1.flac"],
    }),
    keepAutoTagRecoveryFile: keepRecoveryFile,
  } as any;
  render(<AutoTagReview reviewId="run" onRetry={vi.fn()} onSearch={vi.fn()} onChanged={vi.fn()} busy={false} />);
  expect((await screen.findByRole("alert")).textContent).toContain("interrupted write to");
  fireEvent.click(screen.getByRole("button", { name: /keep current file/i }));
  await waitFor(() => expect(keepRecoveryFile).toHaveBeenCalledWith("run", "/music/album/1.flac", "a".repeat(64)));
  expect(confirm).toHaveBeenCalled();
  expect(screen.queryByText(/No changes written/)).toBeNull();
  confirm.mockRestore();
});
it("reloads a partially interrupted reversal and offers resume", async () => {
  const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);
  const initial = { ...review, outcome: "applied", canRevert: true };
  const interrupted = { ...initial, recovered: true, recoveryRequired: true };
  const get = vi.fn().mockResolvedValueOnce(initial).mockResolvedValue(interrupted);
  window.api = { getAutoTagReview: get, revertAutoTagReview: vi.fn().mockRejectedValue(new Error("interrupted")) } as any;
  render(<AutoTagReview reviewId="run" onRetry={vi.fn()} onSearch={vi.fn()} onChanged={vi.fn()} busy={false} />);
  fireEvent.click(await screen.findByRole("button", { name: "Revert album" }));
  expect(await screen.findByRole("button", { name: "Resume album reversal" })).toBeTruthy();
  expect((await screen.findByRole("alert")).textContent).toContain("interrupted");
  confirm.mockRestore();
});
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
