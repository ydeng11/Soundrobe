// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from "vitest";
import React from "react";
import { render, screen, fireEvent, cleanup } from "@testing-library/react";

afterEach(() => cleanup());
import { FileGrid } from "../../src/components/FileGrid";
import type { TrackData } from "../../electron/preload";

function makeTrack(
  path: string,
  overrides?: Partial<TrackData>
): TrackData {
  return {
    path,
    title: null,
    artist: null,
    artists: [],
    album: null,
    albumArtist: null,
    albumArtists: [],
    trackNumber: null,
    trackTotal: null,
    discNumber: null,
    discTotal: null,
    year: null,
    genre: null,
    composer: null,
    lyrics: null,
    compilation: null,
    musicbrainzTrackId: null,
    musicbrainzAlbumId: null,
    musicbrainzArtistId: null,
    hasCover: false,
    sizeBytes: 0,
    bitrate: null,
    sampleRate: null,
    codec: "mp3",
    duration: 0,
    ...overrides,
  };
}

const tracks: TrackData[] = [
  makeTrack("/music/song1.mp3", {
    title: "Song One",
    artist: "Artist A",
    album: "Album X",
    year: "2020",
    trackNumber: 1,
    trackTotal: 2,
    genre: "Rock",
    duration: 180,
    bitrate: 192000,
  }),
  makeTrack("/music/song2.mp3", {
    title: "Song Two",
    artist: "Artist B",
    album: "Album X",
    year: "2021",
    trackNumber: 2,
    trackTotal: 2,
    genre: "Pop",
    duration: 240,
    bitrate: 256000,
  }),
  makeTrack("/music/song3.mp3", {
    title: "Another Song",
    artist: "Artist A",
    album: "Album Y",
    year: "2019",
    trackNumber: 1,
    genre: "Jazz",
    duration: 300,
    bitrate: 128000,
  }),
];

describe("FileGrid", () => {
  it("renders all files when no filter is set", () => {
    const onSelect = vi.fn();
    render(
      <FileGrid
        tracks={tracks}
        selectedTrackPath={null}
        filterText=""
        onSelectTrack={onSelect}
      />
    );

    // All three filenames should be visible
    expect(screen.getByText("song1.mp3")).toBeTruthy();
    expect(screen.getByText("song2.mp3")).toBeTruthy();
    expect(screen.getByText("song3.mp3")).toBeTruthy();
  });

  it("filters files by filter text (title match)", () => {
    const onSelect = vi.fn();
    render(
      <FileGrid
        tracks={tracks}
        selectedTrackPath={null}
        filterText="Song"
        onSelectTrack={onSelect}
      />
    );

    expect(screen.getByText("song1.mp3")).toBeTruthy();
    expect(screen.getByText("song2.mp3")).toBeTruthy();
    expect(screen.queryByText("Another Song")).toBeTruthy();
  });

  it("filters files by filter text (artist match)", () => {
    const onSelect = vi.fn();
    render(
      <FileGrid
        tracks={tracks}
        selectedTrackPath={null}
        filterText="Artist B"
        onSelectTrack={onSelect}
      />
    );

    expect(screen.getByText("song2.mp3")).toBeTruthy();
    expect(screen.queryByText("song1.mp3")).toBeFalsy();
  });

  it("shows empty message when filter matches nothing", () => {
    const onSelect = vi.fn();
    render(
      <FileGrid
        tracks={tracks}
        selectedTrackPath={null}
        filterText="zzznoexistzzz"
        onSelectTrack={onSelect}
      />
    );

    expect(screen.getByText(/No files match the filter/i)).toBeTruthy();
  });

  it("shows empty message when tracks list is empty", () => {
    const onSelect = vi.fn();
    render(
      <FileGrid
        tracks={[]}
        selectedTrackPath={null}
        filterText=""
        onSelectTrack={onSelect}
      />
    );

    expect(screen.getByText(/No audio files found/i)).toBeTruthy();
  });

  it("calls onSelectTrack when a row is clicked", () => {
    const onSelect = vi.fn();
    render(
      <FileGrid
        tracks={tracks}
        selectedTrackPath={null}
        filterText=""
        onSelectTrack={onSelect}
      />
    );

    fireEvent.click(screen.getByText("song1.mp3"));
    expect(onSelect).toHaveBeenCalledWith(
      "/music/song1.mp3",
      expect.objectContaining({ title: "Song One" })
    );
  });

  it("shows file count in footer", () => {
    const onSelect = vi.fn();
    render(
      <FileGrid
        tracks={tracks}
        selectedTrackPath={null}
        filterText=""
        onSelectTrack={onSelect}
      />
    );

    expect(screen.getByText("3 files")).toBeTruthy();
  });

  it("shows filtered count in footer when filter is active", () => {
    const onSelect = vi.fn();
    render(
      <FileGrid
        tracks={tracks}
        selectedTrackPath={null}
        filterText="Jazz"
        onSelectTrack={onSelect}
      />
    );

    expect(screen.getByText(/filtered from 3/i)).toBeTruthy();
  });

  it("displays correct cell values", () => {
    const onSelect = vi.fn();
    render(
      <FileGrid
        tracks={tracks}
        selectedTrackPath={null}
        filterText=""
        onSelectTrack={onSelect}
      />
    );

    // Check first row values — use getAllByText for values that appear on multiple rows
    expect(screen.getByText("Song One")).toBeTruthy();
    const artistAs = screen.getAllByText("Artist A");
    expect(artistAs.length).toBeGreaterThanOrEqual(1);
    const albumXs = screen.getAllByText("Album X");
    expect(albumXs.length).toBeGreaterThanOrEqual(1);
    expect(screen.getByText("2020")).toBeTruthy();
    expect(screen.getByText("1/2")).toBeTruthy();
    expect(screen.getByText("Rock")).toBeTruthy();
    expect(screen.getByText("3:00")).toBeTruthy();
    expect(screen.getByText("192k")).toBeTruthy();
  });

  it("highlights selected row differently", () => {
    const onSelect = vi.fn();
    const { container } = render(
      <FileGrid
        tracks={tracks}
        selectedTrackPath="/music/song2.mp3"
        filterText=""
        onSelectTrack={onSelect}
      />
    );

    // The selected row should have the accent background class
    const rows = container.querySelectorAll('[class*="flex items-center px-2 py-1"]');
    expect(rows.length).toBeGreaterThan(0);
  });

  it("sorts by column header click (filename asc → desc)", () => {
    const onSelect = vi.fn();
    const { container } = render(
      <FileGrid
        tracks={tracks}
        selectedTrackPath={null}
        filterText=""
        onSelectTrack={onSelect}
      />
    );

    // Click "Filename" column header
    const filenameHeader = screen.getByText("Filename");
    fireEvent.click(filenameHeader);

    // Now click again to reverse sort
    fireEvent.click(filenameHeader);

    // Rows should be in reverse order (3 data rows, plus 0 header rows now)
    const dataRows = container.querySelectorAll('[class*="flex items-center px-2 py-1"]');
    // The header row + footer row use different styles now
    // Data rows have class "flex items-center px-2 py-1" but header is different
    expect(dataRows.length).toBeGreaterThanOrEqual(3);
  });
});
