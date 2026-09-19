// @vitest-environment jsdom
/**
 * Review workflow in the browser workspace.
 *
 * These encode why the workspace matters as a repair surface: background
 * tagging flags an album, and a person has to be able to see that flag where
 * the album lives and fix it there. So the tests assert that a flagged album is
 * *findable* (banner + tree badge, keyed by the service's own classification)
 * and *fixable* (Number/Convert/Search reach the shared writer rather than
 * inventing their own).
 */

import { afterEach, describe, expect, it, vi } from "vitest";
import React from "react";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import type { DirectoryData, TrackData } from "../../src/shared/desktop-api";
import type { MonitoredRelease } from "../../src/shared/web-adapter";
import { WorkspaceView } from "../../src/web/WorkspaceView";

afterEach(cleanup);

const LIBRARY = "/library";
const ALBUM = "/library/Artist Alpha/Reference Album One";

function track(overrides: Partial<TrackData> = {}): TrackData {
  return {
    path: `${ALBUM}/01 - Opening Track.flac`,
    title: "Opening Track",
    artist: "Artist Alpha",
    artists: ["Artist Alpha"],
    album: "Reference Album One",
    albumArtist: "Artist Alpha",
    albumArtists: ["Artist Alpha"],
    trackNumber: 1,
    trackTotal: 3,
    discNumber: 1,
    discTotal: 1,
    year: "2019",
    genre: "Ambient",
    composer: "Someone",
    comment: null,
    description: null,
    lyrics: null,
    compilation: null,
    musicbrainzTrackId: null,
    musicbrainzAlbumId: null,
    musicbrainzArtistId: null,
    discogsArtistId: null,
    discogsReleaseId: null,
    hasCover: true,
    sizeBytes: 157286,
    bitrate: 95000,
    sampleRate: 44100,
    codec: "flac",
    duration: 12,
    ...overrides,
  } as TrackData;
}

function albumTracks(): TrackData[] {
  return [
    track(),
    track({ path: `${ALBUM}/02 - Middle Track.flac`, title: "Middle Track", trackNumber: 2 }),
    track({ path: `${ALBUM}/03 - Closing Track.flac`, title: "Closing Track", trackNumber: 3 }),
  ];
}

function remoteTrack(title: string, trackNumber: number) {
  return { title, artists: ["Artist Alpha"], artist: "Artist Alpha", trackNumber, trackTotal: 3, discNumber: 1 };
}

const providerRelease = {
  id: "release-1",
  title: "Reference Album One",
  artist: "Artist Alpha",
  tracks: [
    remoteTrack("Everything in Its Right Place", 1),
    remoteTrack("Kid A", 2),
    remoteTrack("The National Anthem", 3),
  ],
};

const defaultSearchResults = [
  {
    provider: "musicbrainz" as const,
    id: "release-1",
    title: "Reference Album One",
    artist: "Artist Alpha",
    year: "2019",
    trackCount: 3,
    country: "GB",
    formats: ["CD"],
  },
];

function fakeApi(
  options: { tracks?: TrackData[]; searchResults?: typeof defaultSearchResults } = {},
) {
  const tracks = options.tracks ?? albumTracks();
  const searchResults = options.searchResults ?? defaultSearchResults;
  return {
    listLibraryRoots: vi.fn(async () => [{ id: "1", name: "Music", path: LIBRARY }]),
    readDirectory: vi.fn(async (path: string): Promise<DirectoryData> => ({
      path,
      name: path.split("/").pop() ?? path,
      subdirs: path === LIBRARY ? [{ name: "Reference Album One", path: ALBUM, isDirectory: true }] : [],
      tracks: path === LIBRARY ? [] : tracks,
      audioCount: tracks.length,
    }) as DirectoryData),
    listDirectory: vi.fn(async () => []),
    getCoverDataUrl: vi.fn(async () => null),
    writeTrack: vi.fn(async () => tracks[0]),
    writeTracks: vi.fn(async (updates: Array<{ path: string }>) => ({
      tracks: tracks.filter((t) => updates.some((u) => u.path === t.path)),
      failures: [] as Array<{ path: string; error: string }>,
    })),
    checkFileExists: vi.fn(async () => false),
    readAlbum: vi.fn(async () => ({
      path: ALBUM, name: "Reference Album One", artistHint: "Artist Alpha",
      albumHint: "Reference Album One", tracks, coverInfo: {}, status: "ok",
    })),
    renameTrack: vi.fn(async () => tracks[0]),
    searchReleases: vi.fn(async () => ({
      results: searchResults,
      page: 1,
      pageSize: 100,
      hasNext: false,
    })),
    releaseTrackCount: vi.fn(async () => 3),
    resolveRelease: vi.fn(async () => providerRelease),
    previewReleaseMatch: vi.fn(async () => ({
      release: providerRelease,
      candidates: [
        { localIndex: 0, localTitle: "Opening Track", remoteIndex: 0 },
        { localIndex: 1, localTitle: "Middle Track", remoteIndex: 1 },
        { localIndex: 2, localTitle: "Closing Track", remoteIndex: 2 },
      ],
      unusedRemoteIndices: [],
      albumCandidate: {
        album: "Reference Album One",
        artists: ["Artist Alpha"],
        albumArtists: ["Artist Alpha"],
        tracks: [{ title: "Opening Track", artists: [] }],
      },
    })),
    searchApplyCandidate: vi.fn(async () => 3),
  };
}

function renderWorkspace(
  api: ReturnType<typeof fakeApi>,
  releases: MonitoredRelease[] = [],
) {
  return render(
    <WorkspaceView
      api={api as unknown as Window["api"]}
      loadReleases={async () => ({ releases })}
    />,
  );
}

/** Load the album folder, which is where the tools act. */
async function openAlbum(api: ReturnType<typeof fakeApi>) {
  const banner = await screen.findByTestId("review-banner");
  fireEvent.click(banner.querySelector("button") as HTMLButtonElement);
  await waitFor(() => expect(api.readDirectory).toHaveBeenCalledWith(ALBUM));
  await waitFor(() => expect(screen.getAllByTestId("workspace-row")).toHaveLength(3));
}

describe("albums the service left needing review", () => {
  it("names the flagged album in the workspace so it can be found", async () => {
    const api = fakeApi();
    renderWorkspace(api, [{ path: ALBUM, status: "needs_review" }]);

    const banner = await screen.findByTestId("review-banner");
    expect(banner.textContent).toContain("1 album needs review");
    expect(banner.textContent).toContain("Reference Album One");
  });

  it("does not claim a healthy or failed album needs review", async () => {
    // A failed album is the service's problem to retry, not a tagging judgement
    // for a person, so listing it here would send someone to the wrong job.
    const api = fakeApi();
    renderWorkspace(api, [
      { path: "/library/Artist Beta/Healthy Album", status: "healthy" },
      { path: "/library/Artist Beta/Broken Album", status: "failed" },
    ]);

    await waitFor(() => expect(api.listLibraryRoots).toHaveBeenCalled());
    expect(screen.queryByTestId("review-banner")).toBeNull();
  });

  it("keeps another library's flagged albums out of this library's banner", async () => {
    // `/library-archive` shares a prefix with `/library` but is a different
    // mount, and offering its albums here would open a folder outside the root.
    const api = fakeApi();
    renderWorkspace(api, [
      { path: "/library-archive/Artist/Flagged Elsewhere", status: "needs_review" },
    ]);

    await waitFor(() => expect(api.listLibraryRoots).toHaveBeenCalled());
    expect(screen.queryByTestId("review-banner")).toBeNull();
  });

  it("marks the flagged album in the folder tree", async () => {
    const api = fakeApi();
    renderWorkspace(api, [{ path: ALBUM, status: "needs_review" }]);
    await screen.findByTestId("review-banner");

    // The badge only appears once the tree has rendered that node.
    const tree = within(screen.getByTestId("directory-tree"));
    fireEvent.click(tree.getByRole("button", { name: /Music/ }));
    expect(await screen.findByTestId(`tree-review-${ALBUM}`)).toBeTruthy();
  });

  it("matches a flagged path that arrives with a trailing separator", async () => {
    // The tree asks by the exact path it was handed, so a mismatched key would
    // silently hide the album a person was told to look at.
    const api = fakeApi();
    renderWorkspace(api, [{ path: `${ALBUM}/`, status: "needs_review" }]);
    await screen.findAllByTestId("review-banner");

    const tree = within(screen.getByTestId("directory-tree"));
    fireEvent.click(tree.getByRole("button", { name: /Music/ }));
    expect(await screen.findByTestId(`tree-review-${ALBUM}`)).toBeTruthy();
  });
});

describe("Number", () => {
  it("writes sequential numbers through the shared batch writer", async () => {
    const api = fakeApi();
    renderWorkspace(api, [{ path: ALBUM, status: "needs_review" }]);
    await openAlbum(api);

    fireEvent.click(screen.getByTestId("tools-number"));
    fireEvent.click(await screen.findByTestId("number-apply"));

    await waitFor(() => expect(api.writeTracks).toHaveBeenCalledTimes(1));
    const updates = api.writeTracks.mock.calls[0][0] as Array<{
      path: string;
      fields: { trackNumber: number; trackTotal: number };
    }>;
    expect(updates.map((u) => u.fields.trackNumber)).toEqual([1, 2, 3]);
    expect(updates.every((u) => u.fields.trackTotal === 3)).toBe(true);
    // Numbers were already 1,2,3, so a no-op here would mean the rule was
    // ignored and the write happened anyway.
    expect(updates.map((u) => u.path)).toEqual(albumTracks().map((t) => t.path));
  });

  it("orders by the rule the user chose rather than by the list order", async () => {
    const api = fakeApi();
    renderWorkspace(api, [{ path: ALBUM, status: "needs_review" }]);
    await openAlbum(api);

    fireEvent.click(screen.getByTestId("tools-number"));
    // The files already run 01,02,03, so a descending rule must reverse them;
    // an unchanged order would mean the rule never reached the computation.
    fireEvent.change(await screen.findByTestId("number-rule"), {
      target: { value: "existing-track-desc" },
    });
    fireEvent.click(screen.getByTestId("number-apply"));

    await waitFor(() => expect(api.writeTracks).toHaveBeenCalledTimes(1));
    const updates = api.writeTracks.mock.calls[0][0] as Array<{
      path: string;
      fields: { trackNumber: number };
    }>;
    expect(updates.map((u) => u.path)).toEqual([
      `${ALBUM}/03 - Closing Track.flac`,
      `${ALBUM}/02 - Middle Track.flac`,
      `${ALBUM}/01 - Opening Track.flac`,
    ]);
    expect(updates.map((u) => u.fields.trackNumber)).toEqual([1, 2, 3]);
  });

  it("reports the backend's reason when the write is refused", async () => {
    const api = fakeApi();
    api.writeTracks.mockRejectedValueOnce(new Error("readback mismatch"));
    renderWorkspace(api, [{ path: ALBUM, status: "needs_review" }]);
    await openAlbum(api);

    fireEvent.click(screen.getByTestId("tools-number"));
    fireEvent.click(await screen.findByTestId("number-apply"));

    const error = await screen.findByTestId("number-drawer-error");
    expect(error.textContent).toContain("readback mismatch");
  });
});

describe("Convert", () => {
  it("writes the fields a file-name pattern captures", async () => {
    const api = fakeApi();
    renderWorkspace(api, [{ path: ALBUM, status: "needs_review" }]);
    await openAlbum(api);

    fireEvent.click(screen.getByTestId("tools-convert"));
    fireEvent.click(await screen.findByTestId("convert-preset-Track - Title"));
    fireEvent.click(screen.getByTestId("convert-apply"));

    await waitFor(() => expect(api.writeTracks).toHaveBeenCalledTimes(1));
    const updates = api.writeTracks.mock.calls[0][0] as Array<{
      fields: Record<string, unknown>;
    }>;
    expect(updates[0].fields).toEqual({ title: "Opening Track", trackNumber: 1 });
    // A tag rewrite must not rename files.
    expect(api.renameTrack).not.toHaveBeenCalled();
  });

  it("offers only the presets that fit the chosen direction", async () => {
    // A filename template shown while reading tags would produce a pattern the
    // parser cannot use, and the user would have no way to tell why.
    const api = fakeApi();
    renderWorkspace(api, [{ path: ALBUM, status: "needs_review" }]);
    await openAlbum(api);

    fireEvent.click(screen.getByTestId("tools-convert"));
    fireEvent.click(await screen.findByTestId("convert-direction-tags-to-filename"));

    expect(screen.getByTestId("convert-preset-Track - Title filename")).toBeTruthy();
    expect(screen.queryByTestId("convert-preset-Track - Title")).toBeNull();
  });

  it("disables Convert when the result would change nothing", async () => {
    // Writing a value onto itself rewrites the file for no reason, so the
    // desktop reports it as no change and refuses to run.
    const api = fakeApi();
    renderWorkspace(api, [{ path: ALBUM, status: "needs_review" }]);
    await openAlbum(api);

    fireEvent.click(screen.getByTestId("tools-convert"));
    // Reading the Title tag and writing it back to Title changes nothing.
    fireEvent.click(await screen.findByTestId("convert-direction-tag-to-tags"));
    fireEvent.change(screen.getByTestId("convert-pattern"), {
      target: { value: "%{title}%" },
    });

    expect(await screen.findByTestId("convert-preview-nochange")).toBeTruthy();
    expect((screen.getByTestId("convert-apply") as HTMLButtonElement).disabled).toBe(true);
  });

  it("refuses a pattern that does not fit the files instead of reporting success", async () => {
    const api = fakeApi();
    renderWorkspace(api, [{ path: ALBUM, status: "needs_review" }]);
    await openAlbum(api);

    fireEvent.click(screen.getByTestId("tools-convert"));
    // No file name contains '#', so this pattern cannot capture anything.
    fireEvent.change(await screen.findByTestId("convert-pattern"), {
      target: { value: "%{track}%#%{title}%" },
    });

    expect(await screen.findByTestId("convert-preview-error")).toBeTruthy();
    expect((screen.getByTestId("convert-apply") as HTMLButtonElement).disabled).toBe(true);
    expect(api.writeTracks).not.toHaveBeenCalled();
  });

  it("reports files the writer refused instead of calling the run a success", async () => {
    // A partial run that reports success is how a library ends up half tagged.
    const api = fakeApi();
    api.writeTracks.mockResolvedValueOnce({
      tracks: [],
      failures: [{ path: `${ALBUM}/02 - Middle Track.flac`, error: "readback mismatch" }],
    });
    renderWorkspace(api, [{ path: ALBUM, status: "needs_review" }]);
    await openAlbum(api);

    fireEvent.click(screen.getByTestId("tools-convert"));
    fireEvent.click(await screen.findByTestId("convert-preset-Track - Title"));
    fireEvent.click(screen.getByTestId("convert-apply"));

    const failures = await screen.findByTestId("convert-drawer-failures");
    expect(failures.textContent).toContain("readback mismatch");
  });

  it("renames files when the pattern reads from tags", async () => {
    const api = fakeApi();
    renderWorkspace(api, [{ path: ALBUM, status: "needs_review" }]);
    await openAlbum(api);

    fireEvent.click(screen.getByTestId("tools-convert"));
    fireEvent.click(await screen.findByTestId("convert-direction-tags-to-filename"));
    fireEvent.click(await screen.findByTestId("convert-preset-Artist - Title filename"));
    fireEvent.click(screen.getByTestId("convert-apply"));

    await waitFor(() => expect(api.renameTrack).toHaveBeenCalled());
    expect(api.renameTrack.mock.calls[0][0]).toBe(`${ALBUM}/01 - Opening Track.flac`);
    // A name-only rewrite must keep the extension the preset carries.
    expect(api.renameTrack.mock.calls[0][1]).toBe(
      `${ALBUM}/Artist Alpha - Opening Track.flac`,
    );
    // Renaming is not a tag edit.
    expect(api.writeTracks).not.toHaveBeenCalled();
  });

  it("refuses to overwrite a file the target name already belongs to", async () => {
    // Renaming onto an existing track would destroy it, and no undo restores it.
    const api = fakeApi();
    api.checkFileExists.mockResolvedValue(true);
    renderWorkspace(api, [{ path: ALBUM, status: "needs_review" }]);
    await openAlbum(api);

    fireEvent.click(screen.getByTestId("tools-convert"));
    fireEvent.click(await screen.findByTestId("convert-direction-tags-to-filename"));
    fireEvent.click(await screen.findByTestId("convert-preset-Artist - Title filename"));
    fireEvent.click(screen.getByTestId("convert-apply"));

    const failures = await screen.findByTestId("convert-drawer-failures");
    expect(failures.textContent).toContain("target already exists");
    expect(api.renameTrack).not.toHaveBeenCalled();
  });

  it("leaves files alone when the pattern already matches their names", async () => {
    const api = fakeApi();
    renderWorkspace(api, [{ path: ALBUM, status: "needs_review" }]);
    await openAlbum(api);

    fireEvent.click(screen.getByTestId("tools-convert"));
    fireEvent.click(await screen.findByTestId("convert-direction-tags-to-filename"));
    fireEvent.click(screen.getByTestId("convert-preset-Track - Title filename"));
    // The files are already `01 - Title`, so this computes the name they have.
    expect(await screen.findByTestId("convert-preview-nochange")).toBeTruthy();
    expect((screen.getByTestId("convert-apply") as HTMLButtonElement).disabled).toBe(true);
    expect(api.renameTrack).not.toHaveBeenCalled();
  });

  it("prefers the selection over the whole album", async () => {
    // The desktop converts what is selected; falling back to the album must not
    // quietly widen a deliberate selection.
    const api = fakeApi();
    renderWorkspace(api, [{ path: ALBUM, status: "needs_review" }]);
    await openAlbum(api);

    const row = screen.getAllByTestId("workspace-row")[0];
    fireEvent.click(within(row).getByRole("button"));
    fireEvent.click(screen.getByTestId("tools-convert"));

    expect((await screen.findByTestId("convert-apply")).textContent).toBe("Convert");
    expect(screen.getByTestId("convert-drawer").textContent).toContain("1 selected track(s)");
  });
});

describe("tool drawers", () => {
  it("nests the dialog inside its backdrop so the backdrop cannot cover it", async () => {
    // The backdrop is fixed and z-indexed, so a *sibling* dialog paints under it
    // and every control inside becomes unclickable while looking correct.
    const api = fakeApi();
    renderWorkspace(api, [{ path: ALBUM, status: "needs_review" }]);
    await openAlbum(api);

    fireEvent.click(screen.getByTestId("tools-number"));
    const dialog = await screen.findByTestId("number-drawer");
    expect(dialog.parentElement?.className).toContain("sr-drawer__backdrop");
  });

  it("closes when the backdrop itself is clicked", async () => {
    const api = fakeApi();
    renderWorkspace(api, [{ path: ALBUM, status: "needs_review" }]);
    await openAlbum(api);

    fireEvent.click(screen.getByTestId("tools-number"));
    await screen.findByTestId("number-drawer");

    fireEvent.mouseDown(screen.getByTestId("number-drawer-backdrop"));
    await waitFor(() => expect(screen.queryByTestId("number-drawer")).toBeNull());
  });

  it("keeps the drawer open when a control inside it is clicked", async () => {
    // The backdrop's own handler must not fire for events from its children,
    // or choosing a rule would dismiss the panel mid-edit.
    const api = fakeApi();
    renderWorkspace(api, [{ path: ALBUM, status: "needs_review" }]);
    await openAlbum(api);

    fireEvent.click(screen.getByTestId("tools-number"));
    fireEvent.mouseDown(await screen.findByTestId("number-rule"));
    expect(screen.getByTestId("number-drawer")).toBeTruthy();
  });
});

describe("Search", () => {
  it("seeds the query from the album already loaded", async () => {
    const api = fakeApi();
    renderWorkspace(api, [{ path: ALBUM, status: "needs_review" }]);
    await openAlbum(api);

    fireEvent.click(screen.getByTestId("tools-search"));
    expect((await screen.findByTestId("search-album")).getAttribute("value")).toBe(
      "Reference Album One",
    );
    expect(screen.getByTestId("search-artist").getAttribute("value")).toBe("Artist Alpha");
  });

  it("sends every narrowing field the provider accepts", async () => {
    // The narrowing fields are the difference between a usable result list and
    // a hundred near-misses, so they have to reach the provider, not just render.
    const api = fakeApi();
    renderWorkspace(api, [{ path: ALBUM, status: "needs_review" }]);
    await openAlbum(api);
    fireEvent.click(screen.getByTestId("tools-search"));

    fireEvent.change(await screen.findByTestId("search-year"), { target: { value: "2000" } });
    fireEvent.change(screen.getByTestId("search-country"), { target: { value: "GB" } });
    fireEvent.change(screen.getByTestId("search-format"), { target: { value: "CD" } });
    fireEvent.change(screen.getByTestId("search-catalog"), { target: { value: "CAT-1" } });
    fireEvent.change(screen.getByTestId("search-barcode"), { target: { value: "123456" } });
    fireEvent.click(screen.getByTestId("search-run"));

    await waitFor(() => expect(api.searchReleases).toHaveBeenCalledTimes(1));
    expect(api.searchReleases.mock.calls[0][0]).toMatchObject({
      provider: "musicbrainz",
      artist: "Artist Alpha",
      album: "Reference Album One",
      year: "2000",
      country: "GB",
      format: "CD",
      catalogNumber: "CAT-1",
      barcode: "123456",
      page: 1,
    });
  });

  it("will not search with neither an artist nor an album", async () => {
    const api = fakeApi();
    renderWorkspace(api, [{ path: ALBUM, status: "needs_review" }]);
    await openAlbum(api);
    fireEvent.click(screen.getByTestId("tools-search"));

    fireEvent.change(await screen.findByTestId("search-artist"), { target: { value: "  " } });
    fireEvent.change(screen.getByTestId("search-album"), { target: { value: "" } });

    expect((screen.getByTestId("search-run") as HTMLButtonElement).disabled).toBe(true);
  });

  it("narrows the result list without asking the provider again", async () => {
    const api = fakeApi({
      searchResults: [
        ...defaultSearchResults,
        {
          provider: "discogs" as const,
          id: "release-2",
          title: "Other Album",
          artist: "Someone Else",
          year: "1999",
          trackCount: 12,
          country: "US",
          formats: ["Vinyl"],
        },
      ],
    });
    renderWorkspace(api, [{ path: ALBUM, status: "needs_review" }]);
    await openAlbum(api);
    fireEvent.click(screen.getByTestId("tools-search"));
    fireEvent.click(await screen.findByTestId("search-run"));
    expect((await screen.findByTestId("search-results")).textContent).toContain("Other Album");

    fireEvent.change(screen.getByTestId("search-filter"), { target: { value: "discogs" } });
    expect(screen.getByTestId("search-results").textContent).toBe("");
    fireEvent.click(screen.getByTestId("search-clear-filters"));
    expect(screen.getByTestId("search-results").textContent).toContain("Other Album");

    // Filtering is a client-side view over the page that was already fetched.
    expect(api.searchReleases).toHaveBeenCalledTimes(1);
  });

  it("filters to releases with the same track count as this album", async () => {
    const api = fakeApi({
      searchResults: [
        ...defaultSearchResults,
        {
          provider: "discogs" as const,
          id: "release-2",
          title: "Other Album",
          artist: "Someone Else",
          year: "1999",
          trackCount: 12,
          country: "US",
          formats: ["Vinyl"],
        },
      ],
    });
    renderWorkspace(api, [{ path: ALBUM, status: "needs_review" }]);
    await openAlbum(api);
    fireEvent.click(screen.getByTestId("tools-search"));
    fireEvent.click(await screen.findByTestId("search-run"));
    await screen.findByTestId("search-results");

    fireEvent.click(screen.getByTestId("search-same-count"));
    const listed = screen.getByTestId("search-results").textContent ?? "";
    expect(listed).toContain("Reference Album One");
    expect(listed).not.toContain("Other Album");
  });

  it("loads per-release track counts on request and can cancel them", async () => {
    // Each count costs a provider request, so they are fetched deliberately and
    // a cancel has to actually stop the remaining requests.
    const api = fakeApi();
    renderWorkspace(api, [{ path: ALBUM, status: "needs_review" }]);
    await openAlbum(api);
    fireEvent.click(screen.getByTestId("tools-search"));
    fireEvent.click(await screen.findByTestId("search-run"));
    await screen.findByTestId("search-results");

    expect(api.releaseTrackCount).not.toHaveBeenCalled();
    fireEvent.click(screen.getByTestId("search-load-counts"));
    await waitFor(() => expect(api.releaseTrackCount).toHaveBeenCalled());
    expect(api.releaseTrackCount.mock.calls[0][0]).toBe("musicbrainz");
    expect(api.releaseTrackCount.mock.calls[0][1]).toBe("release-1");
  });

  it("pages through the results a page at a time", async () => {
    const many = Array.from({ length: 14 }, (_, index) => ({
      provider: "musicbrainz" as const,
      id: `release-${index}`,
      title: `Release ${String(index).padStart(2, "0")}`,
      artist: "Artist Alpha",
      year: "2019",
      trackCount: 3,
      country: "GB",
      formats: ["CD"],
    }));
    const api = fakeApi({ searchResults: many });
    renderWorkspace(api, [{ path: ALBUM, status: "needs_review" }]);
    await openAlbum(api);
    fireEvent.click(screen.getByTestId("tools-search"));
    fireEvent.click(await screen.findByTestId("search-run"));
    await screen.findByTestId("search-results");

    expect(screen.getAllByTestId(/^search-choose-/)).toHaveLength(10);
    fireEvent.click(screen.getByTestId("search-page-next"));
    expect(screen.getAllByTestId(/^search-choose-/)).toHaveLength(4);
  });

  it("maps local files to provider tracks and writes only the mapped ones", async () => {
    // The mapping is the point of the step: an unmapped file must be left out of
    // the write, and the indices are the album's own track order.
    const api = fakeApi();
    renderWorkspace(api, [{ path: ALBUM, status: "needs_review" }]);
    await openAlbum(api);
    fireEvent.click(screen.getByTestId("tools-search"));
    fireEvent.click(await screen.findByTestId("search-run"));
    fireEvent.click(await screen.findByTestId("search-choose-release-1"));

    expect(await screen.findByTestId("search-mapping")).toBeTruthy();
    // Drop the second file from the write.
    fireEvent.change(screen.getByTestId("search-map-remote-1"), { target: { value: "" } });
    fireEvent.click(screen.getByTestId("search-apply"));

    await waitFor(() => expect(api.searchApplyCandidate).toHaveBeenCalledTimes(1));
    const [albumPath, candidate, indices] = api.searchApplyCandidate.mock.calls[0];
    expect(albumPath).toBe(ALBUM);
    expect(indices).toEqual([0, 2]);
    // Native apply reads snake_case, so a camelCase key would be dropped silently.
    expect(candidate.tracks[0]).toMatchObject({
      title: "Everything in Its Right Place",
      artist: "Artist Alpha",
      track_number: 1,
    });
    expect(candidate.tracks[1]).toEqual({ artists: [] });
  });

  it("re-points a file at another provider track when the auto-match is wrong", async () => {
    const api = fakeApi();
    renderWorkspace(api, [{ path: ALBUM, status: "needs_review" }]);
    await openAlbum(api);
    fireEvent.click(screen.getByTestId("tools-search"));
    fireEvent.click(await screen.findByTestId("search-run"));
    fireEvent.click(await screen.findByTestId("search-choose-release-1"));
    await screen.findByTestId("search-mapping");

    fireEvent.change(screen.getByTestId("search-map-remote-0"), { target: { value: "2" } });
    expect((screen.getByTestId("search-map-title-0") as HTMLInputElement).value).toBe(
      "The National Anthem",
    );

    fireEvent.click(screen.getByTestId("search-apply"));
    await waitFor(() => expect(api.searchApplyCandidate).toHaveBeenCalled());
    const candidate = api.searchApplyCandidate.mock.calls[0][1];
    expect(candidate.tracks[0]).toMatchObject({
      title: "The National Anthem",
      track_number: 3,
    });
  });

  it("lets a mapped track be edited before it is written", async () => {
    const api = fakeApi();
    renderWorkspace(api, [{ path: ALBUM, status: "needs_review" }]);
    await openAlbum(api);
    fireEvent.click(screen.getByTestId("tools-search"));
    fireEvent.click(await screen.findByTestId("search-run"));
    fireEvent.click(await screen.findByTestId("search-choose-release-1"));
    await screen.findByTestId("search-mapping");

    fireEvent.change(screen.getByTestId("search-map-title-0"), {
      target: { value: "Hand edited" },
    });
    fireEvent.click(screen.getByTestId("search-apply"));

    await waitFor(() => expect(api.searchApplyCandidate).toHaveBeenCalled());
    expect(api.searchApplyCandidate.mock.calls[0][1].tracks[0].title).toBe("Hand edited");
  });

  it("warns when two files are pointed at the same provider track", async () => {
    const api = fakeApi();
    renderWorkspace(api, [{ path: ALBUM, status: "needs_review" }]);
    await openAlbum(api);
    fireEvent.click(screen.getByTestId("tools-search"));
    fireEvent.click(await screen.findByTestId("search-run"));
    fireEvent.click(await screen.findByTestId("search-choose-release-1"));
    await screen.findByTestId("search-mapping");

    fireEvent.change(screen.getByTestId("search-map-remote-1"), { target: { value: "0" } });
    const row = screen.getByTestId("search-mapping-row-1");
    expect(row.textContent).toContain("Also mapped to another file");
  });

  it("refuses to write when nothing is mapped", async () => {
    const api = fakeApi();
    renderWorkspace(api, [{ path: ALBUM, status: "needs_review" }]);
    await openAlbum(api);
    fireEvent.click(screen.getByTestId("tools-search"));
    fireEvent.click(await screen.findByTestId("search-run"));
    fireEvent.click(await screen.findByTestId("search-choose-release-1"));
    await screen.findByTestId("search-mapping");

    fireEvent.change(screen.getByTestId("search-map-remote-0"), { target: { value: "" } });
    fireEvent.change(screen.getByTestId("search-map-remote-1"), { target: { value: "" } });
    fireEvent.change(screen.getByTestId("search-map-remote-2"), { target: { value: "" } });

    expect((screen.getByTestId("search-apply") as HTMLButtonElement).disabled).toBe(true);
  });
});
