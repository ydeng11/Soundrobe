// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import React from "react";
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
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
