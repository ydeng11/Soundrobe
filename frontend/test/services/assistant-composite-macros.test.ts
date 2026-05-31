import { describe, it, expect, vi } from "vitest";
import type { AssistantActionBatch } from "../../electron/services/AssistantRuntime";
import type { TrackData } from "../../electron/handlers/tracks";

vi.mock("electron", () => ({
  ipcMain: { handle: vi.fn() },
  BrowserWindow: { getAllWindows: () => [] },
}));

import {
  metadataBatchToExtraInputs,
  metadataBatchToStandardUpdates,
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
