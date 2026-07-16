// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from "vitest";
import React from "react";
import { render, screen, fireEvent, cleanup } from "@testing-library/react";
import { BatchEditor } from "../../src/components/BatchEditor";
import type { TrackData } from "../../src/shared/desktop-api";

afterEach(() => cleanup());

function makeTrack(path: string, overrides?: Partial<TrackData>): TrackData {
  return {
    path,
    title: "Song",
    artist: "Artist",
    artists: [],
    album: "Album",
    albumArtist: null,
    albumArtists: [],
    trackNumber: 1,
    trackTotal: 10,
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
    sizeBytes: 123,
    bitrate: null,
    sampleRate: null,
    codec: "MP3",
    duration: 60,
    ...overrides,
  };
}

describe("BatchEditor", () => {
  it("renders standard batch fields", () => {
    const tracks = [makeTrack("/music/a.mp3"), makeTrack("/music/b.mp3")];
    render(
      <BatchEditor
        tracks={tracks}
        coverDataUrl={null}
        saving={false}
        onSave={vi.fn()}
      />,
    );

    expect(screen.getByText("Batch Edit")).toBeTruthy();
    expect(screen.getByText("2 files selected")).toBeTruthy();
    expect(screen.getByText("Artist")).toBeTruthy();
    expect(screen.getByText("Album")).toBeTruthy();
    expect(screen.getByText("Genre")).toBeTruthy();
    expect(screen.getByText("Year")).toBeTruthy();
  });

  it("does not render an Extra Tags button in the Batch Edit panel", () => {
    const tracks = [makeTrack("/music/a.mp3"), makeTrack("/music/b.mp3")];
    render(
      <BatchEditor
        tracks={tracks}
        coverDataUrl={null}
        saving={false}
        onSave={vi.fn()}
      />,
    );

    expect(screen.queryByText("Extra Tags")).toBeNull();
  });

  it("shows saving indicator when saving is true", () => {
    const tracks = [makeTrack("/music/a.mp3"), makeTrack("/music/b.mp3")];
    render(
      <BatchEditor
        tracks={tracks}
        coverDataUrl={null}
        saving={true}
        onSave={vi.fn()}
      />,
    );

    expect(screen.getByText("Saving")).toBeTruthy();
  });

  it("shows unsaved indicator when a field is edited", () => {
    const tracks = [makeTrack("/music/a.mp3"), makeTrack("/music/b.mp3")];
    render(
      <BatchEditor
        tracks={tracks}
        coverDataUrl={null}
        saving={false}
        onSave={vi.fn()}
      />,
    );

    const artistInput = screen.getByPlaceholderText("Common artist…");
    fireEvent.change(artistInput, { target: { value: "New Artist" } });

    expect(screen.getByText(/Unsaved/i)).toBeTruthy();
  });

  it("calls onSave with filled fields when focus leaves the panel", () => {
    const tracks = [makeTrack("/music/a.mp3"), makeTrack("/music/b.mp3")];
    const onSave = vi.fn();
    render(
      <BatchEditor
        tracks={tracks}
        coverDataUrl={null}
        saving={false}
        onSave={onSave}
      />,
    );

    const artistInput = screen.getByPlaceholderText("Common artist…");
    const albumInput = screen.getByPlaceholderText("Common album…");
    fireEvent.change(artistInput, { target: { value: "New Artist" } });
    fireEvent.change(albumInput, { target: { value: "New Album" } });

    // Simulate focus leaving the panel
    const container = artistInput.closest('[class*="flex flex-col h-full overflow-y-auto"]');
    expect(container).toBeTruthy();
    fireEvent.blur(container!, { relatedTarget: null });

    expect(onSave).toHaveBeenCalledWith({
      artist: "New Artist",
      album: "New Album",
    });
  });

  it("does not call onSave when focus moves between fields within the panel", () => {
    const tracks = [makeTrack("/music/a.mp3"), makeTrack("/music/b.mp3")];
    const onSave = vi.fn();
    render(
      <BatchEditor
        tracks={tracks}
        coverDataUrl={null}
        saving={false}
        onSave={onSave}
      />,
    );

    const artistInput = screen.getByPlaceholderText("Common artist…");
    const albumInput = screen.getByPlaceholderText("Common album…");

    fireEvent.change(artistInput, { target: { value: "New Artist" } });

    // Moving focus from artist to album (both inside the panel)
    fireEvent.blur(artistInput, { relatedTarget: albumInput });

    expect(onSave).not.toHaveBeenCalled();
  });

  it("does not call onSave when all fields are empty on blur", () => {
    const tracks = [makeTrack("/music/a.mp3"), makeTrack("/music/b.mp3")];
    const onSave = vi.fn();
    render(
      <BatchEditor
        tracks={tracks}
        coverDataUrl={null}
        saving={false}
        onSave={onSave}
      />,
    );

    const container = screen.getByText("Batch Edit").closest('[class*="flex flex-col h-full overflow-y-auto"]');
    expect(container).toBeTruthy();
    fireEvent.blur(container!, { relatedTarget: null });

    expect(onSave).not.toHaveBeenCalled();
  });

  it("shows mixed indicator when tracks have differing values for a field", () => {
    const tracks = [
      makeTrack("/music/a.mp3", { artist: "Artist A" }),
      makeTrack("/music/b.mp3", { artist: "Artist B" }),
    ];
    render(
      <BatchEditor
        tracks={tracks}
        coverDataUrl={null}
        saving={false}
        onSave={vi.fn()}
      />,
    );

    const mixedBadges = screen.getAllByText("mixed");
    expect(mixedBadges.length).toBeGreaterThanOrEqual(1);
  });

  it("displays cover art when coverDataUrl is provided", () => {
    const tracks = [makeTrack("/music/a.mp3"), makeTrack("/music/b.mp3")];
    render(
      <BatchEditor
        tracks={tracks}
        coverDataUrl="data:image/jpeg;base64,abc123"
        saving={false}
        onSave={vi.fn()}
      />,
    );

    const img = screen.getByAltText("Cover art");
    expect(img).toBeTruthy();
    expect(img.getAttribute("src")).toBe("data:image/jpeg;base64,abc123");
  });

  it("does not render Apply button", () => {
    const tracks = [makeTrack("/music/a.mp3"), makeTrack("/music/b.mp3")];
    render(
      <BatchEditor
        tracks={tracks}
        coverDataUrl={null}
        saving={false}
        onSave={vi.fn()}
      />,
    );

    expect(screen.queryByText(/Apply/i)).toBeNull();
  });

  it("clears unsaved indicator after blur triggers save", () => {
    const tracks = [makeTrack("/music/a.mp3"), makeTrack("/music/b.mp3")];
    const onSave = vi.fn();
    render(
      <BatchEditor
        tracks={tracks}
        coverDataUrl={null}
        saving={false}
        onSave={onSave}
      />,
    );

    const artistInput = screen.getByPlaceholderText("Common artist…");
    fireEvent.change(artistInput, { target: { value: "New Artist" } });
    expect(screen.getByText(/Unsaved/i)).toBeTruthy();

    // Blur the panel
    const container = artistInput.closest('[class*="flex flex-col h-full overflow-y-auto"]');
    expect(container).toBeTruthy();
    fireEvent.blur(container!, { relatedTarget: null });

    // Indicator should be gone
    expect(screen.queryByText(/Unsaved/i)).toBeFalsy();
  });
});
