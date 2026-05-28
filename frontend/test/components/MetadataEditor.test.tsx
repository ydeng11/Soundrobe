// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from "vitest";
import React from "react";
import { render, screen, fireEvent, cleanup } from "@testing-library/react";

afterEach(() => cleanup());
import { MetadataEditor } from "../../src/components/MetadataEditor";
import type { TrackData } from "../../electron/preload";

function makeTrack(overrides?: Partial<TrackData>): TrackData {
  return {
    path: "/music/song.mp3",
    title: "Test Title",
    artist: "Test Artist",
    artists: [],
    album: "Test Album",
    albumArtist: "Test Album Artist",
    albumArtists: [],
    trackNumber: 1,
    trackTotal: 10,
    discNumber: 1,
    discTotal: 1,
    year: "2023",
    genre: "Rock",
    composer: "Test Composer",
    lyrics: null,
    compilation: null,
    musicbrainzTrackId: "mb-track-123",
    musicbrainzAlbumId: "mb-album-456",
    musicbrainzArtistId: "mb-artist-789",
    hasCover: false,
    sizeBytes: 5000000,
    bitrate: 256000,
    sampleRate: 44100,
    codec: "MP3",
    duration: 240,
    ...overrides,
  };
}

describe("MetadataEditor", () => {
  const baseProps = {
    track: makeTrack(),
    dirPath: "/music",
    coverDataUrl: null,
    saving: false,
    onSave: vi.fn(),
    onCancel: vi.fn(),
    onChangeCover: vi.fn(),
    onRemoveCover: vi.fn(),
  };

  it("displays the filename", () => {
    render(<MetadataEditor {...baseProps} />);
    expect(screen.getByText("song.mp3")).toBeTruthy();
  });

  it("displays tag fields with values", () => {
    render(<MetadataEditor {...baseProps} />);
    expect(screen.getByDisplayValue("Test Title")).toBeTruthy();
    expect(screen.getByDisplayValue("Test Artist")).toBeTruthy();
    expect(screen.getByDisplayValue("Test Album")).toBeTruthy();
    expect(screen.getByDisplayValue("2023")).toBeTruthy();
    expect(screen.getByDisplayValue("1/10")).toBeTruthy();
    expect(screen.getByDisplayValue("Rock")).toBeTruthy();
    expect(screen.getByDisplayValue("Test Composer")).toBeTruthy();
  });

  it("does not render ARTISTS in the metadata panel", () => {
    render(<MetadataEditor {...baseProps} track={makeTrack({ artists: ["foo", "bar"] })} />);

    expect(screen.queryByText("ARTISTS")).toBeNull();
    expect(screen.queryByDisplayValue("foo, bar")).toBeNull();
  });

  it("shows unsaved indicator when a field is edited", () => {
    render(<MetadataEditor {...baseProps} />);
    const titleInput = screen.getByDisplayValue("Test Title");
    fireEvent.change(titleInput, { target: { value: "New Title" } });
    expect(screen.getByText(/Unsaved/i)).toBeTruthy();
  });

  it("calls onSave with changed fields when Save Changes is clicked", () => {
    const onSave = vi.fn();
    render(
      <MetadataEditor {...baseProps} onSave={onSave} />
    );

    const titleInput = screen.getByDisplayValue("Test Title");
    fireEvent.change(titleInput, { target: { value: "New Title" } });

    const artistInput = screen.getByDisplayValue("Test Artist");
    fireEvent.change(artistInput, { target: { value: "New Artist" } });

    fireEvent.click(screen.getByText(/Save Changes/));
    expect(onSave).toHaveBeenCalledTimes(1);
    expect(onSave).toHaveBeenCalledWith({
      title: "New Title",
      artist: "New Artist",
    });
  });

  it("calls onSave when year is edited and saved", () => {
    const onSave = vi.fn();
    render(
      <MetadataEditor {...baseProps} onSave={onSave} />
    );

    const yearInput = screen.getByDisplayValue("2023");
    fireEvent.change(yearInput, { target: { value: "1999" } });
    fireEvent.click(screen.getByText(/Save Changes/));
    expect(onSave).toHaveBeenCalledWith({ year: "1999" });
  });

  it("shows saving indicator when saving is true", () => {
    render(<MetadataEditor {...baseProps} saving={true} />);
    // The header shows "Saving"; use getAllByText since the Save button
    // also shows "Saving…" when disabled
    const savingIndicators = screen.getAllByText(/Saving/i);
    expect(savingIndicators.length).toBeGreaterThanOrEqual(1);
  });

  it("does not show saving indicator when saving is false", () => {
    render(<MetadataEditor {...baseProps} saving={false} />);
    expect(screen.queryByText(/Saving/i)).toBeFalsy();
  });

  it("shows cover image when coverDataUrl is provided", () => {
    render(
      <MetadataEditor
        {...baseProps}
        coverDataUrl="data:image/jpeg;base64,abc123"
      />
    );
    const img = screen.getByAltText("Cover art");
    expect(img).toBeTruthy();
    expect(img.getAttribute("src")).toBe("data:image/jpeg;base64,abc123");
  });

  it('shows "No cover" when coverDataUrl is null', () => {
    render(<MetadataEditor {...baseProps} coverDataUrl={null} />);
    expect(screen.getByText(/No cover/)).toBeTruthy();
  });

  it("calls onChangeCover when Change button is clicked", () => {
    const onChangeCover = vi.fn();
    render(
      <MetadataEditor {...baseProps} onChangeCover={onChangeCover} />
    );
    // The "Change" cover button; avoid matching "Save Changes"
    const changeButton = screen
      .getAllByRole("button")
      .find((btn) => btn.textContent === "Change");
    expect(changeButton).toBeTruthy();
    if (changeButton) fireEvent.click(changeButton);
    expect(onChangeCover).toHaveBeenCalledOnce();
  });

  it("calls onRemoveCover when Remove button is clicked", () => {
    const onRemoveCover = vi.fn();
    render(
      <MetadataEditor
        {...baseProps}
        coverDataUrl="data:image/jpeg;base64,abc123"
        onRemoveCover={onRemoveCover}
      />
    );
    fireEvent.click(screen.getByText(/Remove/));
    expect(onRemoveCover).toHaveBeenCalledOnce();
  });

  it("displays format details", () => {
    render(<MetadataEditor {...baseProps} />);
    expect(screen.getByText("Codec")).toBeTruthy();
    expect(screen.getByText("Sample Rate")).toBeTruthy();
    expect(screen.getByText("Bitrate")).toBeTruthy();
    expect(screen.getByText("Size")).toBeTruthy();
    expect(screen.getByText("44 kHz")).toBeTruthy();
    expect(screen.getByText("256 kbps")).toBeTruthy();
    expect(screen.getByText("4.8 MB")).toBeTruthy();
  });

  it("displays detailed tags section with MusicBrainz IDs", () => {
    render(<MetadataEditor {...baseProps} />);
    const detailedSections = screen.getAllByText(/MusicBrainz Track ID/);
    expect(detailedSections.length).toBeGreaterThanOrEqual(1);
    expect(detailedSections[0].textContent).toContain("mb-track-123");
    expect(detailedSections[0].textContent).toContain("mb-album-456");
    expect(detailedSections[0].textContent).toContain("mb-artist-789");
  });

  it("handles empty fields gracefully", () => {
    const emptyTrack = makeTrack({
      title: null,
      artist: null,
      album: null,
      year: null,
      trackNumber: null,
      trackTotal: null,
      genre: null,
      composer: null,
    });
    render(<MetadataEditor {...baseProps} track={emptyTrack} />);
    const emptyInputs = screen.getAllByDisplayValue("");
    expect(emptyInputs.length).toBeGreaterThanOrEqual(6);
  });

  it("calls onSave when composer (textarea) is edited and saved", () => {
    const onSave = vi.fn();
    render(
      <MetadataEditor {...baseProps} onSave={onSave} />
    );
    const composerInputs = screen.getAllByDisplayValue("Test Composer");
    expect(composerInputs.length).toBeGreaterThanOrEqual(1);
    fireEvent.change(composerInputs[0], { target: { value: "New Composer" } });
    fireEvent.click(screen.getByText(/Save Changes/));
    expect(onSave).toHaveBeenCalledWith({ composer: "New Composer" });
  });

  it("discards changes when Discard is clicked", () => {
    const onSave = vi.fn();
    render(
      <MetadataEditor {...baseProps} onSave={onSave} />
    );

    const titleInput = screen.getByDisplayValue("Test Title");
    fireEvent.change(titleInput, { target: { value: "New Title" } });
    expect(screen.getByText(/Discard/)).toBeTruthy();

    fireEvent.click(screen.getByText(/Discard/));
    // Input should be back to original value
    expect(screen.getByDisplayValue("Test Title")).toBeTruthy();
    // Save Changes button should not be present (nothing to save)
    expect(screen.queryByText(/Save Changes/)).toBeNull();
  });
});
