// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import React from "react";
import { cleanup, fireEvent, render, screen, within } from "@testing-library/react";
import { WebApp } from "../../src/web/WebApp";
import { WebLibraryPicker } from "../../src/components/WebLibraryPicker";
import type { DirectoryData, TrackData } from "../../src/shared/desktop-api";

afterEach(cleanup);

function track(path: string, title: string): TrackData {
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
  return { path, name: path.split("/").pop() ?? "", subdirs, tracks, audioCount: tracks.length };
}

describe("WebApp", () => {
  it("opens the browser workspace without a login request or sign-in screen", async () => {
    const fetchMock = vi.spyOn(globalThis, "fetch").mockResolvedValue(
      new Response(JSON.stringify({ releases: [] }), { status: 200 }),
    );
    const api = {
      listLibraryRoots: vi.fn().mockResolvedValue([]),
    } as unknown as Window["api"];

    render(<WebApp api={api} />);

    expect(await screen.findByTestId("workspace-no-root")).toBeTruthy();
    expect(screen.queryByText(/Sign in to Soundrobe/)).toBeNull();
    expect(screen.queryByRole("button", { name: "Sign out" })).toBeNull();
    expect(fetchMock.mock.calls.some(([url]) => String(url).includes("/auth/"))).toBe(false);
    fetchMock.mockRestore();
  });

  it("keeps the selected folder, filter, and track when the reader visits another page", async () => {
    const rootPath = "/music/Artist";
    const albumPath = `${rootPath}/Album`;
    const albumTrack = track(`${albumPath}/01 - Needle.mp3`, "Needle");
    const root = directory(rootPath, [{ name: "Album", path: albumPath, isDirectory: true }]);
    const album = directory(albumPath, [], [albumTrack]);
    const readDirectory = vi.fn(async (path: string) => path === rootPath ? root : album);
    const api = {
      listLibraryRoots: vi.fn().mockResolvedValue([{ id: "artist", name: "Artist", path: rootPath }]),
      readDirectory,
      listDirectory: vi.fn().mockResolvedValue([]),
      getCoverDataUrl: vi.fn().mockResolvedValue(null),
      getConfig: vi.fn().mockResolvedValue({}),
    } as unknown as Window["api"];

    render(<WebApp api={api} />);
    await screen.findByTestId("workspace");

    const tree = screen.getByTestId("directory-tree");
    const rootFolder = within(tree).getByRole("button", { name: "Artist" });
    if (rootFolder.getAttribute("aria-expanded") !== "true") fireEvent.click(rootFolder);
    fireEvent.click(await within(tree).findByRole("button", { name: "Album" }));
    fireEvent.click(await screen.findByRole("button", { name: "01 - Needle.mp3" }));
    fireEvent.change(screen.getByLabelText("Filter tracks"), { target: { value: "Needle" } });
    const loadsBeforeNavigation = readDirectory.mock.calls.length;

    fireEvent.click(screen.getByRole("button", { name: "Tag sources" }));
    await screen.findByTestId("tag-sources");
    const workspaceHost = screen.getByTestId("workspace-host");
    expect(workspaceHost.hasAttribute("hidden")).toBe(true);
    expect(workspaceHost.hasAttribute("inert")).toBe(true);

    fireEvent.click(screen.getByRole("button", { name: "Workspace" }));

    expect(await screen.findByTestId("workspace-row")).toBeTruthy();
    expect(screen.getByLabelText("Filter tracks")).toHaveProperty("value", "Needle");
    expect(screen.getByTestId("workspace-row").getAttribute("aria-selected")).toBe("true");
    expect(readDirectory).toHaveBeenCalledTimes(loadsBeforeNavigation);
  });
});

describe("WebLibraryPicker", () => {
  it("lists mounted roots and selects the requested root", () => {
    const onSelect = vi.fn();
    render(
      <WebLibraryPicker
        roots={[
          { id: "jazz", name: "jazz", path: "/libraries/jazz" },
          { id: "classical", name: "classical", path: "/libraries/classical" },
        ]}
        onSelect={onSelect}
      />,
    );

    expect(screen.getByText("Choose a library")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: /jazz/ }));
    expect(onSelect).toHaveBeenCalledWith("/libraries/jazz");
  });

  it("shows a distinct loading state without offering stale selections", () => {
    render(
      <WebLibraryPicker
        roots={[{ id: "jazz", name: "jazz", path: "/libraries/jazz" }]}
        loading
        onSelect={vi.fn()}
      />,
    );

    expect(screen.getByRole("status").textContent).toContain("Loading libraries…");
    expect(screen.queryByRole("button", { name: /jazz/ })).toBeNull();
  });

  it("shows a root-loading error and does not expose selections", () => {
    render(
      <WebLibraryPicker
        roots={[{ id: "jazz", name: "jazz", path: "/libraries/jazz" }]}
        error="The library service is unavailable"
        onSelect={vi.fn()}
      />,
    );

    expect(screen.getByRole("alert").textContent).toContain("The library service is unavailable");
    expect(screen.queryByRole("button", { name: /jazz/ })).toBeNull();
  });

  it("explains when the service has no mounted roots", () => {
    render(<WebLibraryPicker roots={[]} onSelect={vi.fn()} />);

    expect(screen.getByText(/No libraries are mounted\./)).toBeTruthy();
  });
});
