// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from "vitest";
import React from "react";
import { render, screen, fireEvent, cleanup } from "@testing-library/react";
import { BatchEditor } from "../../src/components/BatchEditor";
import type { TrackData } from "../../electron/preload";

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

  it("disables Apply button when all fields are empty", () => {
    const tracks = [makeTrack("/music/a.mp3"), makeTrack("/music/b.mp3")];
    render(
      <BatchEditor
        tracks={tracks}
        coverDataUrl={null}
        saving={false}
        onSave={vi.fn()}
      />,
    );

    const applyButton = screen.getByText("Apply to 2 files") as HTMLButtonElement;
    expect(applyButton.disabled).toBe(true);
  });

  it("enables Apply button when a field has a value", () => {
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

    const applyButton = screen.getByText("Apply to 2 files") as HTMLButtonElement;
    expect(applyButton.disabled).toBe(false);
  });

  it("calls onSave with the filled fields when Apply is clicked", () => {
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

    fireEvent.click(screen.getByText("Apply to 2 files"));
    expect(onSave).toHaveBeenCalledWith({
      artist: "New Artist",
      album: "New Album",
    });
  });

  it("clears fields after applying", () => {
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

    const artistInput = screen.getByPlaceholderText("Common artist…") as HTMLInputElement;
    fireEvent.change(artistInput, { target: { value: "New Artist" } });
    fireEvent.click(screen.getByText("Apply to 2 files"));

    expect(artistInput.value).toBe("");
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
});
