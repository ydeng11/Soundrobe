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
    onFieldChange: vi.fn(),
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

  it("calls onFieldChange when a text field is edited", () => {
    const onFieldChange = vi.fn();
    render(
      <MetadataEditor {...baseProps} onFieldChange={onFieldChange} />
    );

    const titleInput = screen.getByDisplayValue("Test Title");
    fireEvent.change(titleInput, { target: { value: "New Title" } });
    expect(onFieldChange).toHaveBeenCalledWith("title", "New Title");
  });

  it("calls onFieldChange when artist is edited", () => {
    const onFieldChange = vi.fn();
    render(
      <MetadataEditor {...baseProps} onFieldChange={onFieldChange} />
    );

    const artistInput = screen.getByDisplayValue("Test Artist");
    fireEvent.change(artistInput, { target: { value: "New Artist" } });
    expect(onFieldChange).toHaveBeenCalledWith("artist", "New Artist");
  });

  it("calls onFieldChange when year is edited", () => {
    const onFieldChange = vi.fn();
    render(
      <MetadataEditor {...baseProps} onFieldChange={onFieldChange} />
    );

    const yearInput = screen.getByDisplayValue("2023");
    fireEvent.change(yearInput, { target: { value: "1999" } });
    expect(onFieldChange).toHaveBeenCalledWith("year", "1999");
  });

  it("shows saving indicator when saving is true", () => {
    render(<MetadataEditor {...baseProps} saving={true} />);
    expect(screen.getByText("● saving")).toBeTruthy();
  });

  it("does not show saving indicator when saving is false", () => {
    render(<MetadataEditor {...baseProps} saving={false} />);
    expect(screen.queryByText("● saving")).toBeFalsy();
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
    fireEvent.click(screen.getByText(/Change/));
    expect(onChangeCover).toHaveBeenCalledOnce();
  });

  it("calls onRemoveCover when Remove button is clicked", () => {
    const onRemoveCover = vi.fn();
    render(
      <MetadataEditor {...baseProps} onRemoveCover={onRemoveCover} />
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
    // All fields should render with empty values
    const emptyInputs = screen.getAllByDisplayValue("");
    expect(emptyInputs.length).toBeGreaterThanOrEqual(6);
  });

  it("calls onFieldChange when composer (textarea) is edited", () => {
    const onFieldChange = vi.fn();
    render(
      <MetadataEditor {...baseProps} onFieldChange={onFieldChange} />
    );
    const composerInputs = screen.getAllByDisplayValue("Test Composer");
    expect(composerInputs.length).toBeGreaterThanOrEqual(1);
    fireEvent.change(composerInputs[0], { target: { value: "New Composer" } });
    expect(onFieldChange).toHaveBeenCalledWith("composer", "New Composer");
  });
});
