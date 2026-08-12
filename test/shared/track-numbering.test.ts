import { describe, expect, it } from "vitest";
import {
  computeNumberedTracks,
  type NumberingInput,
  type OrderingRule,
} from "../../src/shared/track-numbering";

function makeTrack(
  path: string,
  overrides?: Partial<NumberingInput>,
): NumberingInput {
  return {
    path,
    title: null,
    trackNumber: null,
    duration: 0,
    ...overrides,
  };
}

describe("TrackNumberingService", () => {
  // ── Empty / single-track edge cases ──────────────────────

  describe("edge cases", () => {
    it("returns empty array for empty input", () => {
      const result = computeNumberedTracks([], "filename-asc");
      expect(result).toEqual([]);
    });

    it("returns track 1/1 for a single track", () => {
      const result = computeNumberedTracks(
        [makeTrack("/music/song.flac")],
        "filename-asc",
      );
      expect(result).toEqual([
        { path: "/music/song.flac", fields: { trackNumber: 1, trackTotal: 1 } },
      ]);
    });

    it("supports custom startFrom value", () => {
      const tracks = [
        makeTrack("/music/a.flac"),
        makeTrack("/music/b.flac"),
      ];
      const result = computeNumberedTracks(tracks, "filename-asc", 2);
      expect(result).toEqual([
        { path: "/music/a.flac", fields: { trackNumber: 2, trackTotal: 2 } },
        { path: "/music/b.flac", fields: { trackNumber: 3, trackTotal: 2 } },
      ]);
    });
  });

  // ── Filename ordering ────────────────────────────────────

  describe("filename-asc", () => {
    it("sorts tracks by filename alphabetically A-Z", () => {
      const tracks = [
        makeTrack("/music/03-third.flac"),
        makeTrack("/music/01-first.flac"),
        makeTrack("/music/02-second.flac"),
      ];
      const result = computeNumberedTracks(tracks, "filename-asc");
      expect(result.map((r) => r.fields.trackNumber)).toEqual([1, 2, 3]);
      expect(result[0].path).toContain("01-first");
      expect(result[1].path).toContain("02-second");
      expect(result[2].path).toContain("03-third");
    });

    it("handles filenames with different extensions", () => {
      const tracks = [
        makeTrack("/music/b.mp3"),
        makeTrack("/music/a.flac"),
        makeTrack("/music/c.ogg"),
      ];
      const result = computeNumberedTracks(tracks, "filename-asc");
      expect(result[0].path).toContain("a.flac");
      expect(result[1].path).toContain("b.mp3");
      expect(result[2].path).toContain("c.ogg");
    });
  });

  describe("filename-desc", () => {
    it("sorts tracks by filename alphabetically Z-A", () => {
      const tracks = [
        makeTrack("/music/01-first.flac"),
        makeTrack("/music/03-third.flac"),
        makeTrack("/music/02-second.flac"),
      ];
      const result = computeNumberedTracks(tracks, "filename-desc");
      expect(result[0].path).toContain("03-third");
      expect(result[1].path).toContain("02-second");
      expect(result[2].path).toContain("01-first");
    });
  });

  // ── Title ordering ───────────────────────────────────────

  describe("title-asc", () => {
    it("sorts by title A-Z, falling back to filename when title is null", () => {
      const tracks = [
        makeTrack("/music/z.flac", { title: "Zebra" }),
        makeTrack("/music/a.flac", { title: null }), // fallback → "a.flac"
        makeTrack("/music/m.flac", { title: "Apple" }),
      ];
      const result = computeNumberedTracks(tracks, "title-asc");
      // "a.flac" (null title fallback), "Apple", "Zebra"
      expect(result[0].path).toContain("a.flac");
      expect(result[1].path).toContain("m.flac");
      expect(result[2].path).toContain("z.flac");
    });

    it("is case-insensitive", () => {
      const tracks = [
        makeTrack("/music/1.flac", { title: "BRAVO" }),
        makeTrack("/music/2.flac", { title: "alpha" }),
      ];
      const result = computeNumberedTracks(tracks, "title-asc");
      expect(result[0].path).toContain("2.flac"); // "alpha"
      expect(result[1].path).toContain("1.flac"); // "BRAVO"
    });
  });

  describe("title-desc", () => {
    it("sorts by title Z-A", () => {
      const tracks = [
        makeTrack("/music/a.flac", { title: "Apple" }),
        makeTrack("/music/z.flac", { title: "Zebra" }),
      ];
      const result = computeNumberedTracks(tracks, "title-desc");
      expect(result[0].path).toContain("z.flac"); // Zebra first
      expect(result[1].path).toContain("a.flac");
    });
  });

  // ── Existing track number ordering ───────────────────────

  describe("existing-track-asc", () => {
    it("sorts by existing track number ascending", () => {
      const tracks = [
        makeTrack("/music/3.flac", { trackNumber: 3 }),
        makeTrack("/music/1.flac", { trackNumber: 1 }),
        makeTrack("/music/2.flac", { trackNumber: 2 }),
      ];
      const result = computeNumberedTracks(tracks, "existing-track-asc");
      expect(result[0].path).toContain("1.flac");
      expect(result[1].path).toContain("2.flac");
      expect(result[2].path).toContain("3.flac");
      // New numbers are sequential
      expect(result.map((r) => r.fields.trackNumber)).toEqual([1, 2, 3]);
    });

    it("sorts null track numbers to the end", () => {
      const tracks = [
        makeTrack("/music/a.flac", { trackNumber: 2 }),
        makeTrack("/music/b.flac", { trackNumber: null }),
        makeTrack("/music/c.flac", { trackNumber: 1 }),
      ];
      const result = computeNumberedTracks(tracks, "existing-track-asc");
      expect(result[0].path).toContain("c.flac"); // track #1
      expect(result[1].path).toContain("a.flac"); // track #2
      expect(result[2].path).toContain("b.flac"); // null → last
    });

    it("tie-breaks by filename", () => {
      const tracks = [
        makeTrack("/music/b.flac", { trackNumber: 1 }),
        makeTrack("/music/a.flac", { trackNumber: 1 }),
      ];
      const result = computeNumberedTracks(tracks, "existing-track-asc");
      expect(result[0].path).toContain("a.flac");
      expect(result[1].path).toContain("b.flac");
    });
  });

  describe("existing-track-desc", () => {
    it("sorts by existing track number descending", () => {
      const tracks = [
        makeTrack("/music/1.flac", { trackNumber: 1 }),
        makeTrack("/music/3.flac", { trackNumber: 3 }),
        makeTrack("/music/2.flac", { trackNumber: 2 }),
      ];
      const result = computeNumberedTracks(tracks, "existing-track-desc");
      expect(result[0].path).toContain("3.flac");
      expect(result[1].path).toContain("2.flac");
      expect(result[2].path).toContain("1.flac");
    });

    it("sorts null track numbers to the end (lowest priority)", () => {
      const tracks = [
        makeTrack("/music/c.flac", { trackNumber: 1 }),
        makeTrack("/music/b.flac", { trackNumber: null }),
        makeTrack("/music/a.flac", { trackNumber: 2 }),
      ];
      const result = computeNumberedTracks(tracks, "existing-track-desc");
      // Descending: highest number first
      expect(result[0].path).toContain("a.flac"); // track #2 → first
      expect(result[1].path).toContain("c.flac"); // track #1 → second
      expect(result[2].path).toContain("b.flac"); // null → last
    });
  });

  // ── Duration ordering ────────────────────────────────────

  describe("duration-asc", () => {
    it("sorts by duration ascending (shortest first)", () => {
      const tracks = [
        makeTrack("/music/3.flac", { duration: 300 }),
        makeTrack("/music/1.flac", { duration: 100 }),
        makeTrack("/music/2.flac", { duration: 200 }),
      ];
      const result = computeNumberedTracks(tracks, "duration-asc");
      expect(result[0].path).toContain("1.flac"); // 100s
      expect(result[1].path).toContain("2.flac"); // 200s
      expect(result[2].path).toContain("3.flac"); // 300s
    });

    it("tie-breaks equal duration by filename", () => {
      const tracks = [
        makeTrack("/music/b.flac", { duration: 100 }),
        makeTrack("/music/a.flac", { duration: 100 }),
      ];
      const result = computeNumberedTracks(tracks, "duration-asc");
      expect(result[0].path).toContain("a.flac");
      expect(result[1].path).toContain("b.flac");
    });
  });

  describe("duration-desc", () => {
    it("sorts by duration descending (longest first)", () => {
      const tracks = [
        makeTrack("/music/1.flac", { duration: 100 }),
        makeTrack("/music/3.flac", { duration: 300 }),
        makeTrack("/music/2.flac", { duration: 200 }),
      ];
      const result = computeNumberedTracks(tracks, "duration-desc");
      expect(result[0].path).toContain("3.flac"); // 300s
      expect(result[1].path).toContain("2.flac"); // 200s
      expect(result[2].path).toContain("1.flac"); // 100s
    });
  });

  // ── TrackTotal consistency ───────────────────────────────

  describe("trackTotal is set correctly", () => {
    it("sets trackTotal to the total number of tracks", () => {
      const tracks = [
        makeTrack("/music/a.flac"),
        makeTrack("/music/b.flac"),
        makeTrack("/music/c.flac"),
      ];
      const result = computeNumberedTracks(tracks, "filename-asc");
      for (const r of result) {
        expect(r.fields.trackTotal).toBe(3);
      }
    });

    it("sets trackTotal to 1 for a single track", () => {
      const result = computeNumberedTracks(
        [makeTrack("/music/solo.flac")],
        "filename-asc",
      );
      expect(result[0].fields.trackTotal).toBe(1);
    });
  });

  // ── Stable ordering across different rules ───────────────

  describe("stable ordering", () => {
    it("all rules produce exactly the same number of outputs as inputs", () => {
      const tracks = [
        makeTrack("/music/c.flac", { title: "C", trackNumber: 3, duration: 30 }),
        makeTrack("/music/a.flac", { title: "A", trackNumber: 1, duration: 10 }),
        makeTrack("/music/b.flac", { title: "B", trackNumber: 2, duration: 20 }),
      ];
      const rules: OrderingRule[] = [
        "filename-asc",
        "filename-desc",
        "title-asc",
        "title-desc",
        "existing-track-asc",
        "existing-track-desc",
        "duration-asc",
        "duration-desc",
      ];
      for (const rule of rules) {
        const result = computeNumberedTracks(tracks, rule);
        expect(result).toHaveLength(3);
      }
    });
  });
});
