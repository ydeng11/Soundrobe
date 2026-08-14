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
    probeWriteVolumeReal: vi.fn().mockRejectedValue(new Error("no-op")),
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
    searchReleases: vi.fn().mockRejectedValue(new Error("no-op")),
    resolveRelease: vi.fn().mockRejectedValue(new Error("no-op")),
    previewReleaseMatch: vi.fn().mockRejectedValue(new Error("no-op")),
    searchApplyCandidate: vi.fn().mockRejectedValue(new Error("no-op")),
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

  it("summarizes structured lyrics embedding results", async () => {
    const downloadAlbumLyrics = vi.fn().mockResolvedValue({
      total: 5,
      written: 2,
      embeddedPreserved: 1,
      noLyrics: 0,
      unsupported: 1,
      failed: 1,
      results: [],
    });
    window.api.downloadAlbumLyrics = downloadAlbumLyrics;

    render(<App />);
    await act(async () => {
      fireEvent.click(screen.getByText("Open Library"));
    });
    await waitFor(() => expect(screen.getAllByTestId(/^file-row-/)).toHaveLength(2));

    await act(async () => {
      fireEvent.click(screen.getByText("Get Lyrics"));
    });

    await waitFor(() => {
      expect(downloadAlbumLyrics).toHaveBeenCalledWith("/music/Test Album");
      expect(
        screen.getByText(
          "Lyrics: 2 embedded, 1 preserved, 0 unavailable, 1 unsupported, 1 failed",
        ),
      ).toBeTruthy();
    });
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

  it("batch disc field is split into discNumber/discTotal", async () => {
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

    // Multi-select both tracks
    await act(async () => {
      fireEvent.click(trackRows![0], { metaKey: true });
    });
    await act(async () => {
      fireEvent.click(trackRows![1], { metaKey: true });
    });
    await waitFor(() => expect(screen.getByText("Batch Edit")).toBeTruthy());

    // Set Disc to "1"
    const discInput = screen.getByPlaceholderText(/Disc number/);
    await act(async () => {
      fireEvent.change(discInput, { target: { value: "1" } });
    });

    // Click "Apply changes"
    await act(async () => {
      fireEvent.click(screen.getByText("Apply changes"));
    });

    await waitFor(() => {
      expect(window.api.writeTracks).toHaveBeenCalled();
      const writes = (window.api.writeTracks as ReturnType<typeof vi.fn>).mock
        .calls[0][0] as Array<{ path: string; fields: Record<string, unknown> }>;
      expect(writes).toHaveLength(2);
      // Each update must have discNumber, NOT a raw "disc" key
      for (const w of writes) {
        expect(w.fields).not.toHaveProperty("disc");
        expect(w.fields).toHaveProperty("discNumber", 1);
        expect(w.fields).not.toHaveProperty("discTotal");
      }
    });
  });

  it("batch disc field with total splits into discNumber and discTotal", async () => {
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

    // Multi-select both tracks
    await act(async () => {
      fireEvent.click(trackRows![0], { metaKey: true });
    });
    await act(async () => {
      fireEvent.click(trackRows![1], { metaKey: true });
    });
    await waitFor(() => expect(screen.getByText("Batch Edit")).toBeTruthy());

    // Set Disc to "1/2"
    const discInput = screen.getByPlaceholderText(/Disc number/);
    await act(async () => {
      fireEvent.change(discInput, { target: { value: "1/2" } });
    });

    // Click "Apply changes"
    await act(async () => {
      fireEvent.click(screen.getByText("Apply changes"));
    });

    await waitFor(() => {
      expect(window.api.writeTracks).toHaveBeenCalled();
      const writes = (window.api.writeTracks as ReturnType<typeof vi.fn>).mock
        .calls[0][0] as Array<{ path: string; fields: Record<string, unknown> }>;
      expect(writes).toHaveLength(2);
      for (const w of writes) {
        expect(w.fields).not.toHaveProperty("disc");
        expect(w.fields).toHaveProperty("discNumber", 1);
        expect(w.fields).toHaveProperty("discTotal", 2);
      }
    });
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

describe("App — modification history", () => {
  it("does not bind Cmd/Ctrl+Z and restores only right-panel-owned fields", async () => {
    const path = "/music/Test Album/01.mp3";
    const writeTrack = window.api.writeTrack as ReturnType<typeof vi.fn>;
    writeTrack
      .mockResolvedValueOnce(makeTrack(path, { title: "New title" }))
      .mockResolvedValueOnce(makeTrack(path, { title: "Old title" }));

    render(<App />);
    fireEvent.click(screen.getByText("Open Library"));
    const row = (await screen.findAllByTestId(/^file-row-/))[0];
    fireEvent.click(row);
    const titleInput = await screen.findByPlaceholderText("Track title");
    fireEvent.change(titleInput, { target: { value: "New title" } });
    fireEvent.click(screen.getByRole("button", { name: "Apply Changes" }));

    await waitFor(() => expect(writeTrack).toHaveBeenCalledTimes(1));
    fireEvent.keyDown(window, { key: "z", metaKey: true });
    expect(writeTrack).toHaveBeenCalledTimes(1);

    fireEvent.click(
      screen.getByRole("button", { name: "Undo latest modification" }),
    );
    await waitFor(() => expect(writeTrack).toHaveBeenCalledTimes(2));
    expect(writeTrack.mock.calls[1]).toEqual([path, { title: expect.any(String) }]);
    expect(writeTrack.mock.calls[1][1]).not.toHaveProperty("artists");
  });

  it("captures and restores complete Extra Tags through the native API", async () => {
    const path = "/music/Test Album/01.mp3";
    (window.api.showTrackContextMenu as ReturnType<typeof vi.fn>).mockResolvedValue(
      "extra-tags",
    );
    (window.api.readExtraTags as ReturnType<typeof vi.fn>)
      .mockResolvedValueOnce([
        { key: "MOOD", value: "Calm", source: "Vorbis" },
      ])
      .mockResolvedValueOnce([
        { key: "MOOD", value: "Calm", source: "Vorbis" },
      ])
      .mockResolvedValueOnce([
        { key: "MOOD", value: "Bright", source: "Vorbis" },
      ])
      .mockResolvedValueOnce([
        { key: "MOOD", value: "Calm", source: "Vorbis" },
      ]);
    const writeExtraTags = window.api.writeExtraTags as ReturnType<typeof vi.fn>;
    writeExtraTags.mockResolvedValue(makeTrack(path));

    render(<App />);
    fireEvent.click(screen.getByText("Open Library"));
    const row = (await screen.findAllByTestId(/^file-row-/))[0];
    fireEvent.contextMenu(row);

    const valueInput = await screen.findByDisplayValue("Calm");
    fireEvent.change(valueInput, { target: { value: "Bright" } });
    fireEvent.click(screen.getByRole("button", { name: "Save Changes" }));
    await waitFor(() => expect(writeExtraTags).toHaveBeenCalledTimes(1));

    fireEvent.click(
      screen.getByRole("button", { name: "Undo latest modification" }),
    );
    await waitFor(() => expect(writeExtraTags).toHaveBeenCalledTimes(2));
    expect(writeExtraTags.mock.calls[1]).toEqual([
      path,
      [{ key: "MOOD", value: "Calm" }],
    ]);
  });

  it("records and reverts successful files from a partially failed Batch Extra Tags save", async () => {
    const paths = [
      "/music/Test Album/01.mp3",
      "/music/Test Album/02.mp3",
    ];
    const tagState = new Map(
      paths.map((path) => [path, [{ key: "MOOD", value: "Calm" }]]),
    );
    (window.api.showTrackContextMenu as ReturnType<typeof vi.fn>).mockResolvedValue(
      "extra-tags",
    );
    (window.api.readExtraTags as ReturnType<typeof vi.fn>).mockImplementation(
      async (path: string) =>
        (tagState.get(path) ?? []).map((tag) => ({ ...tag, source: "Vorbis" })),
    );
    (window.api.writeExtraTagsBatch as ReturnType<typeof vi.fn>).mockImplementation(
      async (updates: Array<{ path: string; tags: Array<{ key: string; value: string }> }>) => {
        tagState.set(updates[0].path, updates[0].tags);
        throw new Error("second file failed");
      },
    );
    const writeExtraTags = window.api.writeExtraTags as ReturnType<typeof vi.fn>;
    writeExtraTags.mockImplementation(
      async (path: string, tags: Array<{ key: string; value: string }>) => {
        tagState.set(path, tags);
        return makeTrack(path);
      },
    );

    render(<App />);
    fireEvent.click(screen.getByText("Open Library"));
    const rows = await screen.findAllByTestId(/^file-row-/);
    fireEvent.click(rows[0], { metaKey: true });
    fireEvent.click(rows[1], { metaKey: true });
    fireEvent.contextMenu(rows[0]);

    const valueInput = await screen.findByDisplayValue("Calm");
    fireEvent.change(valueInput, { target: { value: "Bright" } });
    fireEvent.click(screen.getByRole("button", { name: "Apply to 2 files" }));
    await screen.findByText("second file failed");

    fireEvent.click(
      screen.getByRole("button", { name: "Undo latest modification" }),
    );
    await waitFor(() => expect(writeExtraTags).toHaveBeenCalledOnce());
    expect(writeExtraTags).toHaveBeenCalledWith(paths[0], [
      { key: "MOOD", value: "Calm" },
    ]);
  });

  it("keeps completed albums revertible when a later auto-tag start fails", async () => {
    const albums = ["/music/Album One", "/music/Album Two"];
    const modifiedAlbums = new Set<string>();
    (window.api.scanLibrary as ReturnType<typeof vi.fn>).mockResolvedValue(
      albums.map((path) => ({
        path,
        name: path.split("/").at(-1)!,
        artistHint: "",
        albumHint: "",
        trackCount: 2,
      })),
    );
    (window.api.readAlbum as ReturnType<typeof vi.fn>).mockImplementation(
      async (albumPath: string) => ({
        path: albumPath,
        name: albumPath.split("/").at(-1)!,
        artistHint: "",
        albumHint: "",
        status: "complete",
        tracks: [1, 2].map((number) =>
          makeTrack(`${albumPath}/0${number}.mp3`, {
            title: modifiedAlbums.has(albumPath)
              ? `Tagged ${number}`
              : `Original ${number}`,
          }),
        ),
        coverInfo: { path: null, source: "missing", dataUrl: null },
      }),
    );
    (window.api.autoTagAlbum as ReturnType<typeof vi.fn>).mockImplementation(
      async (albumPath: string) => {
        if (albumPath === albums[1]) throw new Error("could not start second album");
        modifiedAlbums.add(albumPath);
        return "task-one";
      },
    );
    (window.api.getTaskProgress as ReturnType<typeof vi.fn>).mockResolvedValue({
      status: "completed",
      taskId: "task-one",
      progress: 1,
      total: 1,
      message: "Done",
      result: null,
    });
    const writeTrack = window.api.writeTrack as ReturnType<typeof vi.fn>;
    writeTrack.mockImplementation(async (path: string, fields: Record<string, unknown>) =>
      makeTrack(path, fields as Partial<TrackData>),
    );

    render(<App />);
    fireEvent.click(screen.getByText("Open Library"));
    await waitFor(() => expect(screen.getAllByTestId(/^file-row-/)).toHaveLength(4));
    fireEvent.click(screen.getByText("Auto-Tag"));
    await screen.findByText("could not start second album");

    fireEvent.click(
      screen.getByRole("button", { name: "Undo latest modification" }),
    );
    await waitFor(() => expect(writeTrack).toHaveBeenCalledTimes(2));
    expect(writeTrack.mock.calls.map((call) => call[0])).toEqual([
      `${albums[0]}/01.mp3`,
      `${albums[0]}/02.mp3`,
    ]);
  });

  it("records readable auto-tag changes when another album readback fails", async () => {
    const albums = ["/music/Unreadable", "/music/Readable"];
    const modifiedAlbums = new Set<string>();
    const readCounts = new Map<string, number>();
    (window.api.scanLibrary as ReturnType<typeof vi.fn>).mockResolvedValue(
      albums.map((path) => ({
        path,
        name: path.split("/").at(-1)!,
        artistHint: "",
        albumHint: "",
        trackCount: 2,
      })),
    );
    (window.api.readAlbum as ReturnType<typeof vi.fn>).mockImplementation(
      async (albumPath: string) => {
        const count = (readCounts.get(albumPath) ?? 0) + 1;
        readCounts.set(albumPath, count);
        if (albumPath === albums[0] && count > 1) {
          throw new Error("readback unavailable");
        }
        return {
          path: albumPath,
          name: albumPath.split("/").at(-1)!,
          artistHint: "",
          albumHint: "",
          status: "complete",
          tracks: [1, 2].map((number) =>
            makeTrack(`${albumPath}/0${number}.mp3`, {
              title: modifiedAlbums.has(albumPath)
                ? `Tagged ${number}`
                : `Original ${number}`,
            }),
          ),
          coverInfo: { path: null, source: "missing", dataUrl: null },
        };
      },
    );
    (window.api.autoTagAlbum as ReturnType<typeof vi.fn>).mockImplementation(
      async (albumPath: string) => {
        modifiedAlbums.add(albumPath);
        return `task-${albumPath}`;
      },
    );
    (window.api.getTaskProgress as ReturnType<typeof vi.fn>).mockResolvedValue({
      status: "completed",
      taskId: "task",
      progress: 1,
      total: 1,
      message: "Done",
      result: null,
    });
    const writeTrack = window.api.writeTrack as ReturnType<typeof vi.fn>;
    writeTrack.mockImplementation(async (path: string, fields: Record<string, unknown>) =>
      makeTrack(path, fields as Partial<TrackData>),
    );

    render(<App />);
    fireEvent.click(screen.getByText("Open Library"));
    await waitFor(() => expect(screen.getAllByTestId(/^file-row-/)).toHaveLength(4));
    fireEvent.click(screen.getByText("Auto-Tag"));
    await screen.findByText(/Auto-tag readback failed.*readback unavailable/);

    fireEvent.click(
      screen.getByRole("button", { name: "Undo latest modification" }),
    );
    await waitFor(() => expect(writeTrack).toHaveBeenCalledTimes(2));
    expect(writeTrack.mock.calls.map((call) => call[0])).toEqual([
      `${albums[1]}/01.mp3`,
      `${albums[1]}/02.mp3`,
    ]);
  });

  it("reverts an older history point newest-first after confirmation", async () => {
    const path = "/music/Test Album/01.mp3";
    const originalTitle = makeTrack(path).title;
    const writeTrack = window.api.writeTrack as ReturnType<typeof vi.fn>;
    writeTrack
      .mockResolvedValueOnce(makeTrack(path, { title: "First edit" }))
      .mockResolvedValueOnce(makeTrack(path, { title: "Second edit" }))
      .mockResolvedValueOnce(makeTrack(path, { title: "First edit" }))
      .mockResolvedValueOnce(makeTrack(path, { title: originalTitle }));
    const confirm = vi.spyOn(window, "confirm").mockReturnValue(true);

    render(<App />);
    fireEvent.click(screen.getByText("Open Library"));
    fireEvent.click((await screen.findAllByTestId(/^file-row-/))[0]);

    let titleInput = await screen.findByPlaceholderText("Track title");
    fireEvent.change(titleInput, { target: { value: "First edit" } });
    fireEvent.click(screen.getByRole("button", { name: "Apply Changes" }));
    await waitFor(() => expect(writeTrack).toHaveBeenCalledTimes(1));

    titleInput = await screen.findByPlaceholderText("Track title");
    fireEvent.change(titleInput, { target: { value: "Second edit" } });
    fireEvent.click(screen.getByRole("button", { name: "Apply Changes" }));
    await waitFor(() => expect(writeTrack).toHaveBeenCalledTimes(2));

    fireEvent.click(
      screen.getByRole("button", { name: "Open modification history" }),
    );
    fireEvent.click(screen.getAllByRole("menuitem")[1]);

    await waitFor(() => expect(writeTrack).toHaveBeenCalledTimes(4));
    expect(confirm).toHaveBeenCalledOnce();
    expect(writeTrack.mock.calls[2]).toEqual([path, { title: "First edit" }]);
    expect(writeTrack.mock.calls[3]).toEqual([path, { title: originalTitle }]);
    confirm.mockRestore();
  });

  it("keeps a failed revert retryable and removes it after a successful retry", async () => {
    const path = "/music/Test Album/01.mp3";
    const writeTrack = window.api.writeTrack as ReturnType<typeof vi.fn>;
    writeTrack
      .mockResolvedValueOnce(makeTrack(path, { title: "New title" }))
      .mockRejectedValueOnce(new Error("disk full"))
      .mockResolvedValueOnce(makeTrack(path));

    render(<App />);
    fireEvent.click(screen.getByText("Open Library"));
    fireEvent.click((await screen.findAllByTestId(/^file-row-/))[0]);
    const titleInput = await screen.findByPlaceholderText("Track title");
    fireEvent.change(titleInput, { target: { value: "New title" } });
    fireEvent.click(screen.getByRole("button", { name: "Apply Changes" }));
    await waitFor(() => expect(writeTrack).toHaveBeenCalledTimes(1));

    const undo = screen.getByRole("button", { name: "Undo latest modification" });
    fireEvent.click(undo);
    await screen.findByText(/Revert stopped after 1 file\(s\) failed/);
    expect(undo.getAttribute("disabled")).toBeNull();

    fireEvent.click(undo);
    await waitFor(() => expect(writeTrack).toHaveBeenCalledTimes(3));
    await waitFor(() => expect(undo.getAttribute("disabled")).not.toBeNull());
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

describe("App — auto-tag artwork refresh", () => {
  it("evicts a cached missing cover and refetches the active album after auto-tag", async () => {
    const getCoverDataUrl = window.api
      .getCoverDataUrl as ReturnType<typeof vi.fn>;
    getCoverDataUrl.mockResolvedValue(null);
    (window.api.autoTagAlbum as ReturnType<typeof vi.fn>).mockResolvedValue(
      "auto-tag-1",
    );

    render(<App />);
    await act(async () => {
      fireEvent.click(screen.getByText("Open Library"));
    });

    let trackRows: HTMLElement[];
    await waitFor(() => {
      trackRows = screen.getAllByTestId(/^file-row-/);
      expect(trackRows.length).toBe(2);
    });

    await act(async () => {
      fireEvent.click(trackRows![0]);
    });
    await waitFor(() => {
      expect(getCoverDataUrl).toHaveBeenCalled();
      expect(screen.getByText(/No cover/)).toBeTruthy();
    });

    getCoverDataUrl.mockClear();
    getCoverDataUrl.mockResolvedValue(
      "data:image/jpeg;base64,auto-tag-cover",
    );

    await act(async () => {
      fireEvent.click(screen.getByText("Auto-Tag"));
    });

    await waitFor(() => {
      expect(getCoverDataUrl).toHaveBeenCalledWith(
        "/music/Test Album",
        "/music/Test Album/01.mp3",
      );
      expect(screen.getByAltText("Cover art")).toBeTruthy();
    });
  });
});
