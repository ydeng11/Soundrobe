import { describe, it, expect } from "vitest";
import {
  makeTrackCandidate,
  makeAlbumCandidate,
  makeLookupRequest,
  albumCandidateToJson,
  albumCandidateFromJson,
  lookupRequestToJson,
  lookupRequestFromJson,
  candidatesToJson,
  candidatesFromJson,
  queryHash,
} from "../../electron/handlers/candidates";

describe("AlbumCandidate — provider IDs", () => {
  it("creates with discogs IDs via defaults", () => {
    const c = makeAlbumCandidate();
    expect(c.discogsArtistId).toBeNull();
    expect(c.discogsReleaseId).toBeNull();
  });

  it("creates with discogs IDs via overrides", () => {
    const c = makeAlbumCandidate({
      discogsArtistId: "1902728",
      discogsReleaseId: "6951078",
    });
    expect(c.discogsArtistId).toBe("1902728");
    expect(c.discogsReleaseId).toBe("6951078");
  });

  it("round-trips discogs IDs through album candidate JSON", () => {
    const original = makeAlbumCandidate({
      artist: "Hedgehog",
      album: "幻象波普星",
      musicbrainzAlbumId: "mb-album-1",
      musicbrainzArtistId: "mb-artist-1",
      discogsArtistId: "1902728",
      discogsReleaseId: "6951078",
      tracks: [makeTrackCandidate({ title: "Track 1", trackNumber: 1 })],
      source: "discogs",
    });
    const json = albumCandidateToJson(original);
    const restored = albumCandidateFromJson(json);
    expect(restored.discogsArtistId).toBe("1902728");
    expect(restored.discogsReleaseId).toBe("6951078");
    expect(restored.musicbrainzAlbumId).toBe("mb-album-1");
    expect(restored).toEqual(original);
  });

  it("handles null discogs IDs in JSON round-trip", () => {
    const original = makeAlbumCandidate({ artist: "A", album: "B" });
    const json = albumCandidateToJson(original);
    const restored = albumCandidateFromJson(json);
    expect(restored.discogsArtistId).toBeNull();
    expect(restored.discogsReleaseId).toBeNull();
  });

  it("serializes discogs IDs through candidate list JSON", () => {
    const list = [
      makeAlbumCandidate({
        artist: "Hedgehog",
        discogsArtistId: "1902728",
        discogsReleaseId: "6951078",
      }),
      makeAlbumCandidate({
        artist: "刺猬",
        discogsArtistId: "1902728",
      }),
    ];
    const json = candidatesToJson(list);
    const restored = candidatesFromJson(json);
    expect(restored[0].discogsArtistId).toBe("1902728");
    expect(restored[0].discogsReleaseId).toBe("6951078");
    expect(restored[1].discogsArtistId).toBe("1902728");
    expect(restored[1].discogsReleaseId).toBeNull();
  });
});

describe("LookupRequest — provider IDs", () => {
  it("creates with provider IDs via overrides", () => {
    const r = makeLookupRequest({
      musicbrainzAlbumId: "mb-album-1",
      musicbrainzArtistId: "mb-artist-1",
      discogsReleaseId: "6951078",
      discogsArtistId: "1902728",
    });
    expect(r.musicbrainzAlbumId).toBe("mb-album-1");
    expect(r.musicbrainzArtistId).toBe("mb-artist-1");
    expect(r.discogsReleaseId).toBe("6951078");
    expect(r.discogsArtistId).toBe("1902728");
  });

  it("defaults all provider IDs to null", () => {
    const r = makeLookupRequest();
    expect(r.musicbrainzAlbumId).toBeNull();
    expect(r.musicbrainzArtistId).toBeNull();
    expect(r.discogsReleaseId).toBeNull();
    expect(r.discogsArtistId).toBeNull();
  });

  it("round-trips provider IDs through JSON", () => {
    const original = makeLookupRequest({
      path: "/music/刺猬/幻象波普星",
      artistHint: "刺猬",
      albumHint: "幻象波普星",
      musicbrainzAlbumId: "mb-album-1",
      musicbrainzArtistId: "mb-artist-1",
      discogsReleaseId: "6951078",
      discogsArtistId: "1902728",
      tracks: [makeTrackCandidate({ title: "T1" })],
    });
    const json = lookupRequestToJson(original);
    const restored = lookupRequestFromJson(json);
    expect(restored.musicbrainzAlbumId).toBe("mb-album-1");
    expect(restored.musicbrainzArtistId).toBe("mb-artist-1");
    expect(restored.discogsReleaseId).toBe("6951078");
    expect(restored.discogsArtistId).toBe("1902728");
    expect(restored).toEqual(original);
  });

  it("includes provider IDs in query hash so ID-backed lookups get unique cache keys", () => {
    const r1 = makeLookupRequest({
      artistHint: "Hedgehog",
      albumHint: "Phantom Pop Star",
      musicbrainzAlbumId: "mb-album-1",
    });
    const r2 = makeLookupRequest({
      artistHint: "Hedgehog",
      albumHint: "Phantom Pop Star",
    });
    const r3 = makeLookupRequest({
      artistHint: "Hedgehog",
      albumHint: "Phantom Pop Star",
      discogsReleaseId: "6951078",
    });
    const r4 = makeLookupRequest({
      artistHint: "Hedgehog",
      albumHint: "Phantom Pop Star",
      musicbrainzAlbumId: "mb-album-1",
    });

    expect(queryHash(r1)).not.toBe(queryHash(r2));
    expect(queryHash(r1)).not.toBe(queryHash(r3));
    expect(queryHash(r2)).not.toBe(queryHash(r3));
    expect(queryHash(r1)).toBe(queryHash(r4));
  });

  it("includes per-track MusicBrainz IDs because they change track alignment", () => {
    const withTrackId = makeLookupRequest({
      artistHint: "品冠",
      albumHint: "那些女孩教我的事",
      tracks: [
        makeTrackCandidate({
          title: "06.小白很乖",
          trackNumber: 6,
          musicbrainzTrackId: "96fd68c2-669e-4906-8d8e-041e48e3f78e",
        }),
      ],
    });
    const withoutTrackId = makeLookupRequest({
      artistHint: "品冠",
      albumHint: "那些女孩教我的事",
      tracks: [
        makeTrackCandidate({
          title: "06.小白很乖",
          trackNumber: 6,
        }),
      ],
    });

    expect(queryHash(withTrackId)).not.toBe(queryHash(withoutTrackId));
  });
});
