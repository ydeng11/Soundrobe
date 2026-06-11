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
  });
});
