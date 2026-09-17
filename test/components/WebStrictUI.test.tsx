// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from "vitest";
import React from "react";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { WebServiceShell } from "../../src/components/WebServiceShell";
import { FileGrid } from "../../src/components/FileGrid";
import type { TrackData } from "../../src/shared/desktop-api";

afterEach(cleanup);

function track(path: string): TrackData {
  return {
    path,
    title: "Song",
    artist: "Artist",
    artists: [],
    album: "Album",
    albumArtist: null,
    albumArtists: [],
    trackNumber: 1,
    trackTotal: 1,
    discNumber: null,
    discTotal: null,
    year: "2024",
    genre: "Pop",
    composer: null,
    comment: null,
    lyrics: null,
    compilation: null,
    musicbrainzTrackId: null,
    musicbrainzAlbumId: null,
    musicbrainzArtistId: null,
    hasCover: false,
    sizeBytes: 1,
    bitrate: 128000,
    sampleRate: 44100,
    codec: "mp3",
    duration: 60,
  };
}

describe("strict browser service UI", () => {
  it("exposes the five Soundrobe views and reports the selected view", () => {
    const onViewChange = vi.fn();
    render(
      <WebServiceShell activeView="home" onViewChange={onViewChange}>
        <p>Home content</p>
      </WebServiceShell>,
    );

    expect(screen.getByRole("button", { name: "Home" }).getAttribute("aria-current")).toBe("page");
    expect(screen.getByRole("button", { name: "Workspace" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Library" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Activity" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Settings" })).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Workspace" }));
    expect(onViewChange).toHaveBeenCalledWith("workspace");
  });

  it("owns the browser search field and disables it when no root is loaded", () => {
    const onSearchChange = vi.fn();
    const { rerender } = render(
      <WebServiceShell activeView="home" searchText="" onSearchChange={onSearchChange}>
        <p>Home content</p>
      </WebServiceShell>,
    );

    const search = screen.getByRole("searchbox", { name: "Filter workspace" });
    expect((search as HTMLInputElement).disabled).toBe(true);

    rerender(
      <WebServiceShell
        activeView="workspace"
        searchText=""
        searchEnabled
        onSearchChange={onSearchChange}
      >
        <p>Workspace content</p>
      </WebServiceShell>,
    );
    fireEvent.change(screen.getByRole("searchbox", { name: "Filter workspace" }), {
      target: { value: "jazz" },
    });
    expect(onSearchChange).toHaveBeenCalledWith("jazz");
  });

  it("does not let a hidden web grid steal Select All from Home or Settings", () => {
    const onMultiSelect = vi.fn();
    render(
      <>
        <input aria-label="Home input" />
        <FileGrid
          web
          webActive={false}
          tracks={[track("/music/one.mp3")]}
          selectedTrackPath={null}
          filterText=""
          onSelectTrack={vi.fn()}
          onMultiSelect={onMultiSelect}
        />
      </>,
    );

    fireEvent.keyDown(screen.getByLabelText("Home input"), { key: "a", metaKey: true });
    expect(onMultiSelect).not.toHaveBeenCalled();

    (screen.getByTestId("file-grid-body") as HTMLDivElement).focus();
    fireEvent.keyDown(screen.getByTestId("file-grid-body"), { key: "a", metaKey: true });
    expect(onMultiSelect).not.toHaveBeenCalled();
  });

  it("selects all only when the active web workspace owns focus", () => {
    const onMultiSelect = vi.fn();
    render(
      <FileGrid
        web
        webActive
        tracks={[track("/music/one.mp3")]}
        selectedTrackPath={null}
        filterText=""
        onSelectTrack={vi.fn()}
        onMultiSelect={onMultiSelect}
      />,
    );

    (screen.getByTestId("file-grid-body") as HTMLDivElement).focus();
    fireEvent.keyDown(screen.getByTestId("file-grid-body"), { key: "a", metaKey: true });
    expect(onMultiSelect).toHaveBeenCalledWith(["/music/one.mp3"]);
  });
});
