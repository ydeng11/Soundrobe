import { describe, it, expect, vi } from "vitest";
import type { AssistantActionBatch } from "../../electron/services/AssistantRuntime";
import type { TrackData } from "../../electron/handlers/tracks";

vi.mock("electron", () => ({
  ipcMain: { handle: vi.fn() },
  BrowserWindow: { getAllWindows: () => [] },
}));

import {
  isInsideDirectory,
  metadataBatchToExtraInputs,
  metadataBatchToStandardUpdates,
  planTrackNumbering,
  resolveTargetPathsForState,
} from "../../electron/handlers/assistant";

function track(path: string): TrackData {
  return {
    path,
    title: "Track",
    artist: null,
    artists: [],
    album: null,
    albumArtist: null,
    albumArtists: [],
    year: null,
    genre: null,
    composer: null,
    comment: null,
    lyrics: null,
    trackNumber: null,
    trackTotal: null,
    discNumber: null,
    discTotal: null,
    compilation: null,
    musicbrainzTrackId: null,
    musicbrainzAlbumId: null,
    musicbrainzArtistId: null,
    hasCover: false,
    sizeBytes: 0,
    bitrate: null,
    sampleRate: null,
    codec: "FLAC",
    duration: 0,
  };
}

function batch(actions: AssistantActionBatch["actions"]): AssistantActionBatch {
  return {
    id: "batch-test",
    createdAt: new Date(0).toISOString(),
    sessionId: "session-test",
    kind: "metadata-update",
    title: "Edit metadata",
    summary: "Edit metadata",
    riskLevel: "low",
    actions,
    reversible: true,
    status: "pending",
  };
}

describe("assistant composite macro helpers", () => {
  it("checks directory containment with path boundaries", () => {
    expect(isInsideDirectory("/lib/album/01.flac", "/lib/album")).toBe(true);
    expect(isInsideDirectory("/lib/album deluxe/02.flac", "/lib/album")).toBe(false);
  });

  it("resolves selected, active album, library, and explicit target scopes", () => {
    const tracks = [
      track("/lib/album/01.flac"),
      track("/lib/album/02.flac"),
      track("/lib/other/03.flac"),
    ];
    const state = {
      activeAlbumPath: "/lib/album",
      selectedTrackPaths: ["/lib/other/03.flac"],
      tracks,
    };

    expect(resolveTargetPathsForState(state, "selected").paths).toEqual([
      "/lib/other/03.flac",
    ]);
    expect(resolveTargetPathsForState(state, "active_album").paths).toEqual([
      "/lib/album/01.flac",
      "/lib/album/02.flac",
    ]);
    expect(resolveTargetPathsForState(state, "library").paths).toEqual([
      "/lib/album/01.flac",
      "/lib/album/02.flac",
      "/lib/other/03.flac",
    ]);
    expect(resolveTargetPathsForState(state, "explicit_paths", [
      "/lib/album/01.flac",
      "/outside/not-loaded.flac",
    ]).paths).toEqual(["/lib/album/01.flac"]);
  });

  it("does not include active album path-prefix siblings", () => {
    const state = {
      activeAlbumPath: "/lib/album",
      selectedTrackPaths: [],
      tracks: [
        track("/lib/album/01.flac"),
        track("/lib/album deluxe/02.flac"),
      ],
    };

    expect(resolveTargetPathsForState(state, "active_album").paths).toEqual([
      "/lib/album/01.flac",
    ]);
  });

  it("groups standard metadata actions into per-track tag updates", () => {
    const updates = metadataBatchToStandardUpdates(batch([
      {
        tagKind: "standard",
        trackPath: "/lib/a.flac",
        field: "title",
        operation: "set",
        oldValue: "Old",
        newValue: "New",
      },
      {
        tagKind: "standard",
        trackPath: "/lib/a.flac",
        field: "genre",
        operation: "set",
        oldValue: "Rock",
        newValue: null,
      },
      {
        tagKind: "extra",
        trackPath: "/lib/a.flac",
        field: "MOOD",
        operation: "upsert",
        oldValue: null,
        newValue: "night",
      },
    ]));

    expect(updates).toEqual([
      {
        trackPath: "/lib/a.flac",
        fields: { title: "New", genre: null },
      },
    ]);
  });

  it("groups extra metadata actions into per-track extra tag inputs", () => {
    const inputs = metadataBatchToExtraInputs(batch([
      {
        tagKind: "extra",
        trackPath: "/lib/a.flac",
        field: "MOOD",
        operation: "upsert",
        oldValue: null,
        newValue: "night",
      },
      {
        tagKind: "extra",
        trackPath: "/lib/a.flac",
        field: "BPM",
        operation: "remove",
        oldValue: "120",
        newValue: null,
      },
      {
        tagKind: "standard",
        trackPath: "/lib/a.flac",
        field: "title",
        operation: "set",
        oldValue: "Old",
        newValue: "New",
      },
    ]));

    expect(inputs).toEqual([
      {
        trackPath: "/lib/a.flac",
        upserts: [{ key: "MOOD", value: "night" }],
        removes: ["BPM"],
      },
    ]);
  });
});

describe("planTrackNumbering", () => {
  it("assigns sequential numbers when tracks have null track numbers, sorted by filename", () => {
    const allTracks = [
      {
        ...track("/lib/album/03 - Third.flac"),
        trackNumber: null,
        discNumber: null,
      },
      {
        ...track("/lib/album/01 - First.flac"),
        trackNumber: null,
        discNumber: null,
      },
      {
        ...track("/lib/album/02 - Second.flac"),
        trackNumber: null,
        discNumber: null,
      },
    ];

    const result = planTrackNumbering(
      allTracks.map((t) => t.path),
      allTracks,
    );

    expect(result).toHaveLength(3);
    // Sorted by filename: "01 - First.flac", "02 - Second.flac", "03 - Third.flac"
    expect(result[0]).toMatchObject({ desiredTrackNumber: 1, desiredTrackTotal: 3 });
    expect(result[1]).toMatchObject({ desiredTrackNumber: 2, desiredTrackTotal: 3 });
    expect(result[2]).toMatchObject({ desiredTrackNumber: 3, desiredTrackTotal: 3 });
  });

  it("compacts gaps (1, 3, 5) into sequential (1, 2, 3)", () => {
    const allTracks = [
      { ...track("/lib/album/01.flac"), trackNumber: 1, discNumber: null },
      { ...track("/lib/album/03.flac"), trackNumber: 3, discNumber: null },
      { ...track("/lib/album/05.flac"), trackNumber: 5, discNumber: null },
    ];

    const result = planTrackNumbering(
      allTracks.map((t) => t.path),
      allTracks,
    );

    expect(result).toHaveLength(3);
    expect(result[0]).toMatchObject({ desiredTrackNumber: 1, desiredTrackTotal: 3 });
    expect(result[1]).toMatchObject({ desiredTrackNumber: 2, desiredTrackTotal: 3 });
    expect(result[2]).toMatchObject({ desiredTrackNumber: 3, desiredTrackTotal: 3 });
  });

  it("handles mixed discs, numbering per-disc starting from 1", () => {
    const allTracks = [
      { ...track("/lib/album/disc1-01.flac"), trackNumber: 1, discNumber: 1 },
      { ...track("/lib/album/disc1-02.flac"), trackNumber: 2, discNumber: 1 },
      { ...track("/lib/album/disc1-03.flac"), trackNumber: 3, discNumber: 1 },
      { ...track("/lib/album/disc2-01.flac"), trackNumber: 1, discNumber: 2 },
      { ...track("/lib/album/disc2-02.flac"), trackNumber: 2, discNumber: 2 },
    ];

    const result = planTrackNumbering(
      allTracks.map((t) => t.path),
      allTracks,
    );

    expect(result).toHaveLength(5);
    // Disc 1 tracks
    expect(result[0]).toMatchObject({
      desiredTrackNumber: 1, desiredTrackTotal: 3,
      desiredDiscNumber: 1, desiredDiscTotal: 2,
    });
    expect(result[1]).toMatchObject({
      desiredTrackNumber: 2, desiredTrackTotal: 3,
      desiredDiscNumber: 1, desiredDiscTotal: 2,
    });
    expect(result[2]).toMatchObject({
      desiredTrackNumber: 3, desiredTrackTotal: 3,
      desiredDiscNumber: 1, desiredDiscTotal: 2,
    });
    // Disc 2 tracks
    expect(result[3]).toMatchObject({
      desiredTrackNumber: 1, desiredTrackTotal: 2,
      desiredDiscNumber: 2, desiredDiscTotal: 2,
    });
    expect(result[4]).toMatchObject({
      desiredTrackNumber: 2, desiredTrackTotal: 2,
      desiredDiscNumber: 2, desiredDiscTotal: 2,
    });
  });

  it("renumbers library-wide targets within each album instead of treating all loaded tracks as one album", () => {
    const allTracks = [
      { ...track("/lib/loose/a1.flac"), album: "Alpha", albumArtist: "Artist A", trackNumber: 1 },
      { ...track("/lib/loose/a2.flac"), album: "Alpha", albumArtist: "Artist A", trackNumber: 3 },
      { ...track("/lib/loose/b1.flac"), album: "Beta", albumArtist: "Artist B", trackNumber: 2 },
      { ...track("/lib/loose/b2.flac"), album: "Beta", albumArtist: "Artist B", trackNumber: 4 },
    ];

    const result = planTrackNumbering(
      allTracks.map((t) => t.path),
      allTracks,
    );

    expect(result).toEqual([
      expect.objectContaining({
        trackPath: "/lib/loose/a1.flac",
        desiredTrackNumber: 1,
        desiredTrackTotal: 2,
      }),
      expect.objectContaining({
        trackPath: "/lib/loose/a2.flac",
        desiredTrackNumber: 2,
        desiredTrackTotal: 2,
      }),
      expect.objectContaining({
        trackPath: "/lib/loose/b1.flac",
        desiredTrackNumber: 1,
        desiredTrackTotal: 2,
      }),
      expect.objectContaining({
        trackPath: "/lib/loose/b2.flac",
        desiredTrackNumber: 2,
        desiredTrackTotal: 2,
      }),
    ]);
  });

  it("returns already-correct numbering unchanged", () => {
    const allTracks = [
      { ...track("/lib/album/01.flac"), trackNumber: 1, discNumber: null },
      { ...track("/lib/album/02.flac"), trackNumber: 2, discNumber: null },
      { ...track("/lib/album/03.flac"), trackNumber: 3, discNumber: null },
    ];

    const result = planTrackNumbering(
      allTracks.map((t) => t.path),
      allTracks,
    );

    expect(result).toHaveLength(3);
    expect(result[0]).toMatchObject({ desiredTrackNumber: 1, desiredTrackTotal: 3 });
    expect(result[1]).toMatchObject({ desiredTrackNumber: 2, desiredTrackTotal: 3 });
    expect(result[2]).toMatchObject({ desiredTrackNumber: 3, desiredTrackTotal: 3 });
  });

  it("returns empty array for empty input", () => {
    const result = planTrackNumbering([], []);
    expect(result).toEqual([]);
  });
});
