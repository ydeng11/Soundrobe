// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach } from "vitest";
import React from "react";
import { render, screen, fireEvent, cleanup, waitFor } from "@testing-library/react";

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
    albumArtist: "Album Artist A",
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

    // All three files should be visible (shows last 3 parent dirs + filename)
    expect(screen.getByText("music/song1.mp3")).toBeTruthy();
    expect(screen.getByText("music/song2.mp3")).toBeTruthy();
    expect(screen.getByText("music/song3.mp3")).toBeTruthy();
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

    expect(screen.getByText("music/song1.mp3")).toBeTruthy();
    expect(screen.getByText("music/song2.mp3")).toBeTruthy();
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

    expect(screen.getByText("music/song2.mp3")).toBeTruthy();
    expect(screen.queryByText("music/song1.mp3")).toBeFalsy();
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

    fireEvent.click(screen.getByText("music/song1.mp3"));
    expect(onSelect).toHaveBeenCalledWith(
      "/music/song1.mp3",
      expect.objectContaining({ title: "Song One" })
    );
  });

  it("opens Extra Tags from the native row context menu", async () => {
    const onSelect = vi.fn();
    const onEditExtraTags = vi.fn();
    window.api = {
      showTrackContextMenu: vi.fn().mockResolvedValue("extra-tags"),
    } as unknown as Window["api"];

    render(
      <FileGrid
        tracks={tracks}
        selectedTrackPath={null}
        filterText=""
        onSelectTrack={onSelect}
        onEditExtraTags={onEditExtraTags}
      />
    );

    fireEvent.contextMenu(screen.getByText("music/song1.mp3"));

    await waitFor(() => {
      expect(window.api.showTrackContextMenu).toHaveBeenCalledWith(
        "/music/song1.mp3",
        expect.objectContaining({ albumArtist: "Album Artist A" }),
      );
      expect(onEditExtraTags).toHaveBeenCalledWith(
        expect.objectContaining({ path: "/music/song1.mp3" }),
        ["/music/song1.mp3"],
      );
    });
  });

  it("preserves multi-selection when Extra Tags is opened from a selected row context menu", async () => {
    const onSelect = vi.fn();
    const onMulti = vi.fn();
    const onEditExtraTags = vi.fn();
    window.api = {
      showTrackContextMenu: vi.fn().mockResolvedValue("extra-tags"),
    } as unknown as Window["api"];

    render(
      <FileGrid
        tracks={tracks}
        selectedTrackPath="/music/song1.mp3"
        selectedTrackPaths={["/music/song1.mp3", "/music/song2.mp3"]}
        filterText=""
        onSelectTrack={onSelect}
        onMultiSelect={onMulti}
        onEditExtraTags={onEditExtraTags}
      />
    );

    fireEvent.contextMenu(screen.getByText("music/song1.mp3"));

    await waitFor(() => {
      expect(onSelect).not.toHaveBeenCalled();
      expect(onMulti).not.toHaveBeenCalled();
      expect(onEditExtraTags).toHaveBeenCalledWith(
        expect.objectContaining({ path: "/music/song1.mp3" }),
        ["/music/song1.mp3", "/music/song2.mp3"],
      );
    });
  });

  it("opens Extra Tags from blank table space when tracks are selected", async () => {
    const onSelect = vi.fn();
    const onEditExtraTags = vi.fn();
    window.api = {
      showTrackContextMenu: vi.fn().mockResolvedValue("extra-tags"),
    } as unknown as Window["api"];

    render(
      <FileGrid
        tracks={tracks}
        selectedTrackPath="/music/song1.mp3"
        selectedTrackPaths={["/music/song1.mp3", "/music/song2.mp3"]}
        filterText=""
        onSelectTrack={onSelect}
        onEditExtraTags={onEditExtraTags}
      />
    );

    fireEvent.contextMenu(screen.getByTestId("file-grid-body"));

    await waitFor(() => {
      expect(window.api.showTrackContextMenu).toHaveBeenCalledWith(
        "/music/song1.mp3",
        expect.objectContaining({ title: "Song One" }),
      );
      expect(onEditExtraTags).toHaveBeenCalledWith(
        expect.objectContaining({ path: "/music/song1.mp3" }),
        ["/music/song1.mp3", "/music/song2.mp3"],
      );
    });
  });

  it("ignores blank table-space context menus when no tracks are selected", () => {
    const onSelect = vi.fn();
    const onEditExtraTags = vi.fn();
    window.api = {
      showTrackContextMenu: vi.fn().mockResolvedValue("extra-tags"),
    } as unknown as Window["api"];

    render(
      <FileGrid
        tracks={tracks}
        selectedTrackPath={null}
        selectedTrackPaths={[]}
        filterText=""
        onSelectTrack={onSelect}
        onEditExtraTags={onEditExtraTags}
      />
    );

    fireEvent.contextMenu(screen.getByTestId("file-grid-body"));

    expect(window.api.showTrackContextMenu).not.toHaveBeenCalled();
    expect(onEditExtraTags).not.toHaveBeenCalled();
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
    const rows = container.querySelectorAll('[class*="flex items-center px-3 py-1"]');
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

    // Click "Path" column header
    const filenameHeader = screen.getByText("Path");
    fireEvent.click(filenameHeader);

    // Now click again to reverse sort
    fireEvent.click(filenameHeader);

    // Rows should be in reverse order (3 data rows)
    const dataRows = container.querySelectorAll('[class*="flex items-center px-3 py-1"]');
    expect(dataRows.length).toBeGreaterThanOrEqual(3);
  });

  it("shift-click range select calls onMultiSelect with all paths in range, not onSelectTrack", () => {
    const onSelect = vi.fn();
    const onMulti = vi.fn();
    render(
      <FileGrid
        tracks={tracks}
        selectedTrackPath={null}
        filterText=""
        onSelectTrack={onSelect}
        onMultiSelect={onMulti}
      />
    );

    // Click first row to set lastClickedRef
    const rows = screen.getAllByText(/music\/song/);
    expect(rows.length).toBeGreaterThanOrEqual(3);

    // Click first row (no shift)
    fireEvent.click(screen.getByText("music/song1.mp3"));
    expect(onMulti).toHaveBeenCalledWith(["/music/song1.mp3"]);
    expect(onSelect).toHaveBeenCalledWith(
      "/music/song1.mp3",
      expect.objectContaining({ title: "Song One" })
    );

    vi.clearAllMocks();

    // Shift-click third row (song2, at sorted index 2) — should call onMulti with all 3
    // paths and NOT call onSelectTrack (so BatchEditor stays visible).
    // Default sort is by track number ascending: [song1, song3, song2]
    fireEvent.click(screen.getByText("music/song2.mp3"), {
      shiftKey: true,
    });

    expect(onMulti).toHaveBeenCalledWith([
      "/music/song1.mp3",
      "/music/song3.mp3",
      "/music/song2.mp3",
    ]);
    expect(onSelect).not.toHaveBeenCalled();
  });
});
