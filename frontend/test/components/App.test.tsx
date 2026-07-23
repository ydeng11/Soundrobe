// @vitest-environment jsdom
import { describe, it, expect, vi, afterEach, beforeEach } from "vitest";
import React from "react";
import { render, screen, fireEvent, cleanup, waitFor, act } from "@testing-library/react";
import App from "../../src/App";
import type { TrackData, AlbumInfo, TaskProgress } from "../../src/shared/desktop-api";

afterEach(() => {
  cleanup();
  delete (window as unknown as Record<string, unknown>).api;
});

function makeTrack(path: string, overrides?: Partial<TrackData>): TrackData {
  return {
    path,
    title: `Song ${path.slice(-5)}`,
    artist: "Test Artist",
    artists: [],
    album: "Test Album",
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
    sizeBytes: 1000,
    bitrate: null,
    sampleRate: null,
    codec: "MP3",
    duration: 60,
    ...overrides,
  };
}

beforeEach(() => {
  const disposer = vi.fn();
  window.api = {
    appInfo: vi.fn().mockResolvedValue({
      identifier: "com.soundrobe",
      version: "0.1.0",
      runtime: "tauri" as const,
      dev: false,
    }),
    openFolderDialog: vi.fn().mockResolvedValue("/music"),
    scanLibrary: vi.fn().mockResolvedValue([
      {
        path: "/music/Test Album",
        name: "Test Album",
        artistHint: "",
        albumHint: "",
        trackCount: 2,
      } as AlbumInfo,
    ]),
    readAlbum: vi.fn().mockImplementation((albumPath: string) =>
      Promise.resolve({
        path: albumPath,
        name: "Test Album",
        artistHint: "",
        albumHint: "",
        status: "complete",
        tracks: [
          makeTrack(`${albumPath}/01.mp3`),
          makeTrack(`${albumPath}/02.mp3`),
        ],
        coverInfo: { path: null, source: "missing" as const, dataUrl: null },
      }),
    ),
    refreshAlbum: vi.fn().mockRejectedValue(new Error("no-op")),
    writeTrack: vi.fn().mockRejectedValue(new Error("no-op")),
    writeTracks: vi.fn().mockImplementation(
      (updates: Array<{ path: string; fields: Record<string, unknown> }>) =>
        Promise.resolve({
          tracks: updates.map((u) => makeTrack(u.path, { title: "Updated" })),
          failures: [],
        }),
    ),
    readExtraTags: vi.fn().mockRejectedValue(new Error("no-op")),
    writeExtraTags: vi.fn().mockRejectedValue(new Error("no-op")),
    writeExtraTagsBatch: vi.fn().mockRejectedValue(new Error("no-op")),
    renameTrack: vi.fn().mockRejectedValue(new Error("no-op")),
    checkFileExists: vi.fn().mockRejectedValue(new Error("no-op")),
    probeWriteVolume: vi.fn().mockRejectedValue(new Error("no-op")),
    showTrackContextMenu: vi.fn().mockRejectedValue(new Error("no-op")),
    deleteFiles: vi.fn().mockRejectedValue(new Error("no-op")),
    listDirectory: vi.fn().mockRejectedValue(new Error("no-op")),
    readDirectory: vi.fn().mockRejectedValue(new Error("no-op")),
    autoTagAlbum: vi.fn().mockRejectedValue(new Error("no-op")),
    downloadAlbumLyrics: vi.fn().mockRejectedValue(new Error("no-op")),
    onAutoTagEvent: vi.fn().mockReturnValue(vi.fn()),
    getTaskProgress: vi.fn().mockResolvedValue({
      status: "completed" as const,
      taskId: "noop",
      progress: 0,
      total: 1,
      message: "No-op",
      result: null,
    } as TaskProgress),
    cancelTask: vi.fn().mockRejectedValue(new Error("no-op")),
    getDatasetStatus: vi.fn().mockRejectedValue(new Error("no-op")),
    onTrackWriteEvent: vi.fn().mockReturnValue(disposer),
    runAudit: vi.fn().mockRejectedValue(new Error("no-op")),
    runAuditOnTracks: vi.fn().mockRejectedValue(new Error("no-op")),
    runAuditOnAlbums: vi.fn().mockRejectedValue(new Error("no-op")),
    runAlbumAudit: vi.fn().mockRejectedValue(new Error("no-op")),
    applyAuditFixes: vi.fn().mockRejectedValue(new Error("no-op")),
    onAuditEvent: vi.fn().mockReturnValue(vi.fn()),
    cancelAudit: vi.fn().mockRejectedValue(new Error("no-op")),
    getCoverDataUrl: vi.fn().mockResolvedValue(null),
    setCover: vi.fn().mockRejectedValue(new Error("no-op")),
    removeCover: vi.fn().mockRejectedValue(new Error("no-op")),
    downloadCoverArt: vi.fn().mockRejectedValue(new Error("no-op")),
    downloadArtistArt: vi.fn().mockRejectedValue(new Error("no-op")),
    fetchLyrics: vi.fn().mockRejectedValue(new Error("no-op")),
    getConfig: vi.fn().mockResolvedValue({}),
    setConfig: vi.fn().mockResolvedValue(undefined),
    subscribeDebugLogs: vi.fn().mockResolvedValue(undefined),
    setDebugMode: vi.fn().mockResolvedValue(undefined),
    onFocus: vi.fn().mockResolvedValue(undefined),
    sortByAlbum: vi.fn().mockRejectedValue(new Error("no-op")),
    assistantSend: vi.fn().mockRejectedValue(new Error("no-op")),
    assistantCancel: vi.fn().mockRejectedValue(new Error("no-op")),
    assistantClear: vi.fn().mockRejectedValue(new Error("no-op")),
    assistantApplyActions: vi.fn().mockRejectedValue(new Error("no-op")),
    assistantRejectActions: vi.fn().mockRejectedValue(new Error("no-op")),
    assistantGetBatches: vi.fn().mockResolvedValue([]),
    assistantInitRuntime: vi.fn().mockResolvedValue(undefined),
    assistantInitServices: vi.fn().mockResolvedValue(undefined),
    onAssistantEvent: vi.fn().mockReturnValue(vi.fn()),
    listSessions: vi.fn().mockResolvedValue([]),
    getConversation: vi.fn().mockRejectedValue(new Error("no-op")),
    getSession: vi.fn().mockRejectedValue(new Error("no-op")),
    getCurrentSession: vi.fn().mockRejectedValue(new Error("no-op")),
  } as unknown as Window["api"];
});

describe("App — batch save progress", () => {
  it("renders the title bar and open-library button", async () => {
    render(<App />);
    expect(screen.getByText("Open Library")).toBeTruthy();
  });

  it("subscribes to onTrackWriteEvent on batch save and dispatches progress", async () => {
    render(<App />);
    await act(async () => { await Promise.resolve(); });

    // Open a library to load tracks
    await act(async () => {
      fireEvent.click(screen.getByText("Open Library"));
    });

    // Wait for FileGrid to render track rows
    let trackRows: HTMLElement[];
    await waitFor(() => {
      trackRows = screen.getAllByTestId(/^file-row-/);
      expect(trackRows.length).toBe(2);
    });

    // Multi-select both tracks (cmd+click each)
    await act(async () => { fireEvent.click(trackRows![0], { metaKey: true }); });
    await act(async () => { fireEvent.click(trackRows![1], { metaKey: true }); });

    // BatchEditor panel should be visible
    await waitFor(() => expect(screen.getByText("Batch Edit")).toBeTruthy());

    // Modify a field to make the form dirty (enables "Apply changes")
    const artistInput = screen.getByPlaceholderText(/Common artist/);
    await act(async () => {
      fireEvent.change(artistInput, { target: { value: "New Artist" } });
    });

    // Clear the spy to only count the handleBatchSave subscription
    const trackWriteSpy = window.api.onTrackWriteEvent as ReturnType<typeof vi.fn>;
    trackWriteSpy.mockClear();

    // Click "Apply changes" — now enabled because dirty=true
    await act(async () => {
      fireEvent.click(screen.getByText("Apply changes"));
    });

    // handleBatchSave must have subscribed via onTrackWriteEvent
    expect(trackWriteSpy).toHaveBeenCalledTimes(1);
    const progressCallback = trackWriteSpy.mock.calls[0][0] as (e: {
      current: number;
      total: number;
      message: string;
    }) => void;

    // Simulate a progress event as the Rust batch writer would emit
    await act(async () => {
      progressCallback({ current: 1, total: 2, message: "Writing 1/2" });
    });

    // writeTracks must have been called with both paths
    expect(window.api.writeTracks).toHaveBeenCalled();
    const writes = (window.api.writeTracks as ReturnType<typeof vi.fn>).mock
      .calls[0][0] as Array<{ path: string }>;
    expect(writes).toHaveLength(2);
    expect(writes[0].path).toContain("01.mp3");
    expect(writes[1].path).toContain("02.mp3");
  });

  it("cleans up progress listener after batch save completes", async () => {
    const disposerSpy = vi.fn();
    (window.api.onTrackWriteEvent as ReturnType<typeof vi.fn>).mockReturnValue(disposerSpy);

    render(<App />);
    await act(async () => { await Promise.resolve(); });

    // Open library
    await act(async () => { fireEvent.click(screen.getByText("Open Library")); });

    let trackRows: HTMLElement[];
    await waitFor(() => {
      trackRows = screen.getAllByTestId(/^file-row-/);
      expect(trackRows.length).toBe(2);
    });

    // Multi-select both tracks
    await act(async () => { fireEvent.click(trackRows![0], { metaKey: true }); });
    await act(async () => { fireEvent.click(trackRows![1], { metaKey: true }); });
    await waitFor(() => expect(screen.getByText("Batch Edit")).toBeTruthy());

    // Modify a field to enable "Apply changes"
    const artistInput = screen.getByPlaceholderText(/Common artist/);
    await act(async () => {
      fireEvent.change(artistInput, { target: { value: "New Artist" } });
    });

    // Click "Apply changes"
    await act(async () => {
      fireEvent.click(screen.getByText("Apply changes"));
    });

    // After the save resolves, the disposer must have been called
    await waitFor(() => {
      expect(disposerSpy).toHaveBeenCalled();
    });
  });
});

describe("App — cover removal", () => {
  beforeEach(() => {
    // Override cover-related mocks for this block: show a cover so the
    // Remove button renders, and make the remove API succeed by default.
    (window.api.getCoverDataUrl as ReturnType<typeof vi.fn>).mockResolvedValue(
      "data:image/jpeg;base64,cover123",
    );
    (window.api.removeCover as ReturnType<typeof vi.fn>).mockResolvedValue(true);
  });

  it("removes cover and updates UI when Remove is clicked", async () => {
    render(<App />);
    await act(async () => {
      await Promise.resolve();
    });

    // Open library
    await act(async () => {
      fireEvent.click(screen.getByText("Open Library"));
    });

    // Wait for tracks to appear
    let trackRows: HTMLElement[];
    await waitFor(() => {
      trackRows = screen.getAllByTestId(/^file-row-/);
      expect(trackRows.length).toBe(2);
    });

    // Click a single track (no modifier key) to open MetadataEditor
    await act(async () => {
      fireEvent.click(trackRows![0]);
    });

    // Wait for cover fetch to resolve and cover image to appear
    await waitFor(
      () => {
        expect(screen.getByAltText("Cover art")).toBeTruthy();
      },
      { timeout: 2000 },
    );

    // Remove button must be visible while cover is present
    const removeButton = screen.getByText(/Remove/);
    expect(removeButton).toBeTruthy();

    // Click Remove
    await act(async () => {
      fireEvent.click(removeButton);
    });

    // After removal: IPC called, cover placeholder shows, Remove button gone
    await waitFor(() => {
      expect(window.api.removeCover).toHaveBeenCalledWith("/music/Test Album");
      expect(screen.getByText(/No cover/)).toBeTruthy();
      expect(screen.queryByText(/Remove/)).toBeNull();
    });
  });

  it("shows error when remove cover returns false", async () => {
    (window.api.removeCover as ReturnType<typeof vi.fn>).mockResolvedValue(false);

    render(<App />);
    await act(async () => {
      await Promise.resolve();
    });

    // Open library
    await act(async () => {
      fireEvent.click(screen.getByText("Open Library"));
    });

    let trackRows: HTMLElement[];
    await waitFor(() => {
      trackRows = screen.getAllByTestId(/^file-row-/);
      expect(trackRows.length).toBe(2);
    });

    // Click a single track
    await act(async () => {
      fireEvent.click(trackRows![0]);
    });

    // Wait for cover to appear
    await waitFor(
      () => {
        expect(screen.getByAltText("Cover art")).toBeTruthy();
      },
      { timeout: 2000 },
    );

    // Click Remove
    await act(async () => {
      fireEvent.click(screen.getByText(/Remove/));
    });

    // Error message must appear from the return-value check (not the catch)
    await waitFor(() => {
      expect(screen.getByText("Failed to remove cover art")).toBeTruthy();
    });
  });

  it("shows error when remove cover throws", async () => {
    (window.api.removeCover as ReturnType<typeof vi.fn>).mockRejectedValue(
      new Error("remove failed"),
    );

    render(<App />);
    await act(async () => {
      await Promise.resolve();
    });

    // Open library
    await act(async () => {
      fireEvent.click(screen.getByText("Open Library"));
    });

    let trackRows: HTMLElement[];
    await waitFor(() => {
      trackRows = screen.getAllByTestId(/^file-row-/);
      expect(trackRows.length).toBe(2);
    });

    // Click a single track
    await act(async () => {
      fireEvent.click(trackRows![0]);
    });

    // Wait for cover to appear
    await waitFor(
      () => {
        expect(screen.getByAltText("Cover art")).toBeTruthy();
      },
      { timeout: 2000 },
    );

    // Click Remove
    await act(async () => {
      fireEvent.click(screen.getByText(/Remove/));
    });

    // Error message must appear from the catch branch
    await waitFor(() => {
      expect(screen.getByText("Failed to remove cover art")).toBeTruthy();
    });
  });
});
