// @vitest-environment jsdom
/**
 * Parity tests for the independent browser tagging UI.
 *
 * These encode why the UI matters: the browser workspace is a parity surface
 * for Music Tag Web V2, so its column set, column order, density and patch
 * semantics are contracts, not incidental layout. Widths and row/header
 * heights asserted here are the values measured on the reference DOM; if a
 * change moves them, the browser UI stops matching the reference.
 */

import { afterEach, describe, expect, it, vi } from "vitest";
import React from "react";
import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import type { DirEntry, DirectoryData, LibraryRoot, TrackData } from "../../src/shared/desktop-api";
import {
  WEB_TABLE_COLUMNS,
  WEB_TABLE_TOTAL_WIDTH,
  fileNameOf,
  hasSidecarLrc,
} from "../../src/web/columns";
import { TrackTable } from "../../src/web/TrackTable";
import {
  buildTrackPatch,
  formatBitrate,
  formatBytes,
  formatDuration,
  parseArtistList,
  TrackDetailEditor,
} from "../../src/web/TrackDetailEditor";
import { WebShell } from "../../src/web/WebShell";
import { WorkspaceView } from "../../src/web/WorkspaceView";

afterEach(cleanup);

function track(overrides: Partial<TrackData> = {}): TrackData {
  return {
    path: "/library/Artist Alpha/Reference Album One/01 - Opening Track.flac",
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

describe("workspace table parity contract", () => {
  it("uses the reference's ten columns in the reference's order and labels", () => {
    expect(WEB_TABLE_COLUMNS.map((c) => c.label)).toEqual([
      "Cover",
      "File Name",
      "Title",
      "Artists",
      "Albums",
      "Album Artist",
      "Lyrics",
      "LRC",
      "Year",
      "Genres",
    ]);
  });

  it("keeps the reference's measured column widths", () => {
    expect(WEB_TABLE_COLUMNS.map((c) => c.width)).toEqual([
      80, 174, 118, 92, 148, 100, 62, 62, 56, 78,
    ]);
    // The reference table is 971px wide; its columns total 970px.
    expect(WEB_TABLE_TOTAL_WIDTH).toBe(970);
  });

  it("renders one header cell per reference column", () => {
    render(
      <TrackTable
        tracks={[track()]}
        selectedPaths={[]}
        onSelectTrack={vi.fn()}
      />,
    );

    const headers = screen.getAllByRole("columnheader").map((th) => th.textContent?.trim());
    expect(headers).toEqual([
      "Cover",
      "File Name",
      "Title",
      "Artists",
      "Albums",
      "Album Artist",
      "Lyrics",
      "LRC",
      "Year",
      "Genres",
    ]);
  });

  it("shows tag values, not backend identifiers, in the reference columns", () => {
    render(
      <TrackTable
        tracks={[track({ artists: ["Artist Alpha", "Artist Beta"] })]}
        selectedPaths={[]}
        directoryNames={new Set(["01 - opening track.lrc"])}
        onSelectTrack={vi.fn()}
      />,
    );

    const row = screen.getByTestId("workspace-row");
    const cells = within(row).getAllByRole("cell");
    expect(cells[1].textContent).toBe("01 - Opening Track.flac");
    expect(cells[2].textContent).toBe("Opening Track");
    expect(cells[3].textContent).toBe("Artist Alpha, Artist Beta");
    expect(cells[4].textContent).toBe("Reference Album One");
    expect(cells[5].textContent).toBe("Artist Alpha");
    expect(cells[6].textContent).toBe("No");
    // LRC presence is derived from the directory listing, case-insensitively.
    expect(cells[7].textContent).toBe("Yes");
    expect(cells[8].textContent).toBe("2019");
    expect(cells[9].textContent).toBe("Ambient");
  });

  it("derives LRC sidecar presence from the directory listing", () => {
    const names = new Set(["song.lrc", "other.txt"]);
    expect(hasSidecarLrc("/m/Song.flac", names)).toBe(true);
    expect(hasSidecarLrc("/m/Missing.flac", names)).toBe(false);
    expect(hasSidecarLrc("/m/no-extension", names)).toBe(false);
  });

  it("falls back from album artist to album artists, and from artists to artist", () => {
    render(
      <TrackTable
        tracks={[
          track({
            albumArtist: null,
            albumArtists: ["Beta One", "Beta Two"],
            artists: [],
            artist: "Solo Artist",
          }),
        ]}
        selectedPaths={[]}
        onSelectTrack={vi.fn()}
      />,
    );

    const cells = within(screen.getByTestId("workspace-row")).getAllByRole("cell");
    expect(cells[3].textContent).toBe("Solo Artist");
    expect(cells[5].textContent).toBe("Beta One, Beta Two");
  });
});

describe("workspace selection", () => {
  const tracks = [
    track({ path: "/m/a.flac" }),
    track({ path: "/m/b.flac" }),
    track({ path: "/m/c.flac" }),
  ];

  it("selects a single track by its file-name cell", () => {
    const onSelectTrack = vi.fn();
    render(<TrackTable tracks={tracks} selectedPaths={[]} onSelectTrack={onSelectTrack} />);

    const rows = screen.getAllByTestId("workspace-row");
    fireEvent.click(within(rows[1]).getByRole("button"));

    expect(onSelectTrack).toHaveBeenCalledWith("/m/b.flac");
  });

  it("adds to the selection on ctrl-click so batch editing is reachable", () => {
    const onSelectionChange = vi.fn();
    render(
      <TrackTable
        tracks={tracks}
        selectedPaths={["/m/a.flac"]}
        onSelectTrack={vi.fn()}
        onSelectionChange={onSelectionChange}
      />,
    );

    const rows = screen.getAllByTestId("workspace-row");
    fireEvent.click(within(rows[2]).getByRole("button"), { ctrlKey: true });

    expect(onSelectionChange).toHaveBeenCalledWith(["/m/a.flac", "/m/c.flac"]);
  });

  it("selects a contiguous range on shift-click", () => {
    const onSelectionChange = vi.fn();
    render(
      <TrackTable
        tracks={tracks}
        selectedPaths={["/m/a.flac"]}
        onSelectTrack={vi.fn()}
        onSelectionChange={onSelectionChange}
      />,
    );

    const rows = screen.getAllByTestId("workspace-row");
    fireEvent.click(within(rows[0]).getByRole("button"));
    fireEvent.click(within(rows[2]).getByRole("button"), { shiftKey: true });

    expect(onSelectionChange).toHaveBeenLastCalledWith(["/m/a.flac", "/m/b.flac", "/m/c.flac"]);
  });

  it("marks selected rows for assistive technology", () => {
    render(
      <TrackTable tracks={tracks} selectedPaths={["/m/b.flac"]} onSelectTrack={vi.fn()} />,
    );
    const rows = screen.getAllByTestId("workspace-row");
    expect(rows[1].getAttribute("aria-selected")).toBe("true");
    expect(rows[0].getAttribute("aria-selected")).toBe("false");
  });

  it("reports sort changes through the column header", () => {
    const onSortChange = vi.fn();
    render(
      <TrackTable
        tracks={tracks}
        selectedPaths={[]}
        onSelectTrack={vi.fn()}
        sort={{ key: "fileName", ascending: true }}
        onSortChange={onSortChange}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: /Title/ }));
    expect(onSortChange).toHaveBeenCalledWith({ key: "title", ascending: true });
  });
});

describe("detail editor patch semantics", () => {
  it("splits a disc value into discNumber and discTotal", () => {
    const patch = buildTrackPatch({
      title: "T",
      artistsText: "A",
      album: "AL",
      albumArtist: "AA",
      year: "2019",
      genre: "G",
      composer: "",
      comment: "",
      description: "",
      lyrics: "",
      discNumber: "1",
      discTotal: "2",
      trackNumber: "3",
      trackTotal: "4",
    });

    expect(patch.discNumber).toBe(1);
    expect(patch.discTotal).toBe(2);
    expect(patch.trackNumber).toBe(3);
    expect(patch.trackTotal).toBe(4);
  });

  it("clears optional fields instead of sending empty strings", () => {
    const patch = buildTrackPatch({
      title: "T",
      artistsText: "",
      album: "",
      albumArtist: "",
      year: "",
      genre: "",
      composer: "",
      comment: "",
      description: "",
      lyrics: "",
      discNumber: "",
      discTotal: "",
      trackNumber: "",
      trackTotal: "",
    });

    expect(patch.title).toBe("T");
    expect(patch.album).toBeNull();
    expect(patch.genre).toBeNull();
    expect(patch.comment).toBeNull();
    expect(patch.description).toBeNull();
    expect(patch.trackNumber).toBeNull();
    expect(patch.lyrics).toBeNull();
    expect(patch.artists).toEqual([]);
    expect(patch.discNumber).toBeNull();
  });

  it("turns lyrics text into a lyrics document", () => {
    const patch = buildTrackPatch({
      title: "",
      artistsText: "",
      album: "",
      albumArtist: "",
      year: "",
      genre: "",
      composer: "",
      comment: "",
      description: "",
      lyrics: "line one",
      discNumber: "",
      discTotal: "",
      trackNumber: "",
      trackTotal: "",
    });
    expect(patch.lyrics).toEqual({ plainLyrics: "line one", syncedLyrics: null });
  });

  it("splits comma and semicolon separated artist credits", () => {
    expect(parseArtistList("Alpha, Beta; Gamma")).toEqual(["Alpha", "Beta", "Gamma"]);
    expect(parseArtistList("  ")).toEqual([]);
  });

  it("formats technical readouts and degrades to an em dash", () => {
    expect(formatDuration(12)).toBe("0:12");
    expect(formatDuration(125)).toBe("2:05");
    expect(formatDuration(null)).toBe("—");
    expect(formatBitrate(95000)).toBe("95 kbps");
    expect(formatBitrate(0)).toBe("—");
    expect(formatBytes(157286)).toBe("154 KB");
    expect(formatBytes(null)).toBe("—");
  });

  it("renders the reference's editor sections", () => {
    render(<TrackDetailEditor track={track()} onApply={vi.fn()} />);

    const headings = screen.getAllByRole("heading", { level: 3 }).map((h) => h.textContent);
    expect(headings).toEqual([
      "Basic Information",
      "Media Information",
      "Lyric Information",
      "Category Information",
      "Metadata Information",
      "Technical Information",
    ]);
  });

  it("asks for a selection instead of rendering a half-empty editor", () => {
    render(<TrackDetailEditor track={null} onApply={vi.fn()} />);
    expect(screen.getByTestId("editor-empty")).toBeTruthy();
    expect(screen.queryByTestId("track-editor")).toBeNull();
  });

  it("leaves the file name out of the file details", () => {
    render(<TrackDetailEditor track={track()} onApply={vi.fn()} />);

    // The file name is not an editable tag, and the file table already carries
    // it as its own column, so repeating it in the details pane only crowded
    // out the fields the user can actually change.
    expect(screen.queryByTestId("editor-file-name")).toBeNull();
    expect(screen.queryByText("File Name")).toBeNull();
    expect(screen.getByLabelText("Title")).toBeTruthy();
  });

  it("disables Apply until a field changes, so Cancel can never write", () => {
    render(<TrackDetailEditor track={track()} onApply={vi.fn()} />);
    const apply = screen.getByRole("button", { name: "Apply" }) as HTMLButtonElement;
    expect(apply.disabled).toBe(true);

    fireEvent.change(screen.getByLabelText("Title"), { target: { value: "Changed" } });
    expect((screen.getByRole("button", { name: "Apply" }) as HTMLButtonElement).disabled).toBe(false);

    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect((screen.getByRole("button", { name: "Apply" }) as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByLabelText("Title") as HTMLInputElement).value).toBe("Opening Track");
  });

  it("submits the staged patch through onApply", () => {
    const onApply = vi.fn();
    render(<TrackDetailEditor track={track()} onApply={onApply} />);

    fireEvent.change(screen.getByLabelText("Genres"), { target: { value: "Jazz" } });
    fireEvent.click(screen.getByRole("button", { name: "Apply" }));

    expect(onApply).toHaveBeenCalledTimes(1);
    expect(onApply.mock.calls[0][0].genre).toBe("Jazz");
  });

  it("reports LRC sidecar state", () => {
    const { rerender } = render(
      <TrackDetailEditor track={track()} hasLrc onApply={vi.fn()} />,
    );
    expect(screen.getByTestId("editor-lrc-status").textContent).toBe("Present");
    rerender(<TrackDetailEditor track={track()} hasLrc={false} onApply={vi.fn()} />);
    expect(screen.getByTestId("editor-lrc-status").textContent).toBe("None");
  });
});

describe("browser shell", () => {
  it("offers the in-scope tagging destinations and marks the active one", () => {
    const onViewChange = vi.fn();
    render(<WebShell activeView="workspace" onViewChange={onViewChange}><p>body</p></WebShell>);

    expect(screen.getByRole("button", { name: "Workspace" }).getAttribute("aria-current")).toBe("page");
    expect(screen.getByRole("button", { name: "Operation log" }).getAttribute("aria-current")).toBeNull();

    fireEvent.click(screen.getByRole("button", { name: "Operation log" }));
    expect(onViewChange).toHaveBeenCalledWith("log");
  });

  it("offers no library or player destinations", () => {
    render(<WebShell><p>body</p></WebShell>);
    // Browsing and streaming surfaces stay out of scope: this UI is
    // tagging-centric.
    for (const label of ["Library", "Albums", "Artists", "Player"]) {
      expect(screen.queryByRole("button", { name: label })).toBeNull();
    }
    // Monitoring and Settings are in scope: they configure and observe the
    // background tagging runs, so their absence is a regression.
    for (const label of [
      "Workspace",
      "Monitoring",
      "Operation log",
      "Tag sources",
      "Settings",
    ]) {
      expect(screen.getByRole("button", { name: label })).toBeTruthy();
    }
  });

  it("exposes a sign-out control only when one is supplied", () => {
    const onSignOut = vi.fn();
    const { rerender } = render(<WebShell onSignOut={onSignOut}><p>b</p></WebShell>);
    fireEvent.click(screen.getByRole("button", { name: "Sign out" }));
    expect(onSignOut).toHaveBeenCalled();

    rerender(<WebShell><p>b</p></WebShell>);
    expect(screen.queryByRole("button", { name: "Sign out" })).toBeNull();
  });
});

describe("workspace data flow", () => {
  function fakeApi(options: {
    roots?: LibraryRoot[];
    directory?: Partial<DirectoryData>;
    entries?: DirEntry[];
    cover?: string | null;
  } = {}) {
    const roots = options.roots ?? [{ id: "1", name: "Music", path: "/library" }];
    const tracks = (options.directory?.tracks as TrackData[]) ?? [track()];
    return {
      listLibraryRoots: vi.fn(async () => roots),
      readDirectory: vi.fn(async (path: string) => ({
        path,
        name: "library",
        subdirs: options.directory?.subdirs ?? [],
        tracks,
        audioCount: tracks.length,
      })),
      listDirectory: vi.fn(async () => options.entries ?? []),
      getCoverDataUrl: vi.fn(async () => options.cover ?? null),
      writeTrack: vi.fn(async (_path: string, _fields: Record<string, unknown>) => track()),
    };
  }

  it("loads the first mounted root and lists its files", async () => {
    const api = fakeApi();
    render(<WorkspaceView api={api as unknown as Window["api"]} />);

    await waitFor(() => expect(screen.getByTestId("workspace-table")).toBeTruthy());
    expect(api.listLibraryRoots).toHaveBeenCalled();
    expect(api.readDirectory).toHaveBeenCalledWith("/library");
    expect(screen.getAllByTestId("workspace-row")).toHaveLength(1);
  });

  it("explains an unmounted library instead of rendering an empty table", async () => {
    const api = fakeApi({ roots: [] });
    render(<WorkspaceView api={api as unknown as Window["api"]} />);

    await waitFor(() => expect(screen.getByTestId("workspace-no-root")).toBeTruthy());
  });

  it("explains an empty folder and keeps the workspace usable", async () => {
    const api = fakeApi({ directory: { tracks: [] } });
    render(<WorkspaceView api={api as unknown as Window["api"]} />);

    await waitFor(() => expect(screen.getByTestId("workspace-empty")).toBeTruthy());
  });

  it("writes through the shared writer and reloads the directory", async () => {
    const api = fakeApi();
    render(<WorkspaceView api={api as unknown as Window["api"]} />);
    await waitFor(() => expect(screen.getByTestId("workspace-table")).toBeTruthy());

    fireEvent.click(within(screen.getByTestId("workspace-row")).getByRole("button"));
    fireEvent.change(await screen.findByLabelText("Title"), { target: { value: "Renamed" } });
    fireEvent.click(screen.getByRole("button", { name: "Apply" }));

    await waitFor(() => expect(api.writeTrack).toHaveBeenCalledTimes(1));
    expect(api.writeTrack.mock.calls[0][0]).toBe(track().path);
    expect(api.writeTrack.mock.calls[0][1].title).toBe("Renamed");
    expect(api.readDirectory).toHaveBeenCalledTimes(2);
  });

  it("surfaces a write failure instead of silently dropping it", async () => {
    const api = fakeApi();
    api.writeTrack.mockRejectedValueOnce(new Error("readback mismatch"));
    render(<WorkspaceView api={api as unknown as Window["api"]} />);
    await waitFor(() => expect(screen.getByTestId("workspace-table")).toBeTruthy());

    fireEvent.click(within(screen.getByTestId("workspace-row")).getByRole("button"));
    fireEvent.change(await screen.findByLabelText("Title"), { target: { value: "Renamed" } });
    fireEvent.click(screen.getByRole("button", { name: "Apply" }));

    expect(await screen.findByRole("alert")).toBeTruthy();
    expect(screen.getByRole("alert").textContent).toContain("readback mismatch");
  });

  it("filters the visible files without refetching", async () => {
    const api = fakeApi({
      directory: {
        tracks: [track({ path: "/m/keep.flac", title: "Keep" }), track({ path: "/m/drop.flac", title: "Drop" })],
      },
    });
    render(<WorkspaceView api={api as unknown as Window["api"]} />);
    await waitFor(() => expect(screen.getAllByTestId("workspace-row")).toHaveLength(2));

    fireEvent.change(screen.getByPlaceholderText("Search files or folders"), {
      target: { value: "keep" },
    });

    expect(screen.getAllByTestId("workspace-row")).toHaveLength(1);
    expect(api.readDirectory).toHaveBeenCalledTimes(1);
  });

  it("derives file names from paths on both separators", () => {
    expect(fileNameOf("/a/b/song.flac")).toBe("song.flac");
    expect(fileNameOf("C:\\\\a\\\\b\\\\song.flac")).toBe("song.flac");
    expect(fileNameOf("song.flac")).toBe("song.flac");
  });
});
