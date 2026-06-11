import { describe, it, expect, vi, beforeEach } from "vitest";
import { TrackTagService } from "../../electron/services/TrackTagService";

// Mock the tracks module
vi.mock("../../electron/handlers/tracks", () => ({
  readTrackMetadata: vi.fn(),
}));

// Mock the write queue
const mockQueueSubmit = vi.fn();
vi.mock("../../electron/services/TagWriteQueue", () => ({
  getDefaultWriteQueue: () => ({
    submit: mockQueueSubmit,
  }),
}));

import { readTrackMetadata } from "../../electron/handlers/tracks";

describe("TrackTagService", () => {
  let service: TrackTagService;

  beforeEach(() => {
    service = new TrackTagService();
    vi.clearAllMocks();
  });

  describe("planTagUpdates", () => {
    it("produces actions showing field diffs", async () => {
      (readTrackMetadata as any).mockResolvedValue({
        path: "/test/track.flac",
        title: "Old Title",
        artist: "Old Artist",
        album: "Old Album",
        albumArtist: null,
        trackNumber: 1,
        trackTotal: null,
        discNumber: null,
        discTotal: null,
        year: "2023",
        genre: "Rock",
        composer: null,
        comment: null,
        lyrics: null,
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
        artists: [],
        albumArtists: [],
      });

      const plan = await service.planTagUpdates([
        {
          trackPath: "/test/track.flac",
          fields: { title: "New Title", album: "New Album", year: "2024" },
        },
      ]);

      expect(plan.kind).toBe("tag-update");
      expect(plan.actions).toHaveLength(3);
      expect(plan.affectedTracks).toBe(1);
      expect(plan.reversible).toBe(true);

      const titleAction = plan.actions.find((a) => a.field === "title");
      expect(titleAction).toBeDefined();
      expect(titleAction!.oldValue).toBe("Old Title");
      expect(titleAction!.newValue).toBe("New Title");

      const yearAction = plan.actions.find((a) => a.field === "year");
      expect(yearAction).toBeDefined();
      expect(yearAction!.oldValue).toBe("2023");
      expect(yearAction!.newValue).toBe("2024");
    });

    it("skips fields when values are the same", async () => {
      (readTrackMetadata as any).mockResolvedValue({
        path: "/test/track.flac",
        title: "Same Title",
        artist: null,
        album: null,
        albumArtist: null,
        trackNumber: null,
        trackTotal: null,
        discNumber: null,
        discTotal: null,
        year: null,
        genre: null,
        composer: null,
        comment: null,
        lyrics: null,
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
        artists: [],
        albumArtists: [],
      });

      const plan = await service.planTagUpdates([
        {
          trackPath: "/test/track.flac",
          fields: { title: "Same Title" },
        },
      ]);

      expect(plan.actions).toHaveLength(0);
      expect(plan.summary).toBe("No changes needed");
    });

    it("returns batch summary with no actions when no fields given", async () => {
      (readTrackMetadata as any).mockResolvedValue({
        path: "/test/track.flac",
        title: "Title",
        artist: null,
        album: null,
        albumArtist: null,
        trackNumber: null,
        trackTotal: null,
        discNumber: null,
        discTotal: null,
        year: null,
        genre: null,
        composer: null,
        comment: null,
        lyrics: null,
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
        artists: [],
        albumArtists: [],
      });

      const plan = await service.planTagUpdates([
        { trackPath: "/test/track.flac", fields: {} },
      ]);

      expect(plan.actions).toHaveLength(0);
      expect(plan.summary).toBe("No changes needed");
    });
  });

  describe("applyTagUpdates", () => {
    it("writes tags and re-reads metadata", async () => {
      mockQueueSubmit.mockResolvedValue([
        { filePath: "/test/track.flac", success: true },
      ]);

      (readTrackMetadata as any).mockResolvedValue({
        path: "/test/track.flac",
        title: "Updated",
        artist: null,
        album: null,
        albumArtist: null,
        trackNumber: null,
        trackTotal: null,
        discNumber: null,
        discTotal: null,
        year: null,
        genre: null,
        composer: null,
        comment: null,
        lyrics: null,
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
        artists: [],
        albumArtists: [],
      });

      const results = await service.applyTagUpdates([
        {
          trackPath: "/test/track.flac",
          fields: { title: "Updated" },
        },
      ]);

      // Verify the queue was called with the right job
      expect(mockQueueSubmit).toHaveBeenCalledWith([
        { filePath: "/test/track.flac", fields: { title: "Updated" } },
      ]);
      expect(results).toHaveLength(1);
      expect(results[0].success).toBe(true);
      expect(results[0].updatedTrack?.title).toBe("Updated");
    });
  });
});
