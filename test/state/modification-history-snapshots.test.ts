import { describe, expect, it } from "vitest";
import {
  createOwnedTrackSnapshot,
  filterChangedSnapshots,
  extraTagsEqual,
} from "../../src/App";
import type { TrackData } from "../../src/shared/desktop-api";

function makeTrack(path: string, overrides: Partial<TrackData> = {}): TrackData {
  return {
    path,
    title: "Old title",
    artist: "Old artist",
    artists: ["Old artist", "Guest"],
    album: "Album",
    albumArtist: "Album artist",
    albumArtists: ["Album artist"],
    trackNumber: 1,
    trackTotal: 2,
    discNumber: 1,
    discTotal: 1,
    year: "2020",
    genre: "Rock",
    composer: null,
    comment: null,
    description: null,
    lyrics: null,
    compilation: null,
    musicbrainzTrackId: null,
    musicbrainzAlbumId: null,
    musicbrainzArtistId: null,
    discogsArtistId: null,
    discogsReleaseId: null,
    hasCover: false,
    sizeBytes: 1,
    bitrate: null,
    sampleRate: null,
    codec: "flac",
    duration: 1,
    ...overrides,
  };
}

describe("modification history snapshots", () => {
  it("captures only fields owned by a right-panel command", () => {
    const snapshot = createOwnedTrackSnapshot(makeTrack("/one.flac"), {
      artist: "New artist",
      genre: "Pop",
    });

    expect(snapshot.fields).toEqual({ artist: "Old artist", genre: "Rock" });
    expect(snapshot.fields).not.toHaveProperty("artists");
  });

  it("keeps only targets whose authoritative readback changed", () => {
    const snapshots = [
      createOwnedTrackSnapshot(makeTrack("/changed.flac"), { title: "New" }),
      createOwnedTrackSnapshot(makeTrack("/noop.flac"), { title: "Old title" }),
      createOwnedTrackSnapshot(makeTrack("/failed.flac"), { title: "New" }),
    ];

    expect(
      filterChangedSnapshots(snapshots, [
        makeTrack("/changed.flac", { title: "New" }),
        makeTrack("/noop.flac"),
      ]).map((snapshot) => snapshot.path),
    ).toEqual(["/changed.flac"]);
  });

  it("compares Extra Tags by normalized key/value state without treating order as a write", () => {
    const before = [{ key: "MOOD", value: "Calm" }];
    expect(extraTagsEqual(before, before.map((tag) => ({ ...tag })))).toBe(true);
    expect(
      extraTagsEqual(
        [
          { key: "mood", value: " Calm " },
          { key: "label", value: "Example" },
        ],
        [
          { key: "LABEL", value: "Example" },
          { key: "MOOD", value: "Calm" },
        ],
      ),
    ).toBe(true);
    expect(
      extraTagsEqual(
        [
          { key: "ARTISTS", value: "Lead" },
          { key: "ARTISTS", value: "Guest" },
        ],
        [
          { key: "ARTISTS", value: "Guest" },
          { key: "ARTISTS", value: "Lead" },
        ],
      ),
    ).toBe(false);
    expect(extraTagsEqual(before, [{ key: "MOOD", value: "Bright" }])).toBe(false);
  });
});
