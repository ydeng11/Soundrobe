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
  planStripFilenamePrefixes,
  planStripTitlePrefixes,
  stripFilenamePrefix,
  stripTitlePrefix,
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

describe("stripTitlePrefix", () => {
  it("strips '01. ' prefix", () => {
    expect(stripTitlePrefix("01. 友情岁月")).toBe("友情岁月");
  });

  it("strips '1. ' prefix (no leading zero)", () => {
    expect(stripTitlePrefix("1. Hello")).toBe("Hello");
  });

  it("strips '01 - ' prefix with dash", () => {
    expect(stripTitlePrefix("01 - Hello World")).toBe("Hello World");
  });

  it("strips '01 – ' prefix with en-dash", () => {
    expect(stripTitlePrefix("01 – Hello")).toBe("Hello");
  });

  it("strips '01) ' prefix", () => {
    expect(stripTitlePrefix("01) Hello")).toBe("Hello");
  });

  it("strips bare track number followed by a space", () => {
    expect(stripTitlePrefix("01 寂寞在唱歌")).toBe("寂寞在唱歌");
  });

  it("returns null for null input", () => {
    expect(stripTitlePrefix(null)).toBeNull();
  });

  it("returns original title when no prefix found", () => {
    expect(stripTitlePrefix("友情岁月")).toBe("友情岁月");
  });

  it("returns original title when no digit prefix", () => {
    expect(stripTitlePrefix("Hello World")).toBe("Hello World");
  });

  it("does not strip internal numbers", () => {
    expect(stripTitlePrefix("Track 01 B-side")).toBe("Track 01 B-side");
  });

  it("strips multi-digit prefixes", () => {
    expect(stripTitlePrefix("123. Title")).toBe("Title");
  });

  it("strips 'NN - ' prefix", () => {
    expect(stripTitlePrefix("10 - Title")).toBe("Title");
  });
});

describe("planStripTitlePrefixes", () => {
  it("strips prefixes from tracks with numbered titles", () => {
    const allTracks = [
      { ...track("/lib/album/01.flac"), title: "01. 友情岁月" },
      { ...track("/lib/album/02.flac"), title: "02. 战无不胜" },
      { ...track("/lib/album/03.flac"), title: "03. 古古惑惑" },
    ];

    const result = planStripTitlePrefixes(
      allTracks.map((t) => t.path),
      allTracks,
    );

    expect(result).toHaveLength(3);
    expect(result[0]).toEqual({
      trackPath: "/lib/album/01.flac",
      fields: { title: "友情岁月" },
    });
    expect(result[1]).toEqual({
      trackPath: "/lib/album/02.flac",
      fields: { title: "战无不胜" },
    });
    expect(result[2]).toEqual({
      trackPath: "/lib/album/03.flac",
      fields: { title: "古古惑惑" },
    });
  });

  it("skips tracks without prefixes", () => {
    const allTracks = [
      { ...track("/lib/album/01.flac"), title: "友情岁月" },
      { ...track("/lib/album/02.flac"), title: "战无不胜" },
    ];

    const result = planStripTitlePrefixes(
      allTracks.map((t) => t.path),
      allTracks,
    );

    expect(result).toHaveLength(0);
  });

  it("handles empty input", () => {
    const result = planStripTitlePrefixes([], []);
    expect(result).toEqual([]);
  });

  it("skips tracks with null titles", () => {
    const allTracks = [
      { ...track("/lib/album/01.flac"), title: null },
    ];

    const result = planStripTitlePrefixes(
      allTracks.map((t) => t.path),
      allTracks,
    );

    expect(result).toHaveLength(0);
  });

  it("processes a mix of prefixed and clean titles", () => {
    const allTracks = [
      { ...track("/lib/album/01.flac"), title: "01. Intro" },
      { ...track("/lib/album/02.flac"), title: "Main Song" },
      { ...track("/lib/album/03.flac"), title: "03 - Outro" },
    ];

    const result = planStripTitlePrefixes(
      allTracks.map((t) => t.path),
      allTracks,
    );

    expect(result).toHaveLength(2);
    expect(result[0]).toEqual({
      trackPath: "/lib/album/01.flac",
      fields: { title: "Intro" },
    });
    expect(result[1]).toEqual({
      trackPath: "/lib/album/03.flac",
      fields: { title: "Outro" },
    });
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

describe("stripFilenamePrefix", () => {
  it("strips '01 ' prefix with space", () => {
    expect(stripFilenamePrefix("01 寂寞在唱歌.wav")).toBe("寂寞在唱歌.wav");
  });

  it("strips '01. ' prefix", () => {
    expect(stripFilenamePrefix("01. Track.flac")).toBe("Track.flac");
  });

  it("strips '01 - ' prefix with dash", () => {
    expect(stripFilenamePrefix("01 - Song.mp3")).toBe("Song.mp3");
  });

  it("strips '01) ' prefix with paren", () => {
    expect(stripFilenamePrefix("01) Track.flac")).toBe("Track.flac");
  });

  it("strips multi-digit prefixes", () => {
    expect(stripFilenamePrefix("123. Title.wav")).toBe("Title.wav");
  });

  it("strips '1 - ' single-digit prefix", () => {
    expect(stripFilenamePrefix("1 - Song.flac")).toBe("Song.flac");
  });

  it("returns original when no prefix", () => {
    expect(stripFilenamePrefix("寂寞在唱歌.wav")).toBe("寂寞在唱歌.wav");
  });

  it("does not strip internal numbers", () => {
    expect(stripFilenamePrefix("Track 123 Title.flac")).toBe("Track 123 Title.flac");
  });

  it("preserves extension", () => {
    expect(stripFilenamePrefix("01. intro.flac")).toBe("intro.flac");
  });
});

describe("planStripFilenamePrefixes", () => {
  it("strips prefixes from track paths", () => {
    const result = planStripFilenamePrefixes([
      "/lib/album/01 寂寞在唱歌.wav",
      "/lib/album/02 一直很安静.wav",
      "/lib/album/03 叶子.wav",
    ]);

    expect(result).toEqual([
      { sourcePath: "/lib/album/01 寂寞在唱歌.wav", destinationPath: "/lib/album/寂寞在唱歌.wav" },
      { sourcePath: "/lib/album/02 一直很安静.wav", destinationPath: "/lib/album/一直很安静.wav" },
      { sourcePath: "/lib/album/03 叶子.wav", destinationPath: "/lib/album/叶子.wav" },
    ]);
  });

  it("skips files without prefixes", () => {
    const result = planStripFilenamePrefixes([
      "/lib/album/寂寞在唱歌.wav",
      "/lib/album/一直很安静.wav",
    ]);

    expect(result).toHaveLength(0);
  });

  it("handles empty input", () => {
    const result = planStripFilenamePrefixes([]);
    expect(result).toEqual([]);
  });

  it("handles a mix of prefixed and clean filenames", () => {
    const result = planStripFilenamePrefixes([
      "/lib/album/01 Intro.flac",
      "/lib/album/Main Song.flac",
      "/lib/album/03 - Outro.flac",
    ]);

    expect(result).toEqual([
      { sourcePath: "/lib/album/01 Intro.flac", destinationPath: "/lib/album/Intro.flac" },
      { sourcePath: "/lib/album/03 - Outro.flac", destinationPath: "/lib/album/Outro.flac" },
    ]);
  });

  it("preserves directory structure", () => {
    const result = planStripFilenamePrefixes([
      "/lib/compilation/disc1/01 Track.flac",
      "/lib/compilation/disc1/02 Track.flac",
    ]);

    expect(result).toEqual([
      { sourcePath: "/lib/compilation/disc1/01 Track.flac", destinationPath: "/lib/compilation/disc1/Track.flac" },
      { sourcePath: "/lib/compilation/disc1/02 Track.flac", destinationPath: "/lib/compilation/disc1/Track.flac" },
    ]);
  });
});
