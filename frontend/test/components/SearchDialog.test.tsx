// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach, beforeEach } from "vitest";
import React from "react";
import { render, screen, fireEvent, cleanup, waitFor } from "@testing-library/react";
import { SearchDialog } from "../../src/components/SearchDialog";
import type { ReleaseSearchPage, ProviderAlbum } from "../../src/shared/desktop-api";

afterEach(() => {
  cleanup();
  delete (window as unknown as Record<string, unknown>).api;
});

function makeSearchPage(overrides?: Partial<ReleaseSearchPage>): ReleaseSearchPage {
  return {
    results: [
      {
        provider: "musicbrainz",
        id: "mb-1",
        title: "OK Computer",
        artist: "Radiohead",
        year: "1997",
        country: "GB",
        formats: ["CD"],
        catalogNumber: undefined,
        barcode: undefined,
      },
      {
        provider: "musicbrainz",
        id: "mb-2",
        title: "Kid A",
        artist: "Radiohead",
        year: "2000",
        country: "GB",
        formats: ["CD"],
        catalogNumber: undefined,
        barcode: undefined,
      },
    ],
    page: 1,
    pageSize: 10,
    total: 2,
    hasNext: false,
    ...overrides,
  };
}

function makePagedSearchPage(page: number, pageSize = 100, total = 205): ReleaseSearchPage {
  const start = (page - 1) * pageSize;
  const end = Math.min(start + pageSize, total);
  return {
    results: Array.from({ length: Math.max(0, end - start) }, (_, index) => {
      const number = start + index + 1;
      return {
        provider: "musicbrainz" as const,
        id: `mb-${number}`,
        title: `Release ${number}`,
        artist: "Artist",
        year: "2000",
        country: "US",
        formats: ["CD"],
      };
    }),
    page,
    pageSize,
    total,
    hasNext: end < total,
  };
}

function makeProviderAlbum(overrides?: Partial<ProviderAlbum>): ProviderAlbum {
  return {
    id: "mb-1",
    title: "OK Computer",
    artist: "Radiohead",
    artists: ["Radiohead"],
    artistId: "mb-artist-1",
    year: "1997",
    genre: "Alternative",
    tracks: [
      { title: "Airbag", matchTitles: [], artists: [], trackNumber: 1, recordingId: "t1", length: 4 * 60 },
      { title: "Paranoid Android", matchTitles: [], artists: [], trackNumber: 2, recordingId: "t2", length: 6 * 60 },
    ],
    ...overrides,
  };
}

beforeEach(() => {
  window.api = {
    searchReleases: vi.fn().mockResolvedValue(makeSearchPage()),
    resolveRelease: vi.fn().mockResolvedValue(makeProviderAlbum()),
  } as unknown as Window["api"];
});

describe("SearchDialog", () => {
  const defaultProps = {
    open: true,
    albumPath: "/music/Album",
    onClose: vi.fn(),
    onSelectRelease: vi.fn(),
  };

  it("renders the search form with provider selector", () => {
    render(<SearchDialog {...defaultProps} />);
    expect(screen.getByText("MusicBrainz")).toBeTruthy();
    expect(screen.getByText("Discogs")).toBeTruthy();
    expect(screen.getByPlaceholderText("Artist name")).toBeTruthy();
    expect(screen.getByPlaceholderText("Album title")).toBeTruthy();
  });

  it("renders optional fields: year, country, format, catalog, barcode", () => {
    render(<SearchDialog {...defaultProps} />);
    expect(screen.getByPlaceholderText("e.g. 2004")).toBeTruthy();
    expect(screen.getByPlaceholderText("e.g. US")).toBeTruthy();
    expect(screen.getByPlaceholderText("e.g. CD")).toBeTruthy();
  });

  it("disables search button when both artist and album are empty", () => {
    render(<SearchDialog {...defaultProps} />);
    const btn = screen.getByText("Search") as HTMLButtonElement;
    expect(btn.disabled).toBe(true);
  });

  it("enables search button when artist is filled", () => {
    render(<SearchDialog {...defaultProps} />);
    fireEvent.change(screen.getByPlaceholderText("Artist name"), {
      target: { value: "Radiohead" },
    });
    expect((screen.getByText("Search") as HTMLButtonElement).disabled).toBe(false);
  });

  it("calls searchReleases on submit and shows results", async () => {
    render(<SearchDialog {...defaultProps} />);
    fireEvent.change(screen.getByPlaceholderText("Artist name"), {
      target: { value: "Radiohead" },
    });
    fireEvent.change(screen.getByPlaceholderText("Album title"), {
      target: { value: "OK Computer" },
    });
    fireEvent.click(screen.getByText("Search"));

    await waitFor(() => {
      expect(window.api.searchReleases).toHaveBeenCalledWith(
        expect.objectContaining({ provider: "musicbrainz", artist: "Radiohead", album: "OK Computer" }),
      );
    });
    await waitFor(() => {
      expect(screen.getByText("OK Computer")).toBeTruthy();
      expect(screen.getByText("Kid A")).toBeTruthy();
    });
  });

  it("fetches every provider page once and paginates the cached results locally", async () => {
    const mockSearch = vi.fn().mockImplementation(({ page = 1, pageSize = 100 }) =>
      Promise.resolve(makePagedSearchPage(page, pageSize)),
    );
    window.api.searchReleases = mockSearch;
    render(<SearchDialog {...defaultProps} />);
    fireEvent.change(screen.getByPlaceholderText("Artist name"), {
      target: { value: "Artist" },
    });
    fireEvent.click(screen.getByText("Search"));

    await waitFor(() => expect(mockSearch).toHaveBeenCalledTimes(3));
    expect(mockSearch.mock.calls.map(([request]) => request.page)).toEqual([1, 2, 3]);
    expect(mockSearch.mock.calls.every(([request]) => request.pageSize === 100)).toBe(true);
    expect(screen.getByText("Release 1")).toBeTruthy();
    expect(screen.queryByText("Release 11")).toBeNull();
    expect(screen.getByText("Page 1 of 21")).toBeTruthy();

    fireEvent.click(screen.getByText("Next >"));
    expect(screen.getByText("Release 11")).toBeTruthy();
    expect(screen.queryByText("Release 1")).toBeNull();
    expect(mockSearch).toHaveBeenCalledTimes(3);
  });

  it("keeps server pagination for Discogs searches", async () => {
    const mockSearch = vi.fn().mockImplementation(({ page = 1 }) => Promise.resolve({
      results: [{
        provider: "discogs" as const,
        id: `dg-${page}`,
        title: `Discogs page ${page}`,
        formats: ["CD"],
      }],
      page,
      pageSize: 10,
      total: 20,
      hasNext: page === 1,
    }));
    window.api.searchReleases = mockSearch;
    render(<SearchDialog {...defaultProps} />);
    fireEvent.click(screen.getByText("Discogs"));
    fireEvent.change(screen.getByPlaceholderText("Artist name"), {
      target: { value: "Artist" },
    });
    fireEvent.click(screen.getByText("Search"));

    await waitFor(() => expect(screen.getByText("Discogs page 1")).toBeTruthy());
    expect(screen.queryByPlaceholderText("Filter release titles")).toBeNull();
    expect(mockSearch).toHaveBeenLastCalledWith(expect.objectContaining({ page: 1, pageSize: 10 }));

    fireEvent.click(screen.getByText("Next >"));
    await waitFor(() => expect(screen.getByText("Discogs page 2")).toBeTruthy());
    expect(mockSearch).toHaveBeenLastCalledWith(expect.objectContaining({ page: 2, pageSize: 10 }));
  });

  it("filters all cached release titles without making another provider request", async () => {
    const mockSearch = vi.fn().mockImplementation(({ page = 1, pageSize = 100 }) =>
      Promise.resolve(makePagedSearchPage(page, pageSize)),
    );
    window.api.searchReleases = mockSearch;
    render(<SearchDialog {...defaultProps} />);
    fireEvent.change(screen.getByPlaceholderText("Artist name"), {
      target: { value: "Artist" },
    });
    fireEvent.click(screen.getByText("Search"));

    await waitFor(() => expect(mockSearch).toHaveBeenCalledTimes(3));
    fireEvent.click(screen.getByText("Next >"));
    fireEvent.change(screen.getByPlaceholderText("Filter release titles"), {
      target: { value: "release 205" },
    });

    expect(screen.getByText("Release 205")).toBeTruthy();
    expect(screen.getByText("Results (1 of 205)")).toBeTruthy();
    expect(screen.queryByText("Next >")).toBeNull();
    expect(mockSearch).toHaveBeenCalledTimes(3);

    fireEvent.change(screen.getByPlaceholderText("Filter release titles"), {
      target: { value: "does not exist" },
    });
    expect(screen.getByText("No cached releases match this title.")).toBeTruthy();
    expect(mockSearch).toHaveBeenCalledTimes(3);
  });

  it("clears visible results when the dialog closes", async () => {
    const { rerender } = render(<SearchDialog {...defaultProps} />);
    fireEvent.change(screen.getByPlaceholderText("Artist name"), {
      target: { value: "Radiohead" },
    });
    fireEvent.click(screen.getByText("Search"));
    await waitFor(() => expect(screen.getByText("OK Computer")).toBeTruthy());

    rerender(<SearchDialog {...defaultProps} open={false} />);
    rerender(<SearchDialog {...defaultProps} open />);

    await waitFor(() => expect(screen.getByText("Search releases")).toBeTruthy());
    expect(screen.queryByText("OK Computer")).toBeNull();
    expect(screen.queryByPlaceholderText("Filter release titles")).toBeNull();
  });

  it("reuses a completed MusicBrainz catalog after Back and close/reopen", async () => {
    const mockSearch = vi.fn().mockImplementation(({ page = 1, pageSize = 100 }) =>
      Promise.resolve(makePagedSearchPage(page, pageSize, 101)),
    );
    window.api.searchReleases = mockSearch;
    const { rerender } = render(<SearchDialog {...defaultProps} />);
    fireEvent.change(screen.getByPlaceholderText("Artist name"), {
      target: { value: "Artist" },
    });
    fireEvent.click(screen.getByText("Search"));
    await waitFor(() => expect(mockSearch).toHaveBeenCalledTimes(2));

    fireEvent.click(document.querySelector("[title='Back to search']")!);
    fireEvent.click(screen.getByText("Search"));
    await waitFor(() => expect(screen.getByText("Release 1")).toBeTruthy());
    expect(mockSearch).toHaveBeenCalledTimes(2);

    rerender(<SearchDialog {...defaultProps} open={false} />);
    rerender(<SearchDialog {...defaultProps} open />);
    fireEvent.click(screen.getByText("Search"));
    await waitFor(() => expect(screen.getByText("Release 1")).toBeTruthy());
    expect(mockSearch).toHaveBeenCalledTimes(2);
  });

  it("does not duplicate an identical search while its request is in flight", async () => {
    let resolveSearch!: (page: ReleaseSearchPage) => void;
    const pendingSearch = new Promise<ReleaseSearchPage>((resolve) => {
      resolveSearch = resolve;
    });
    const mockSearch = vi.fn().mockReturnValue(pendingSearch);
    window.api.searchReleases = mockSearch;
    render(<SearchDialog {...defaultProps} />);
    const artistInput = screen.getByPlaceholderText("Artist name");
    fireEvent.change(artistInput, { target: { value: "Radiohead" } });

    fireEvent.keyDown(artistInput, { key: "Enter" });
    fireEvent.keyDown(artistInput, { key: "Enter" });

    expect(mockSearch).toHaveBeenCalledTimes(1);
    resolveSearch(makeSearchPage());
    await waitFor(() => expect(screen.getByText("OK Computer")).toBeTruthy());
  });

  it("caches successful server-paged results and fetches each uncached page once", async () => {
    const mockSearch = vi.fn().mockImplementation(({ page = 1 }) => Promise.resolve({
      results: [{
        provider: "discogs" as const,
        id: `dg-${page}`,
        title: `Discogs page ${page}`,
        formats: ["CD"],
      }],
      page,
      pageSize: 10,
      total: 30,
      hasNext: page < 3,
    }));
    window.api.searchReleases = mockSearch;
    render(<SearchDialog {...defaultProps} />);
    fireEvent.click(screen.getByText("Discogs"));
    fireEvent.change(screen.getByPlaceholderText("Artist name"), {
      target: { value: "Artist" },
    });
    fireEvent.click(screen.getByText("Search"));
    await waitFor(() => expect(screen.getByText("Discogs page 1")).toBeTruthy());

    fireEvent.click(document.querySelector("[title='Back to search']")!);
    fireEvent.click(screen.getByText("Search"));
    await waitFor(() => expect(screen.getByText("Discogs page 1")).toBeTruthy());
    expect(mockSearch).toHaveBeenCalledTimes(1);

    fireEvent.click(screen.getByText("Next >"));
    await waitFor(() => expect(screen.getByText("Discogs page 2")).toBeTruthy());
    expect(mockSearch).toHaveBeenCalledTimes(2);

    fireEvent.click(document.querySelector("[title='Back to search']")!);
    fireEvent.click(screen.getByText("Search"));
    fireEvent.click(screen.getByText("Next >"));
    await waitFor(() => expect(screen.getByText("Discogs page 2")).toBeTruthy());
    expect(mockSearch).toHaveBeenCalledTimes(2);
  });

  it("retries an abandoned server page after returning to the cached query", async () => {
    let resolveAbandonedPage!: (page: ReleaseSearchPage) => void;
    const abandonedPage = new Promise<ReleaseSearchPage>((resolve) => {
      resolveAbandonedPage = resolve;
    });
    let pageTwoAttempts = 0;
    const discogsPage = (page: number): ReleaseSearchPage => ({
      results: [{
        provider: "discogs",
        id: `dg-${page}`,
        title: `Discogs page ${page}`,
        formats: ["CD"],
      }],
      page,
      pageSize: 10,
      total: 20,
      hasNext: page === 1,
    });
    const mockSearch = vi.fn().mockImplementation(({ page = 1 }) => {
      if (page === 1) return Promise.resolve(discogsPage(1));
      pageTwoAttempts += 1;
      return pageTwoAttempts === 1
        ? abandonedPage
        : Promise.resolve(discogsPage(2));
    });
    window.api.searchReleases = mockSearch;
    render(<SearchDialog {...defaultProps} />);
    fireEvent.click(screen.getByText("Discogs"));
    const artistInput = screen.getByPlaceholderText("Artist name");
    fireEvent.change(artistInput, { target: { value: "Artist" } });
    fireEvent.click(screen.getByText("Search"));
    await waitFor(() => expect(screen.getByText("Discogs page 1")).toBeTruthy());

    fireEvent.click(screen.getByText("Next >"));
    await waitFor(() => expect(pageTwoAttempts).toBe(1));
    fireEvent.click(document.querySelector("[title='Back to search']")!);
    fireEvent.keyDown(screen.getByPlaceholderText("Artist name"), { key: "Enter" });
    await waitFor(() => expect(screen.getByText("Discogs page 1")).toBeTruthy());
    fireEvent.click(screen.getByText("Next >"));

    await waitFor(() => expect(screen.getByText("Discogs page 2")).toBeTruthy());
    expect(pageTwoAttempts).toBe(2);
    resolveAbandonedPage(discogsPage(2));
  });

  it("stops loading when Back abandons a server page request", async () => {
    const mockSearch = vi.fn()
      .mockResolvedValueOnce({
        results: [{
          provider: "discogs" as const,
          id: "dg-1",
          title: "Discogs page 1",
          formats: ["CD"],
        }],
        page: 1,
        pageSize: 10,
        total: 20,
        hasNext: true,
      })
      .mockReturnValueOnce(new Promise<ReleaseSearchPage>(() => {}));
    window.api.searchReleases = mockSearch;
    render(<SearchDialog {...defaultProps} />);
    fireEvent.click(screen.getByText("Discogs"));
    fireEvent.change(screen.getByPlaceholderText("Artist name"), {
      target: { value: "Artist" },
    });
    fireEvent.click(screen.getByText("Search"));
    await waitFor(() => expect(screen.getByText("Discogs page 1")).toBeTruthy());

    fireEvent.click(screen.getByText("Next >"));
    await waitFor(() => expect(mockSearch).toHaveBeenCalledTimes(2));
    fireEvent.click(document.querySelector("[title='Back to search']")!);

    expect((screen.getByText("Search") as HTMLButtonElement).disabled).toBe(false);
  });

  it("uses all trimmed search fields and provider to invalidate the latest-query cache", async () => {
    const mockSearch = vi.fn().mockResolvedValue(makeSearchPage());
    window.api.searchReleases = mockSearch;
    render(<SearchDialog {...defaultProps} />);
    fireEvent.change(screen.getByPlaceholderText("Artist name"), {
      target: { value: "  Radiohead  " },
    });
    fireEvent.click(screen.getByText("Search"));
    await waitFor(() => expect(screen.getByText("OK Computer")).toBeTruthy());

    fireEvent.click(document.querySelector("[title='Back to search']")!);
    fireEvent.change(screen.getByPlaceholderText("Artist name"), {
      target: { value: "Radiohead" },
    });
    fireEvent.click(screen.getByText("Search"));
    await waitFor(() => expect(screen.getByText("OK Computer")).toBeTruthy());
    expect(mockSearch).toHaveBeenCalledTimes(1);

    const changedFields = [
      ["Artist name", "Muse"],
      ["Album title", "Kid A"],
      ["e.g. 2004", "2000"],
      ["e.g. US", "GB"],
      ["e.g. CD", "Vinyl"],
      ["e.g. CDP 7243...", "CAT-1"],
      ["UPC/EAN", "123456"],
    ] as const;
    for (const [placeholder, value] of changedFields) {
      fireEvent.click(document.querySelector("[title='Back to search']")!);
      fireEvent.change(screen.getByPlaceholderText(placeholder), { target: { value } });
      fireEvent.click(screen.getByText("Search"));
      await waitFor(() => expect(screen.getByText("OK Computer")).toBeTruthy());
    }

    fireEvent.click(document.querySelector("[title='Back to search']")!);
    fireEvent.click(screen.getByText("Discogs"));
    fireEvent.click(screen.getByText("Search"));
    await waitFor(() => expect(screen.getByText("OK Computer")).toBeTruthy());
    expect(mockSearch).toHaveBeenCalledTimes(9);

    fireEvent.click(document.querySelector("[title='Back to search']")!);
    fireEvent.click(screen.getByText("MusicBrainz"));
    fireEvent.click(screen.getByText("Search"));
    await waitFor(() => expect(screen.getByText("OK Computer")).toBeTruthy());
    expect(mockSearch).toHaveBeenCalledTimes(10);
  });

  it("caches a successful empty result", async () => {
    const mockSearch = vi.fn().mockResolvedValue({
      results: [], page: 1, pageSize: 10, total: 0, hasNext: false,
    });
    window.api.searchReleases = mockSearch;
    render(<SearchDialog {...defaultProps} />);
    fireEvent.change(screen.getByPlaceholderText("Album title"), {
      target: { value: "Missing Album" },
    });
    fireEvent.click(screen.getByText("Search"));
    await waitFor(() => expect(screen.getByText(/No releases found/)).toBeTruthy());

    fireEvent.click(document.querySelector("[title='Back to search']")!);
    fireEvent.click(screen.getByText("Search"));
    await waitFor(() => expect(screen.getByText(/No releases found/)).toBeTruthy());
    expect(mockSearch).toHaveBeenCalledTimes(1);
  });

  it("does not cache a partial MusicBrainz catalog after a later page fails", async () => {
    const mockSearch = vi.fn()
      .mockImplementationOnce(() => Promise.resolve(makePagedSearchPage(1, 100, 101)))
      .mockRejectedValueOnce(new Error("Page 2 failed"))
      .mockImplementation(({ page = 1, pageSize = 100 }) =>
        Promise.resolve(makePagedSearchPage(page, pageSize, 101)),
      );
    window.api.searchReleases = mockSearch;
    render(<SearchDialog {...defaultProps} />);
    fireEvent.change(screen.getByPlaceholderText("Artist name"), {
      target: { value: "Artist" },
    });
    fireEvent.click(screen.getByText("Search"));
    await waitFor(() => expect(screen.getByText("Page 2 failed")).toBeTruthy());

    fireEvent.click(screen.getByText("Search"));
    await waitFor(() => expect(screen.getByText("Results (101)")).toBeTruthy());
    expect(mockSearch.mock.calls.map(([request]) => request.page)).toEqual([1, 2, 1, 2]);
  });

  it("shows empty state when no results", async () => {
    window.api.searchReleases = vi.fn().mockResolvedValue({
      results: [], page: 1, pageSize: 10, total: 0, hasNext: false,
    });
    render(<SearchDialog {...defaultProps} />);
    fireEvent.change(screen.getByPlaceholderText("Artist name"), {
      target: { value: "Unknown Artist" },
    });
    fireEvent.click(screen.getByText("Search"));
    await waitFor(() => {
      expect(screen.getByText(/No releases found/)).toBeTruthy();
    });
  });

  it("shows error state and retry works", async () => {
    window.api.searchReleases = vi.fn().mockRejectedValue(new Error("Network error"));
    render(<SearchDialog {...defaultProps} />);
    fireEvent.change(screen.getByPlaceholderText("Artist name"), {
      target: { value: "Radiohead" },
    });
    fireEvent.click(screen.getByText("Search"));
    await waitFor(() => {
      expect(screen.getByText(/Network error/)).toBeTruthy();
    });
    // Retry works
    window.api.searchReleases = vi.fn().mockResolvedValue(makeSearchPage());
    fireEvent.click(screen.getByText("Search"));
    await waitFor(() => {
      expect(screen.getByText("OK Computer")).toBeTruthy();
    });
  });

  it("opens detail view when clicking a result", async () => {
    render(<SearchDialog {...defaultProps} />);
    fireEvent.change(screen.getByPlaceholderText("Artist name"), {
      target: { value: "Radiohead" },
    });
    fireEvent.click(screen.getByText("Search"));
    await waitFor(() => {
      expect(screen.getByText("OK Computer")).toBeTruthy();
    });
    fireEvent.click(screen.getByText("OK Computer"));
    await waitFor(() => {
      expect(window.api.resolveRelease).toHaveBeenCalledWith("musicbrainz", "mb-1", undefined);
      expect(screen.getByText("Airbag")).toBeTruthy();
    });
  });

  it("back from detail returns to results", async () => {
    render(<SearchDialog {...defaultProps} />);
    fireEvent.change(screen.getByPlaceholderText("Artist name"), {
      target: { value: "Radiohead" },
    });
    fireEvent.click(screen.getByText("Search"));
    await waitFor(() => {
      expect(screen.getByText("OK Computer")).toBeTruthy();
    });
    fireEvent.click(screen.getByText("OK Computer"));
    await waitFor(() => {
      expect(screen.getByText("Airbag")).toBeTruthy();
    });
    // Back button
    const backBtn = document.querySelector("[title='Back to results']");
    expect(backBtn).toBeTruthy();
    fireEvent.click(backBtn!);
    await waitFor(() => {
      expect(screen.getByText("OK Computer")).toBeTruthy();
    });
  });

  it("selects a release and calls onSelectRelease", async () => {
    render(<SearchDialog {...defaultProps} />);
    fireEvent.change(screen.getByPlaceholderText("Artist name"), {
      target: { value: "Radiohead" },
    });
    fireEvent.click(screen.getByText("Search"));
    await waitFor(() => {
      expect(screen.getByText("OK Computer")).toBeTruthy();
    });
    fireEvent.click(screen.getByText("OK Computer"));
    await waitFor(() => {
      expect(screen.getByText("Airbag")).toBeTruthy();
    });
    fireEvent.click(screen.getByText("Select this release"));
    await waitFor(() => {
      expect(defaultProps.onSelectRelease).toHaveBeenCalledWith(
        expect.objectContaining({ id: "mb-1" }),
        "musicbrainz",
      );
    });
  });

  it("cancels without calling write methods", () => {
    render(<SearchDialog {...defaultProps} />);
    const closeBtn = document.querySelector("[aria-label='Search releases'] button:last-child");
    if (closeBtn) fireEvent.click(closeBtn);
    expect(defaultProps.onClose).toHaveBeenCalled();
    expect(window.api.resolveRelease).not.toHaveBeenCalled();
  });

  it("switches provider label for catalog number", () => {
    render(<SearchDialog {...defaultProps} />);
    // Default is MusicBrainz
    const labels = screen.getAllByText(/Catalog Number/);
    expect(labels.length).toBeGreaterThanOrEqual(1);
    // Switch to Discogs
    fireEvent.click(screen.getByText("Discogs"));
    expect(screen.getByText(/Catalog Number/)).toBeTruthy();
  });

  it("accepts artist-only search on MusicBrainz", async () => {
    const mockSearch = vi.fn().mockResolvedValue(makeSearchPage());
    window.api.searchReleases = mockSearch;
    render(<SearchDialog {...defaultProps} />);
    fireEvent.change(screen.getByPlaceholderText("Artist name"), {
      target: { value: "Radiohead" },
    });
    fireEvent.click(screen.getByText("Search"));
    await waitFor(() => {
      expect(mockSearch).toHaveBeenCalledWith(
        expect.objectContaining({ artist: "Radiohead", album: undefined }),
      );
    });
  });

  it("accepts album-only search on MusicBrainz", async () => {
    const mockSearch = vi.fn().mockResolvedValue(makeSearchPage());
    window.api.searchReleases = mockSearch;
    render(<SearchDialog {...defaultProps} />);
    fireEvent.change(screen.getByPlaceholderText("Album title"), {
      target: { value: "OK Computer" },
    });
    fireEvent.click(screen.getByText("Search"));
    await waitFor(() => {
      expect(mockSearch).toHaveBeenCalledWith(
        expect.objectContaining({ album: "OK Computer", artist: undefined }),
      );
    });
  });

  it("accepts artist-only search on Discogs", async () => {
    const mockSearch = vi.fn().mockResolvedValue(makeSearchPage());
    window.api.searchReleases = mockSearch;
    render(<SearchDialog {...defaultProps} />);
    fireEvent.click(screen.getByText("Discogs"));
    fireEvent.change(screen.getByPlaceholderText("Artist name"), {
      target: { value: "Nirvana" },
    });
    fireEvent.click(screen.getByText("Search"));
    await waitFor(() => {
      expect(mockSearch).toHaveBeenCalledWith(
        expect.objectContaining({ artist: "Nirvana", album: undefined }),
      );
    });
  });

  it("sends undefined for blank optional fields", async () => {
    const mockSearch = vi.fn().mockResolvedValue(makeSearchPage());
    window.api.searchReleases = mockSearch;
    render(<SearchDialog {...defaultProps} />);
    fireEvent.change(screen.getByPlaceholderText("Artist name"), {
      target: { value: "Radiohead" },
    });
    fireEvent.change(screen.getByPlaceholderText("Album title"), {
      target: { value: "OK Computer" },
    });
    fireEvent.click(screen.getByText("Search"));
    await waitFor(() => {
      const call = mockSearch.mock.calls[0][0];
      expect(call.year).toBeUndefined();
      expect(call.country).toBeUndefined();
      expect(call.format).toBeUndefined();
      expect(call.catalogNumber).toBeUndefined();
      expect(call.barcode).toBeUndefined();
    });
  });

  it("sends trim whitespace values", async () => {
    const mockSearch = vi.fn().mockResolvedValue(makeSearchPage());
    window.api.searchReleases = mockSearch;
    render(<SearchDialog {...defaultProps} />);
    fireEvent.change(screen.getByPlaceholderText("Artist name"), {
      target: { value: "  Radiohead  " },
    });
    fireEvent.click(screen.getByText("Search"));
    await waitFor(() => {
      expect(mockSearch).toHaveBeenCalledWith(
        expect.objectContaining({ artist: "Radiohead" }),
      );
    });
  });

  it("sends undefined for whitespace-only optional fields", async () => {
    const mockSearch = vi.fn().mockResolvedValue(makeSearchPage());
    window.api.searchReleases = mockSearch;
    render(<SearchDialog {...defaultProps} />);
    fireEvent.change(screen.getByPlaceholderText("Artist name"), {
      target: { value: "Radiohead" },
    });
    fireEvent.change(screen.getByPlaceholderText("Album title"), {
      target: { value: "OK Computer" },
    });
    fireEvent.change(screen.getByPlaceholderText("e.g. 2004"), {
      target: { value: "  " },
    });
    fireEvent.click(screen.getByText("Search"));
    await waitFor(() => {
      const call = mockSearch.mock.calls[0][0];
      expect(call.year).toBeUndefined();
    });
  });

  it("shows Artist or Album is required hint", () => {
    render(<SearchDialog {...defaultProps} />);
    expect(screen.getByText(/Artist or Album is required/)).toBeTruthy();
    expect(screen.getByText(/All other fields are optional/)).toBeTruthy();
  });

  // ── Discogs-specific ─────────────────────────────────────────

  it("accepts album-only search on Discogs", async () => {
    const mockSearch = vi.fn().mockResolvedValue(makeSearchPage());
    window.api.searchReleases = mockSearch;
    render(<SearchDialog {...defaultProps} />);
    fireEvent.click(screen.getByText("Discogs"));
    fireEvent.change(screen.getByPlaceholderText("Album title"), {
      target: { value: "Nevermind" },
    });
    fireEvent.click(screen.getByText("Search"));
    await waitFor(() => {
      expect(mockSearch).toHaveBeenCalledWith(
        expect.objectContaining({ album: "Nevermind", artist: undefined }),
      );
    });
  });

  it("accepts artist+album search on Discogs", async () => {
    const mockSearch = vi.fn().mockResolvedValue(makeSearchPage());
    window.api.searchReleases = mockSearch;
    render(<SearchDialog {...defaultProps} />);
    fireEvent.click(screen.getByText("Discogs"));
    fireEvent.change(screen.getByPlaceholderText("Artist name"), {
      target: { value: "Nirvana" },
    });
    fireEvent.change(screen.getByPlaceholderText("Album title"), {
      target: { value: "Nevermind" },
    });
    fireEvent.click(screen.getByText("Search"));
    await waitFor(() => {
      expect(mockSearch).toHaveBeenCalledWith(
        expect.objectContaining({ artist: "Nirvana", album: "Nevermind" }),
      );
    });
  });
});
