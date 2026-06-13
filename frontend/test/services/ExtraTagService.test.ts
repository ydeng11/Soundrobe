import { describe, it, expect, vi, beforeEach } from "vitest";
import { ExtraTagService } from "../../electron/services/ExtraTagService";

// Mock the tracks module
vi.mock("../../electron/handlers/tracks", () => ({
  readExtraTags: vi.fn(),
  readTrackMetadata: vi.fn(),
}));

// Mock the write queue
const mockQueueSubmit = vi.fn();
vi.mock("../../electron/services/TagWriteQueue", () => ({
  getDefaultWriteQueue: () => ({
    submit: mockQueueSubmit,
  }),
}));

import { readExtraTags } from "../../electron/handlers/tracks";

describe("ExtraTagService", () => {
  let service: ExtraTagService;

  beforeEach(() => {
    service = new ExtraTagService();
    vi.clearAllMocks();
  });

  describe("planExtraTagUpdates", () => {
    it("plans upsert operations", async () => {
      (readExtraTags as any).mockResolvedValue([
        { key: "MOOD", value: "happy", source: "id3" },
      ]);

      const plan = await service.planExtraTagUpdates([
        {
          trackPath: "/test/track.flac",
          upserts: [{ key: "MOOD", value: "chill" }],
          removes: [],
        },
      ]);

      expect(plan.kind).toBe("extra-tag-update");
      expect(plan.actions).toHaveLength(1);
      expect(plan.actions[0].operation).toBe("upsert");
      expect(plan.actions[0].oldValue).toBe("happy");
      expect(plan.actions[0].newValue).toBe("chill");
      expect(plan.reversible).toBe(true);
    });

    it("plans remove operations", async () => {
      (readExtraTags as any).mockResolvedValue([
        { key: "MOOD", value: "happy", source: "id3" },
        { key: "BPM", value: "120", source: "id3" },
      ]);

      const plan = await service.planExtraTagUpdates([
        {
          trackPath: "/test/track.flac",
          upserts: [],
          removes: ["MOOD"],
        },
      ]);

      expect(plan.actions).toHaveLength(1);
      expect(plan.actions[0].operation).toBe("remove");
      expect(plan.actions[0].key).toBe("MOOD");
      expect(plan.actions[0].oldValue).toBe("happy");
    });

    it("skips upsert when value is unchanged", async () => {
      (readExtraTags as any).mockResolvedValue([
        { key: "MOOD", value: "happy", source: "id3" },
      ]);

      const plan = await service.planExtraTagUpdates([
        {
          trackPath: "/test/track.flac",
          upserts: [{ key: "MOOD", value: "happy" }],
          removes: [],
        },
      ]);

      expect(plan.actions).toHaveLength(0);
    });

    it("handles multiple tracks", async () => {
      (readExtraTags as any)
        .mockResolvedValueOnce([])
        .mockResolvedValueOnce([{ key: "TAG", value: "old", source: "id3" }]);

      const plan = await service.planExtraTagUpdates([
        {
          trackPath: "/test/track1.flac",
          upserts: [{ key: "NEW_TAG", value: "value1" }],
          removes: [],
        },
        {
          trackPath: "/test/track2.flac",
          upserts: [{ key: "TAG", value: "new" }],
          removes: [],
        },
      ]);

      expect(plan.actions).toHaveLength(2);
      expect(plan.affectedTracks).toBe(2);
    });
  });

  describe("applyExtraTagUpdates", () => {
    it("applies upserts and preserves other tags", async () => {
      mockQueueSubmit.mockResolvedValue([
        { filePath: "/test/track.flac", success: true },
      ]);

      (readExtraTags as any).mockResolvedValue([
        { key: "MOOD", value: "happy", source: "id3" },
        { key: "BPM", value: "120", source: "id3" },
      ]);

      const results = await service.applyExtraTagUpdates([
        {
          trackPath: "/test/track.flac",
          upserts: [{ key: "MOOD", value: "chill" }],
          removes: [],
        },
      ]);

      // The queue should receive the merged extra tags
      expect(mockQueueSubmit).toHaveBeenCalledWith([
        expect.objectContaining({
          filePath: "/test/track.flac",
          extraTags: expect.arrayContaining([
            { key: "MOOD", value: "chill" },
            { key: "BPM", value: "120" },
          ]),
        }),
      ]);
      expect(results).toHaveLength(1);
      expect(results[0].success).toBe(true);
    });

    it("removes specified tags", async () => {
      mockQueueSubmit.mockResolvedValue([
        { filePath: "/test/track.flac", success: true },
      ]);

      (readExtraTags as any).mockResolvedValue([
        { key: "MOOD", value: "happy", source: "id3" },
      ]);

      const results = await service.applyExtraTagUpdates([
        {
          trackPath: "/test/track.flac",
          upserts: [],
          removes: ["MOOD"],
        },
      ]);

      expect(mockQueueSubmit).toHaveBeenCalledWith([
        expect.objectContaining({
          filePath: "/test/track.flac",
          extraTags: [],
        }),
      ]);
      expect(results[0].success).toBe(true);
    });

    it("handles write errors gracefully", async () => {
      mockQueueSubmit.mockResolvedValue([
        { filePath: "/test/track.flac", success: false, error: "Write failed" },
      ]);

      (readExtraTags as any).mockResolvedValue([]);

      const results = await service.applyExtraTagUpdates([
        {
          trackPath: "/test/track.flac",
          upserts: [{ key: "TAG", value: "val" }],
          removes: [],
        },
      ]);

      expect(results[0].success).toBe(false);
      expect(results[0].error).toBe("Write failed");
    });

    it("skips track when removal key does not exist (optimization)", async () => {
      // Track has MOOD and BPM, but we're removing GENRE (which doesn't exist)
      (readExtraTags as any).mockResolvedValue([
        { key: "MOOD", value: "happy", source: "id3" },
        { key: "BPM", value: "120", source: "id3" },
      ]);

      const results = await service.applyExtraTagUpdates([
        {
          trackPath: "/test/track.flac",
          upserts: [],
          removes: ["GENRE"],  // GENRE doesn't exist on this track
        },
      ]);

      // Should return success without submitting to write queue
      expect(mockQueueSubmit).not.toHaveBeenCalled();
      expect(results).toHaveLength(1);
      expect(results[0].success).toBe(true);
    });

    it("skips track when upsert value is same as current (optimization)", async () => {
      // Track already has MOOD=happy, upserting same value
      (readExtraTags as any).mockResolvedValue([
        { key: "MOOD", value: "happy", source: "id3" },
      ]);

      const results = await service.applyExtraTagUpdates([
        {
          trackPath: "/test/track.flac",
          upserts: [{ key: "MOOD", value: "happy" }],  // Same value
          removes: [],
        },
      ]);

      // Should return success without submitting to write queue
      expect(mockQueueSubmit).not.toHaveBeenCalled();
      expect(results).toHaveLength(1);
      expect(results[0].success).toBe(true);
    });

    it("processes only affected tracks when removing tag from multiple tracks (optimization)", async () => {
      // Simulate 5 tracks, only 2 have the MOOD tag
      mockQueueSubmit.mockResolvedValue([
        { filePath: "/test/track1.flac", success: true },
        { filePath: "/test/track2.flac", success: true },
      ]);

      (readExtraTags as any)
        .mockResolvedValueOnce([{ key: "MOOD", value: "happy", source: "id3" }])  // track1 has MOOD
        .mockResolvedValueOnce([])  // track2 doesn't have MOOD
        .mockResolvedValueOnce([{ key: "MOOD", value: "sad", source: "id3" }])   // track3 has MOOD
        .mockResolvedValueOnce([])  // track4 doesn't have MOOD
        .mockResolvedValueOnce([{ key: "BPM", value: "120", source: "id3" }]); // track5 has BPM, not MOOD

      const results = await service.applyExtraTagUpdates([
        { trackPath: "/test/track1.flac", upserts: [], removes: ["MOOD"] },
        { trackPath: "/test/track2.flac", upserts: [], removes: ["MOOD"] },
        { trackPath: "/test/track3.flac", upserts: [], removes: ["MOOD"] },
        { trackPath: "/test/track4.flac", upserts: [], removes: ["MOOD"] },
        { trackPath: "/test/track5.flac", upserts: [], removes: ["MOOD"] },
      ]);

      // Only track1 and track3 have MOOD, so only 2 should be submitted to queue
      expect(mockQueueSubmit).toHaveBeenCalledTimes(1);
      expect(mockQueueSubmit).toHaveBeenCalledWith([
        expect.objectContaining({ filePath: "/test/track1.flac", extraTags: [] }),
        expect.objectContaining({ filePath: "/test/track3.flac", extraTags: [] }),
      ]);

      // All 5 tracks should return success
      expect(results).toHaveLength(5);
      expect(results.every((r) => r.success)).toBe(true);
    });
  });
});
