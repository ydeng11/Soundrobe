import { describe, expect, it, vi } from "vitest";
import { buildAutoTagUndoSnapshots } from "../../src/App";
import type { AlbumDetail, TrackData } from "../../electron/preload";

function makeTrack(path: string, title: string): TrackData {
  return {
    path,
    title,
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
    comment: null,
    description: null,
    lyrics: null,
    compilation: null,
    musicbrainzTrackId: null,
    musicbrainzAlbumId: null,
    musicbrainzArtistId: null,
    hasCover: false,
    sizeBytes: 0,
    bitrate: null,
    sampleRate: null,
    codec: "flac",
    duration: 0,
  };
}

describe("buildAutoTagUndoSnapshots", () => {
  it("reads every targeted album so batch auto-tag can undo unloaded albums", async () => {
    const loadedTrack = makeTrack("/music/邓丽君/假如我是真的/01.flac", "old loaded title");
    const unloadedTrack = makeTrack("/music/邓丽君/香港之恋/01.flac", "old unloaded title");
    const readAlbum = vi.fn(async (albumPath: string): Promise<AlbumDetail> => ({
      path: albumPath,
      name: albumPath.split("/").pop() ?? albumPath,
      tracks: albumPath.endsWith("香港之恋") ? [unloadedTrack] : [loadedTrack],
    }));

    const snapshots = await buildAutoTagUndoSnapshots(
      ["/music/邓丽君/假如我是真的", "/music/邓丽君/香港之恋"],
      [loadedTrack],
      readAlbum,
    );

    expect(readAlbum).toHaveBeenCalledTimes(2);
    expect(snapshots.map((snapshot) => snapshot.path)).toEqual([
      loadedTrack.path,
      unloadedTrack.path,
    ]);
    expect(snapshots[1].fields.title).toBe("old unloaded title");
  });

  it("fails before auto-tag when an unloaded album cannot be snapshotted", async () => {
    const loadedTrack = makeTrack("/music/邓丽君/假如我是真的/01.flac", "old loaded title");
    const readAlbum = vi.fn(async (albumPath: string): Promise<AlbumDetail> => {
      if (albumPath.endsWith("香港之恋")) {
        throw new Error("read failed");
      }
      return {
        path: albumPath,
        name: albumPath.split("/").pop() ?? albumPath,
        tracks: [loadedTrack],
      };
    });

    await expect(buildAutoTagUndoSnapshots(
      ["/music/邓丽君/假如我是真的", "/music/邓丽君/香港之恋"],
      [loadedTrack],
      readAlbum,
    )).rejects.toThrow(
      "Cannot auto-tag without undo snapshot for /music/邓丽君/香港之恋: read failed",
    );
  });
});
