// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import React from "react";
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { WorkspaceView } from "../../src/web/WorkspaceView";
import type { DirectoryData, TrackData } from "../../src/shared/desktop-api";

afterEach(cleanup);

function makeTrack(path: string, title: string): TrackData {
  return {
    path,
    title,
    artist: "Artist",
    artists: ["Artist"],
    album: "Album",
    albumArtist: "Artist",
    albumArtists: ["Artist"],
    trackNumber: 1,
    trackTotal: 1,
    discNumber: null,
    discTotal: null,
    year: "2024",
    genre: "Rock",
    composer: null,
    comment: null,
    description: null,
    lyrics: null,
    compilation: null,
    musicbrainzTrackId: null,
    musicbrainzAlbumId: null,
    musicbrainzArtistId: null,
    discogsArtistId: null,
    discogsReleaseId: null,
    hasCover: false,
    sizeBytes: 1,
    bitrate: 128000,
    sampleRate: 44100,
    codec: "mp3",
    duration: 180,
  };
}

function directory(path: string, subdirs: DirectoryData["subdirs"], tracks: TrackData[] = []): DirectoryData {
  return {
    path,
    name: path.split("/").pop() ?? "",
    subdirs,
    tracks,
    audioCount: tracks.length,
  };
}

describe("WorkspaceView", () => {
  it("keeps the folder tree populated when arriving directly at an album during startup", async () => {
    const rootPath = "/music/Artist";
    const albumPath = `${rootPath}/Album`;
    const album = directory(albumPath, [], [makeTrack(`${albumPath}/01.mp3`, "Requested track")]);
    let resolveInitialAlbum!: (value: DirectoryData) => void;
    const initialAlbumRead = new Promise<DirectoryData>((resolve) => { resolveInitialAlbum = resolve; });
    let albumReads = 0;
    const readDirectory = vi.fn(async (path: string) => {
      if (path === rootPath) return directory(rootPath, [{ name: "Album", path: albumPath, isDirectory: true }]);
      albumReads += 1;
      return albumReads === 1 ? initialAlbumRead : album;
    });
    const api = {
      listLibraryRoots: vi.fn().mockResolvedValue([{ id: "artist", name: "Artist", path: rootPath }]),
      readDirectory,
      listDirectory: vi.fn().mockResolvedValue([]),
      getCoverDataUrl: vi.fn().mockResolvedValue(null),
    } as unknown as Window["api"];
    render(<WorkspaceView api={api} openAlbumPath={albumPath} loadReleases={() => Promise.resolve({ releases: [] })} />);
    await waitFor(() => expect(readDirectory).toHaveBeenCalledWith(albumPath));
    await act(async () => { resolveInitialAlbum(album); });

    await screen.findByRole("heading", { name: "Album", exact: true });
    expect(within(screen.getByTestId("directory-tree"))
      .getByRole("button", { name: "Album", exact: true })).toBeTruthy();
  });

  it("returns the retained library selector to the first library on Monitoring drill-in", async () => {
    const firstRoot = "/music/First";
    const secondRoot = "/music/Second";
    const albumPath = `${firstRoot}/Requested Album`;
    const api = {
      listLibraryRoots: vi.fn().mockResolvedValue([
        { id: "first", name: "First", path: firstRoot },
        { id: "second", name: "Second", path: secondRoot },
      ]),
      readDirectory: vi.fn(async (path: string) => directory(path, [],
        path === albumPath ? [makeTrack(`${path}/01.mp3`, "Requested track")] : [],
      )),
      listDirectory: vi.fn().mockResolvedValue([]),
      getCoverDataUrl: vi.fn().mockResolvedValue(null),
    } as unknown as Window["api"];
    const props = { api, loadReleases: () => Promise.resolve({ releases: [] }) };
    const view = render(<WorkspaceView {...props} />);
    const picker = await screen.findByLabelText("Library");
    fireEvent.change(picker, { target: { value: "1" } });
    await screen.findByRole("heading", { name: "Second", exact: true });

    view.rerender(<WorkspaceView {...props} openAlbumPath={albumPath} />);

    await screen.findByText("Requested track");
    expect(picker).toHaveProperty("value", "0");
    expect(within(screen.getByRole("navigation", { name: "Current folder" }))
      .getByRole("button", { name: "First", exact: true })).toBeTruthy();
  });

  it.each(["success", "failure"] as const)("ignores a stale directory %s after a newer Monitoring request", async (outcome) => {
    const rootPath = "/music/Artist";
    const oldPath = `${rootPath}/Old Album`;
    const newPath = `${rootPath}/Requested Album`;
    let resolveOld!: (value: DirectoryData) => void;
    let rejectOld!: (reason: Error) => void;
    const oldRead = new Promise<DirectoryData>((resolve, reject) => {
      resolveOld = resolve;
      rejectOld = reject;
    });
    const readDirectory = vi.fn(async (path: string) => {
      if (path === oldPath) return oldRead;
      return directory(path, [], path === newPath
        ? [makeTrack(`${path}/01.mp3`, "Requested track")] : []);
    });
    const api = {
      listLibraryRoots: vi.fn().mockResolvedValue([{ id: "artist", name: "Artist", path: rootPath }]),
      readDirectory,
      listDirectory: vi.fn().mockResolvedValue([]),
      getCoverDataUrl: vi.fn().mockResolvedValue(null),
    } as unknown as Window["api"];
    const props = { api, loadReleases: () => Promise.resolve({ releases: [] }) };
    const view = render(<WorkspaceView {...props} />);
    await screen.findByTestId("workspace");
    view.rerender(<WorkspaceView {...props} openAlbumPath={oldPath} />);
    await waitFor(() => expect(readDirectory).toHaveBeenCalledWith(oldPath));
    view.rerender(<WorkspaceView {...props} openAlbumPath={newPath} />);
    await screen.findByText("Requested track");

    await act(async () => {
      if (outcome === "success") resolveOld(directory(oldPath, [], [makeTrack(`${oldPath}/01.mp3`, "Old track")]));
      else rejectOld(new Error("Stale read failed"));
    });

    expect(screen.getByRole("heading", { name: "Requested Album", exact: true })).toBeTruthy();
    expect(screen.getByText("Requested track")).toBeTruthy();
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("keeps an aggregated root identified as a folder instead of naming one mixed album", async () => {
    const rootPath = "/music/Artist";
    const albumEntries = ["Album A", "Album B"].map((name) => ({
      name,
      path: `${rootPath}/${name}`,
      isDirectory: true,
    }));
    const root = directory(rootPath, albumEntries);
    const albumTracks = albumEntries.map((entry, index) =>
      makeTrack(`${entry.path}/0${index + 1}.mp3`, `Track ${index + 1}`),
    );
    albumTracks[0].album = "Album A";
    albumTracks[1].album = "Album B";
    const api = {
      listLibraryRoots: vi.fn().mockResolvedValue([{ id: "artist", name: "Artist", path: rootPath }]),
      readDirectory: vi.fn(async (path: string) => {
        if (path === rootPath) return root;
        const index = albumEntries.findIndex((entry) => entry.path === path);
        return directory(path, [], [albumTracks[index]]);
      }),
      listDirectory: vi.fn().mockResolvedValue([]),
      getCoverDataUrl: vi.fn().mockResolvedValue(null),
    } as unknown as Window["api"];

    render(<WorkspaceView api={api} loadReleases={() => Promise.resolve({ releases: [] })} />);

    const identity = await screen.findByTestId("workspace-identity");
    expect(identity.getAttribute("data-kind")).toBe("library");
    expect(identity.textContent).toContain("Artist");
    expect(identity.textContent).toContain("2 tracks");
    expect(identity.textContent).not.toContain("Album A");
    expect(identity.textContent).not.toContain("Album B");
    expect((screen.getByTestId("tools-search") as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getAllByTestId("workspace-row")).toHaveLength(2);
  });

  it("keeps track tools available for a non-root folder with its own tracks and artwork folder", async () => {
    const rootPath = "/music/Artist";
    const albumPath = `${rootPath}/Album`;
    const artworkPath = `${albumPath}/Artwork`;
    const root = directory(rootPath, [{ name: "Album", path: albumPath, isDirectory: true }]);
    const album = directory(
      albumPath,
      [{ name: "Artwork", path: artworkPath, isDirectory: true }],
      [makeTrack(`${albumPath}/01.mp3`, "Track")],
    );
    const api = {
      listLibraryRoots: vi.fn().mockResolvedValue([{ id: "artist", name: "Artist", path: rootPath }]),
      readDirectory: vi.fn(async (path: string) =>
        path === rootPath ? root : path === albumPath ? album : directory(artworkPath, []),
      ),
      listDirectory: vi.fn().mockResolvedValue([]),
      getCoverDataUrl: vi.fn().mockResolvedValue(null),
    } as unknown as Window["api"];

    render(<WorkspaceView api={api} loadReleases={() => Promise.resolve({ releases: [] })} />);
    const tree = await screen.findByTestId("directory-tree");
    fireEvent.click(within(tree).getByRole("button", { name: "Album" }));

    expect((await screen.findByTestId("workspace-identity")).getAttribute("data-kind")).toBe("folder");
    expect((screen.getByTestId("tools-search") as HTMLButtonElement).disabled).toBe(false);
    expect((screen.getByTestId("tools-number") as HTMLButtonElement).disabled).toBe(false);
    expect((screen.getByTestId("tools-convert") as HTMLButtonElement).disabled).toBe(false);
  });

  it("reopens an album when Monitoring requests it again after another folder", async () => {
    const rootPath = "/music/Artist";
    const albumPath = `${rootPath}/A Gift From A Virgo`;
    const otherPath = `${rootPath}/Other Album`;
    const root = directory(rootPath, [
      { name: "A Gift From A Virgo", path: albumPath, isDirectory: true },
      { name: "Other Album", path: otherPath, isDirectory: true },
    ]);
    const readDirectory = vi.fn(async (path: string) =>
      path === rootPath
        ? root
        : directory(path, [], [makeTrack(`${path}/01.mp3`, "Track")]),
    );
    const api = {
      listLibraryRoots: vi.fn().mockResolvedValue([{ id: "artist", name: "Artist", path: rootPath }]),
      readDirectory,
      listDirectory: vi.fn().mockResolvedValue([]),
      getCoverDataUrl: vi.fn().mockResolvedValue(null),
    } as unknown as Window["api"];
    const props = { api, loadReleases: () => Promise.resolve({ releases: [] }) };
    const view = render(<WorkspaceView {...props} openAlbumPath={albumPath} />);

    await waitFor(() => {
      expect(readDirectory.mock.calls.filter(([path]) => path === albumPath)).toHaveLength(2);
    });
    view.rerender(<WorkspaceView {...props} openAlbumPath={null} />);
    const tree = await screen.findByTestId("directory-tree");
    fireEvent.click(within(tree).getByRole("button", { name: "Other Album" }));
    await waitFor(() => {
      expect(readDirectory.mock.calls.filter(([path]) => path === otherPath)).toHaveLength(2);
    });

    view.rerender(<WorkspaceView {...props} openAlbumPath={albumPath} />);

    await waitFor(() => {
      expect(readDirectory.mock.calls.filter(([path]) => path === albumPath)).toHaveLength(3);
    });
  });

  it("collapses the folder browser when the viewport becomes mobile", async () => {
    const rootPath = "/music/Artist";
    const root = directory(rootPath, []);
    let matches = true;
    const listeners: EventListener[] = [];
    const mediaQuery = {
      get matches() { return matches; },
      media: "(min-width: 901px)",
      addEventListener: vi.fn((_type: string, listener: EventListenerOrEventListenerObject) => {
        listeners.push(listener as EventListener);
      }),
      removeEventListener: vi.fn(),
    } as unknown as MediaQueryList;
    const originalDescriptor = Object.getOwnPropertyDescriptor(window, "matchMedia");
    Object.defineProperty(window, "matchMedia", {
      configurable: true,
      value: vi.fn(() => mediaQuery),
    });
    const api = {
      listLibraryRoots: vi.fn().mockResolvedValue([{ id: "artist", name: "Artist", path: rootPath }]),
      readDirectory: vi.fn().mockResolvedValue(root),
      listDirectory: vi.fn().mockResolvedValue([]),
      getCoverDataUrl: vi.fn().mockResolvedValue(null),
    } as unknown as Window["api"];

    try {
      render(<WorkspaceView api={api} loadReleases={() => Promise.resolve({ releases: [] })} />);
      const disclosure = await screen.findByTestId("folder-disclosure") as HTMLDetailsElement;
      expect(disclosure.open).toBe(true);

      matches = false;
      act(() => {
        for (const listener of listeners) listener(new Event("change"));
      });

      expect(disclosure.open).toBe(false);
    } finally {
      if (originalDescriptor) Object.defineProperty(window, "matchMedia", originalDescriptor);
      else Reflect.deleteProperty(window, "matchMedia");
    }
  });

  it("shows loaded album details and keeps every selected-track field in the bottom editor", async () => {
    const rootPath = "/music/Artist";
    const albumPath = `${rootPath}/4 (Deluxe Edition)`;
    const albumEntry = { name: "4 (Deluxe Edition)", path: albumPath, isDirectory: true };
    const root = directory(rootPath, [albumEntry]);
    const albumTrack = makeTrack(`${albumPath}/01.mp3`, "Track title");
    albumTrack.album = "4";
    albumTrack.year = "2024";
    albumTrack.trackNumber = 17;
    const coverDataUrl = "data:image/png;base64,loaded-cover";
    const api = {
      listLibraryRoots: vi.fn().mockResolvedValue([{ id: "artist", name: "Artist", path: rootPath }]),
      readDirectory: vi.fn(async (path: string) =>
        path === rootPath ? root : directory(albumPath, [], [albumTrack]),
      ),
      listDirectory: vi.fn().mockResolvedValue([]),
      getCoverDataUrl: vi.fn().mockResolvedValue(coverDataUrl),
    } as unknown as Window["api"];

    render(<WorkspaceView api={api} loadReleases={() => Promise.resolve({ releases: [] })} />);

    const tree = await screen.findByTestId("directory-tree");
    const rootFolder = within(tree).getByRole("button", { name: "Artist" });
    if (rootFolder.getAttribute("aria-expanded") !== "true") fireEvent.click(rootFolder);
    fireEvent.click(await within(tree).findByRole("button", { name: "4 (Deluxe Edition)" }));
    fireEvent.click(await screen.findByRole("button", { name: "01.mp3" }));

    const identity = await screen.findByTestId("workspace-identity");
    expect(identity.getAttribute("data-kind")).toBe("album");
    expect(identity.textContent).toContain("4 (Deluxe Edition)");
    expect(identity.textContent).toContain("Artist");
    expect(identity.textContent).toContain("2024");
    expect(identity.textContent).toContain("1 track");
    expect(screen.getByRole("heading", { name: "Edit track 1 of 1" })).toBeTruthy();
    // Filtering must not turn a display position back into an invalid stored tag number.
    fireEvent.change(screen.getByLabelText("Filter tracks"), { target: { value: "no matching track" } });
    expect(screen.getByRole("heading", { name: "Edit track 1 of 1" })).toBeTruthy();
    fireEvent.change(screen.getByLabelText("Filter tracks"), { target: { value: "" } });
    expect(identity.querySelector("img")?.getAttribute("src")).toBe(coverDataUrl);
    expect(screen.getByTestId("workspace-row").textContent).toContain("4");

    const table = screen.getByTestId("workspace-table");
    const editor = screen.getByTestId("track-editor");
    expect(table.compareDocumentPosition(editor) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    fireEvent.click(screen.getByText("More fields"));
    expect(screen.getByLabelText("Disc Number")).toBeTruthy();
    expect(screen.getByLabelText("Track No.")).toBeTruthy();
    expect(screen.getByLabelText("Total Tracks")).toBeTruthy();
    expect(screen.getByLabelText("Total Discs")).toBeTruthy();
    expect(screen.getByLabelText("Lyrics")).toBeTruthy();
    expect(screen.getByLabelText("Genres")).toBeTruthy();
    expect(screen.getByLabelText("Composer")).toBeTruthy();
    expect(screen.getByLabelText("Summary")).toBeTruthy();
    expect(screen.getByLabelText("Description")).toBeTruthy();
    expect(screen.getByTestId("editor-technical")).toBeTruthy();
  });

  it("dismisses only the current editor and reopens it on a new track selection", async () => {
    const detailsKey = "soundrobe.web.workspace.details";
    const storage = new Map<string, string>();
    const localStorage = {
      getItem: (key: string) => storage.get(key) ?? null,
      setItem: (key: string, value: string) => storage.set(key, value),
      removeItem: (key: string) => storage.delete(key),
    };
    const localStorageDescriptor = Object.getOwnPropertyDescriptor(window, "localStorage");
    Object.defineProperty(window, "localStorage", { configurable: true, value: localStorage });
    localStorage.setItem(detailsKey, "true");
    const rootPath = "/music/Artist";
    const albumPath = `${rootPath}/Album`;
    const root = directory(rootPath, [{ name: "Album", path: albumPath, isDirectory: true }]);
    const tracks = [
      makeTrack(`${albumPath}/01.mp3`, "First track"),
      makeTrack(`${albumPath}/02.mp3`, "Second track"),
    ];
    const api = {
      listLibraryRoots: vi.fn().mockResolvedValue([{ id: "artist", name: "Artist", path: rootPath }]),
      readDirectory: vi.fn(async (path: string) =>
        path === rootPath ? root : directory(albumPath, [], tracks),
      ),
      listDirectory: vi.fn().mockResolvedValue([]),
      getCoverDataUrl: vi.fn().mockResolvedValue(null),
    } as unknown as Window["api"];

    try {
      render(<WorkspaceView api={api} loadReleases={() => Promise.resolve({ releases: [] })} />);
      const tree = await screen.findByTestId("directory-tree");
      fireEvent.click(within(tree).getByRole("button", { name: "Album" }));
      fireEvent.click(await screen.findByRole("button", { name: "01.mp3" }));
      expect(screen.getByTestId("track-editor")).toBeTruthy();

      fireEvent.click(screen.getByLabelText("Close track editor"));
      expect(screen.queryByTestId("track-editor")).toBeNull();
      expect(
        screen.getAllByTestId("workspace-row").some((row) => row.getAttribute("aria-selected") === "true"),
      ).toBe(true);
      expect(localStorage.getItem(detailsKey)).toBe("true");

      fireEvent.click(screen.getByRole("button", { name: "02.mp3" }));
      expect(await screen.findByTestId("track-editor")).toBeTruthy();
    } finally {
      if (localStorageDescriptor) Object.defineProperty(window, "localStorage", localStorageDescriptor);
      else Reflect.deleteProperty(window, "localStorage");
    }
  });

  it("keeps tracks visible while sixty review albums stay in a closed queue", async () => {
    const rootPath = "/music/Artist";
    const entries = Array.from({ length: 60 }, (_, index) => ({
      name: `Album ${String(index + 1).padStart(2, "0")}`,
      path: `${rootPath}/Album ${String(index + 1).padStart(2, "0")}`,
      isDirectory: true,
    }));
    const root = directory(rootPath, entries);
    const api = {
      listLibraryRoots: vi.fn().mockResolvedValue([{ id: "artist", name: "Artist", path: rootPath }]),
      readDirectory: vi.fn(async (path: string) => {
        if (path === rootPath) return root;
        const name = path.split("/").pop() ?? "Album";
        return directory(path, [], [makeTrack(`${path}/01.mp3`, `Track ${name}`)]);
      }),
      listDirectory: vi.fn().mockResolvedValue([]),
      getCoverDataUrl: vi.fn().mockResolvedValue(null),
    } as unknown as Window["api"];
    const releases = entries.map(({ path }) => ({ path, status: "needs_review" as const }));

    render(<WorkspaceView api={api} loadReleases={() => Promise.resolve({ releases })} />);

    const queue = await screen.findByTestId("review-queue");
    expect((queue as HTMLDetailsElement).open).toBe(false);
    expect(screen.getAllByTestId("workspace-row")).toHaveLength(60);
    expect(screen.getByTestId("review-queue-summary").textContent).toContain("60 albums need review");
    expect(queue.querySelectorAll('[data-testid^="review-acknowledge-"]')).toHaveLength(60);
  });

  it("retries the selected library when its initial load fails", async () => {
    const rootPath = "/music/Artist";
    const root = directory(rootPath, [], [makeTrack(`${rootPath}/01.mp3`, "Recovered")]);
    const readDirectory = vi
      .fn()
      .mockRejectedValueOnce(new Error("temporary read failure"))
      .mockResolvedValue(root);
    const api = {
      listLibraryRoots: vi.fn().mockResolvedValue([{ id: "artist", name: "Artist", path: rootPath }]),
      readDirectory,
      listDirectory: vi.fn().mockResolvedValue([]),
      getCoverDataUrl: vi.fn().mockResolvedValue(null),
    } as unknown as Window["api"];

    render(
      <WorkspaceView
        api={api}
        loadReleases={() => Promise.resolve({ releases: [] })}
      />,
    );

    expect((await screen.findByRole("alert")).textContent).toContain("temporary read failure");
    fireEvent.click(screen.getByRole("button", { name: "Refresh" }));

    expect(await screen.findByText("Recovered")).toBeTruthy();
    expect(readDirectory).toHaveBeenCalledTimes(2);
    expect(readDirectory).toHaveBeenLastCalledWith(rootPath);
  });

  it("shows every loaded album track for an artist root when no album is selected", async () => {
    const rootPath = "/music/Artist";
    const groupingPath = `${rootPath}/keep`;
    const albumAPath = `${groupingPath}/Album A`;
    const albumBPath = `${groupingPath}/Album B`;
    const albumSubdirs = [
      { name: "Album A", path: albumAPath, isDirectory: true },
      { name: "Album B", path: albumBPath, isDirectory: true },
    ];
    const root = directory(rootPath, [{ name: "keep", path: groupingPath, isDirectory: true }]);
    const grouping = directory(groupingPath, albumSubdirs);
    const albumA = directory(albumAPath, [], [makeTrack(`${albumAPath}/01.mp3`, "First")]);
    const albumB = directory(albumBPath, [], [makeTrack(`${albumBPath}/01.mp3`, "Second")]);
    const readDirectory = vi.fn(async (path: string) => {
      if (path === rootPath) return root;
      if (path === groupingPath) return grouping;
      if (path === albumAPath) return albumA;
      if (path === albumBPath) return albumB;
      throw new Error(`Unexpected directory: ${path}`);
    });
    const api = {
      listLibraryRoots: vi.fn().mockResolvedValue([{ id: "artist", name: "Artist", path: rootPath }]),
      readDirectory,
      listDirectory: vi.fn().mockResolvedValue([]),
      getCoverDataUrl: vi.fn().mockResolvedValue(null),
    } as unknown as Window["api"];

    render(
      <WorkspaceView
        api={api}
        loadReleases={() => Promise.resolve({ releases: [] })}
      />,
    );

    await waitFor(() => expect(screen.getAllByTestId("workspace-row")).toHaveLength(2));
    expect(screen.getByText("First")).toBeTruthy();
    expect(screen.getByText("Second")).toBeTruthy();
    expect(readDirectory).toHaveBeenCalledWith(groupingPath);
    expect(readDirectory).toHaveBeenCalledWith(albumAPath);
    expect(readDirectory).toHaveBeenCalledWith(albumBPath);
  });
});
