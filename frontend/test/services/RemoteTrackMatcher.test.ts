/**
 * Tests for RemoteTrackMatcher service — deterministic per-file matching.
 */

import { describe, it, expect } from "vitest";
import {
  generateTitleForms,
  cleanFilenameTitle,
  normalizeDurationSeconds,
  durationsMatch,
  matchRemoteCandidateTracks,
} from "../../electron/services/RemoteTrackMatcher";
import { makeTrackCandidate } from "../../electron/handlers/candidates";

// ── Title form generation ─────────────────────────────────────────

describe("generateTitleForms", () => {
  it("generates tag title forms from tag title", async () => {
    const forms = await generateTitleForms("传奇", "");
    const tagForms = forms.filter((f) => f.source === "tag").map((f) => f.text);
    // At minimum the base normalized form
    expect(tagForms).toContain("传奇");
  });

  it("includes Simplified/Traditional Chinese variants", async () => {
    const forms = await generateTitleForms("传奇", "");
    const texts = forms.filter((f) => f.source === "tag").map((f) => f.text);
    // Should include both Simplified and Traditional forms
    expect(texts).toContain("传奇");
    expect(texts).toContain("傳奇");
  });

  it("strips trailing annotation suffixes in brackets", async () => {
    const forms = await generateTitleForms(
      "唱一遍一遍 (東風電視台 『茶館』 片頭曲)",
      "",
    );
    const texts = forms.filter((f) => f.source === "tag").map((f) => f.text);
    // Should match the base title after annotation stripping
    expect(texts).toContain("唱一遍一遍");
  });

  it("strips trailing (Live) annotation", async () => {
    const forms = await generateTitleForms("传奇 (Live)", "");
    const texts = forms.filter((f) => f.source === "tag").map((f) => f.text);
    expect(texts).toContain("传奇");
  });

  it("strips trailing [remaster] annotation", async () => {
    const forms = await generateTitleForms("Song [Remastered]", "");
    const texts = forms.filter((f) => f.source === "tag").map((f) => f.text);
    expect(texts).toContain("song");
  });

  it("does not perform romanization (Sakura ≠ さくら)", async () => {
    const sakuraForms = await generateTitleForms("Sakura", "");
    const sakuraTexts = sakuraForms
      .filter((f) => f.source === "tag")
      .map((f) => f.text);
    const kanaForms = await generateTitleForms("さくら", "");
    const kanaTexts = kanaForms
      .filter((f) => f.source === "tag")
      .map((f) => f.text);
    // No overlap between Latin and Kana normalized forms
    for (const st of sakuraTexts) {
      expect(kanaTexts).not.toContain(st);
    }
  });

  it("normalizes Unicode fullwidth punctuation", async () => {
    // Fullwidth characters should NFKC-normalize
    const forms = await generateTitleForms("ＡＢＣ　Ｄ", "");
    const texts = forms.filter((f) => f.source === "tag").map((f) => f.text);
    expect(texts).toContain("abc d");
  });

  it("generates filename-derived forms from filename", async () => {
    const forms = await generateTitleForms("", "01. Song Title.flac");
    const filenameForms = forms
      .filter((f) => f.source === "filename")
      .map((f) => f.text);
    expect(filenameForms).toContain("song title");
  });

  it("strips track number and Artist - prefix from filename", async () => {
    const forms = await generateTitleForms("", "05. 费玉清 - 变色湖长城.flac");
    const filenameForms = forms
      .filter((f) => f.source === "filename")
      .map((f) => f.text);
    expect(filenameForms).toContain("变色湖长城");
  });

  it("handles empty tag title", async () => {
    const forms = await generateTitleForms(null, "");
    expect(forms.length).toBe(0);
  });
});

// ── cleanFilenameTitle ──────────────────────────────────────────

describe("cleanFilenameTitle", () => {
  it("strips track number and extension", () => {
    expect(cleanFilenameTitle("01. Song.flac")).toBe("Song");
  });

  it("strips Artist - prefix", () => {
    expect(cleanFilenameTitle("05. 费玉清 - 变色湖长城.flac")).toBe("变色湖长城");
  });

  it("returns null for empty filename", () => {
    expect(cleanFilenameTitle("")).toBeNull();
  });
});

// ── Duration normalization ───────────────────────────────────────

describe("normalizeDurationSeconds", () => {
  it("converts MusicBrainz milliseconds to seconds", () => {
    // MusicBrainz returns values in milliseconds (>1000)
    expect(normalizeDurationSeconds(200000, "musicbrainz")).toBe(200);
    expect(normalizeDurationSeconds(245000, "musicbrainz")).toBe(245);
  });

  it("preserves seconds for non-MusicBrainz sources", () => {
    expect(normalizeDurationSeconds(200, "discogs")).toBe(200);
    expect(normalizeDurationSeconds(245.5, "local")).toBe(245.5);
  });

  it("preserves values ≤ 1000 for MusicBrainz source", () => {
    // A very short track (< 1 second) should not be divided
    expect(normalizeDurationSeconds(500, "musicbrainz")).toBe(500);
  });

  it("returns null for null/undefined/zero", () => {
    expect(normalizeDurationSeconds(null, "local")).toBeNull();
    expect(normalizeDurationSeconds(undefined, "discogs")).toBeNull();
    expect(normalizeDurationSeconds(0, "musicbrainz")).toBeNull();
  });
});

describe("durationsMatch", () => {
  it("matches close durations within 3%", () => {
    expect(durationsMatch(200, 203)).toBe(true);  // 1.5% diff
    expect(durationsMatch(200, 206)).toBe(true);  // 3% diff
  });

  it("matches durations within absolute 5s minimum", () => {
    // For short tracks, 5s is the threshold
    expect(durationsMatch(10, 14)).toBe(true);  // 4s diff < 5s
    expect(durationsMatch(10, 16)).toBe(false); // 6s diff > 5s
  });

  it("rejects far durations", () => {
    expect(durationsMatch(200, 220)).toBe(false); // 10% > 3%
  });

  it("returns false when either duration is null", () => {
    expect(durationsMatch(null, 200)).toBe(false);
    expect(durationsMatch(200, null)).toBe(false);
    expect(durationsMatch(null, null)).toBe(false);
  });
});

// ── Main matching logic ─────────────────────────────────────────

describe("matchRemoteCandidateTracks", () => {
  it("matches tracks by title alone when both exist", async () => {
    const localTracks = [
      makeTrackCandidate({ title: "Song A", trackNumber: 1 }),
      makeTrackCandidate({ title: "Song B", trackNumber: 2 }),
    ];
    const remoteTracks = [
      makeTrackCandidate({ title: "Song A", trackNumber: 1 }),
      makeTrackCandidate({ title: "Song B", trackNumber: 2 }),
    ];

    const result = await matchRemoteCandidateTracks(
      localTracks,
      [],
      remoteTracks,
      "musicbrainz",
    );

    expect(result.isFullOrderedMatch).toBe(true);
    expect(result.stats.matched).toBe(2);
    expect(result.tracks[0].title).toBe("Song A");
    expect(result.tracks[1].title).toBe("Song B");
  });

  it("matches by title + close duration", async () => {
    const localTracks = [
      makeTrackCandidate({ title: "Song A", length: 200, trackNumber: 1 }),
    ];
    const remoteTracks = [
      makeTrackCandidate({ title: "Song A", length: 203000, trackNumber: 1 }),
    ];

    const result = await matchRemoteCandidateTracks(
      localTracks,
      [],
      remoteTracks,
      "musicbrainz",
    );

    expect(result.stats.matched).toBe(1);
    expect(result.tracks[0].title).toBe("Song A");
  });

  it("rejects match when title matches but duration is far", async () => {
    const localTracks = [
      makeTrackCandidate({ title: "Song A", length: 200, trackNumber: 1 }),
    ];
    const remoteTracks = [
      makeTrackCandidate({ title: "Song A", length: 300000, trackNumber: 1 }), // 300s vs 200s = 50% diff
    ];

    const result = await matchRemoteCandidateTracks(
      localTracks,
      [],
      remoteTracks,
      "musicbrainz",
    );

    expect(result.stats.matched).toBe(0);
    expect(result.stats.skipped[0].reason).toBe("duration_mismatch");
  });

  it("does not match by duration alone (no title match)", async () => {
    const localTracks = [
      makeTrackCandidate({ title: "Song A", length: 200, trackNumber: 1 }),
    ];
    const remoteTracks = [
      makeTrackCandidate({ title: "Different Song", length: 200, trackNumber: 1 }),
    ];

    const result = await matchRemoteCandidateTracks(
      localTracks,
      [],
      remoteTracks,
      "musicbrainz",
    );

    expect(result.stats.matched).toBe(0);
  });

  it("skips duplicate remote titles when duration cannot disambiguate", async () => {
    const localTracks = [
      makeTrackCandidate({ title: "Song", length: 200, trackNumber: 1 }),
    ];
    const remoteTracks = [
      makeTrackCandidate({ title: "Song", length: 200, trackNumber: 1 }),
      makeTrackCandidate({ title: "Song", length: 201, trackNumber: 2 }),
    ];

    const result = await matchRemoteCandidateTracks(
      localTracks,
      [],
      remoteTracks,
      "musicbrainz",
    );

    // Both have close duration → ambiguous
    expect(result.stats.matched).toBe(0); // or 1 with unique duration resolution
  });

  it("preserves local title when local tag title matched", async () => {
    const localTracks = [
      makeTrackCandidate({ title: "传奇", trackNumber: 1 }),
    ];
    const remoteTracks = [
      makeTrackCandidate({ title: "传奇", trackNumber: 1 }),
    ];

    const result = await matchRemoteCandidateTracks(
      localTracks,
      [],
      remoteTracks,
      "musicbrainz",
    );

    // Local title preserved (tag title matched)
    expect(result.tracks[0].title).toBe("传奇");
  });

  it("does not write remote track numbers for subset matches", async () => {
    const localTracks = [
      makeTrackCandidate({ title: "不变的心", trackNumber: 1 }),
      makeTrackCandidate({ title: "变色的长城", trackNumber: 2 }),
    ];
    const remoteTracks = [
      makeTrackCandidate({ title: "梦驼铃", trackNumber: 1 }),
      makeTrackCandidate({ title: "一剪梅", trackNumber: 2 }),
      makeTrackCandidate({ title: "变色的长城", trackNumber: 5 }),
    ];

    const result = await matchRemoteCandidateTracks(
      localTracks,
      [],
      remoteTracks,
      "musicbrainz",
    );

    // Track 0 unmatched — preserves local
    expect(result.tracks[0].title).toBe("不变的心");
    expect(result.tracks[0].trackNumber).toBe(1);

    // Track 1 matched but not full ordered — local track number preserved
    expect(result.tracks[1].title).toBe("变色的长城");
    expect(result.tracks[1].trackNumber).toBe(2);
  });

  it("handles empty local tracks gracefully", async () => {
    const result = await matchRemoteCandidateTracks(
      [],
      [],
      [makeTrackCandidate({ title: "Song" })],
      "musicbrainz",
    );

    expect(result.tracks).toEqual([]);
    expect(result.stats.local).toBe(0);
  });

  it("handles empty remote tracks gracefully", async () => {
    const localTracks = [makeTrackCandidate({ title: "Song" })];
    const result = await matchRemoteCandidateTracks(
      localTracks,
      [],
      [],
      "musicbrainz",
    );

    expect(result.tracks).toHaveLength(1);
    expect(result.stats.matched).toBe(0);
  });

  it("allows remote artist/artists when local artist is blank", async () => {
    const localTracks = [
      makeTrackCandidate({ title: "Song", artist: null, artists: [] }),
    ];
    const remoteTracks = [
      makeTrackCandidate({
        title: "Song",
        artist: "Remote Singer",
        artists: ["Remote Singer"],
      }),
    ];

    const result = await matchRemoteCandidateTracks(
      localTracks,
      [],
      remoteTracks,
      "musicbrainz",
    );

    expect(result.tracks[0].artist).toBe("Remote Singer");
    expect(result.tracks[0].artists).toEqual(["Remote Singer"]);
  });

  it("does NOT overwrite non-empty local artist", async () => {
    const localTracks = [
      makeTrackCandidate({
        title: "Song",
        artist: "Local Singer",
        artists: ["Local Singer"],
      }),
    ];
    const remoteTracks = [
      makeTrackCandidate({
        title: "Song",
        artist: "Remote Singer",
        artists: ["Remote Singer"],
      }),
    ];

    const result = await matchRemoteCandidateTracks(
      localTracks,
      [],
      remoteTracks,
      "musicbrainz",
    );

    expect(result.tracks[0].artist).toBe("Local Singer");
    expect(result.tracks[0].artists).toEqual(["Local Singer"]);
  });

  it("writes musicbrainzTrackId for matched tracks", async () => {
    const localTracks = [
      makeTrackCandidate({ title: "Song" }),
    ];
    const remoteTracks = [
      makeTrackCandidate({
        title: "Song",
        musicbrainzTrackId: "mbid-12345",
      }),
    ];

    const result = await matchRemoteCandidateTracks(
      localTracks,
      [],
      remoteTracks,
      "musicbrainz",
    );

    expect(result.tracks[0].musicbrainzTrackId).toBe("mbid-12345");
  });

  it("writes cleaned filename title when only filename form matched", async () => {
    const localTracks = [
      makeTrackCandidate({
        title: "Wrong Title",  // tag title doesn't match remote
        trackNumber: 1,
      }),
    ];
    const filenames = ["01. Song A.flac"];  // filename stem matches remote

    const result = await matchRemoteCandidateTracks(
      localTracks,
      filenames,
      [makeTrackCandidate({ title: "Song A" })],
      "musicbrainz",
    );

    // Tag title ("Wrong Title") doesn't match remote ("Song A")
    // Filename form matches → write cleaned filename title
    expect(result.tracks[0].title).toBe("Song A");
  });
});

// ── SC/TC matching ───────────────────────────────────────────────

describe("matchRemoteCandidateTracks — SC/TC matching", () => {
  it("matches Simplified vs Traditional Chinese (传奇 ↔ 傳奇)", async () => {
    const localTracks = [
      makeTrackCandidate({ title: "传奇", trackNumber: 1 }),
    ];
    const remoteTracks = [
      makeTrackCandidate({ title: "傳奇", trackNumber: 1 }),
    ];

    const result = await matchRemoteCandidateTracks(
      localTracks,
      [],
      remoteTracks,
      "musicbrainz",
    );

    expect(result.stats.matched).toBe(1);
    expect(result.tracks[0].title).toBe("传奇"); // local preserved
  });

  it("matches annotated title to base title via stripping", async () => {
    const localTracks = [
      makeTrackCandidate({
        title: "唱一遍一遍",
        trackNumber: 1,
      }),
    ];
    const remoteTracks = [
      makeTrackCandidate({
        title: "唱一遍一遍 (東風電視台 『茶館』 片頭曲)",
        trackNumber: 1,
      }),
    ];

    const result = await matchRemoteCandidateTracks(
      localTracks,
      [],
      remoteTracks,
      "musicbrainz",
    );

    expect(result.stats.matched).toBe(1);
    expect(result.tracks[0].title).toBe("唱一遍一遍");
  });
});

// ── Source-specific duration handling ─────────────────────────

describe("matchRemoteCandidateTracks — source-specific durations", () => {
  it("handles MusicBrainz millisecond durations", async () => {
    const localTracks = [
      makeTrackCandidate({
        title: "Song",
        length: 200,        // seconds
      }),
    ];
    const remoteTracks = [
      makeTrackCandidate({
        title: "Song",
        length: 203000,     // MusicBrainz returns milliseconds
      }),
    ];

    const result = await matchRemoteCandidateTracks(
      localTracks,
      [],
      remoteTracks,
      "musicbrainz",
    );

    // 200s local vs 203s remote = 1.5% diff → match
    expect(result.stats.matched).toBe(1);
  });

  it("does not match by duration alone without title form match", async () => {
    const localTracks = [
      makeTrackCandidate({
        title: "Song A",
        length: 200,
      }),
    ];
    const remoteTracks = [
      makeTrackCandidate({
        title: "Song B",
        length: 200,
      }),
    ];

    const result = await matchRemoteCandidateTracks(
      localTracks,
      [],
      remoteTracks,
      "musicbrainz",
    );

    expect(result.stats.matched).toBe(0);
  });
});
