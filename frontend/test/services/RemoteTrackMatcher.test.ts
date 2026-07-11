/**
 * Tests for RemoteTrackMatcher service — deterministic per-file matching.
 */

import { describe, it, expect } from "vitest";
import {
  generateTitleForms,
  cleanFilenameTitle,
  normalizeDurationSeconds,
  durationsMatch,
  shouldReplacePollutedTitleWithApiTitle,
  replacementTitleForPollutedTitle,
  isPlaceholderTitle,
  matchRemoteCandidateTracks,
  scoreRemoteTrackTitleCoverage,
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

  it("strips known artist suffix from tag title", async () => {
    const forms = await generateTitleForms("想念-林宥嘉", "", ["林宥嘉"]);
    const tagForms = forms.filter((f) => f.source === "tag").map((f) => f.text);
    // Should include both original and suffix-stripped forms
    // Note: hyphen is normalized to space by stripPunctuationAndSymbols
    expect(tagForms).toContain("想念 林宥嘉"); // original normalized
    expect(tagForms).toContain("想念"); // suffix-stripped
  });

  it("strips artist suffix with different separators", async () => {
    const forms = await generateTitleForms("不换–林宥嘉", "", ["林宥嘉"]);
    const tagForms = forms.filter((f) => f.source === "tag").map((f) => f.text);
    expect(tagForms).toContain("不换");
  });

  it("does not strip when artist is not known", async () => {
    const forms = await generateTitleForms("想念-林宥嘉", "", ["周杰伦"]);
    const tagForms = forms.filter((f) => f.source === "tag").map((f) => f.text);
    // Should NOT include stripped form since "林宥嘉" is not a known artist
    expect(tagForms).not.toContain("想念");
    expect(tagForms).toContain("想念 林宥嘉");
  });

  it("does not strip when suffix does not match any known artist", async () => {
    // "Remix" is NOT an annotation keyword, so it stays in the title
    const forms = await generateTitleForms("Song - Remix", "", ["Artist"]);
    const tagForms = forms.filter((f) => f.source === "tag").map((f) => f.text);
    // Should contain the full normalized form, NOT a stripped "song" form
    expect(tagForms).toContain("song remix");
    expect(tagForms).not.toContain("song");
  });
});

describe("scoreRemoteTrackTitleCoverage", () => {
  it("counts unique title containment matches without preparing write output", async () => {
    const localTracks = [
      makeTrackCandidate({ title: "小白很乖(那些女孩教我的事)(24bit-48Hz)", trackNumber: 6 }),
      makeTrackCandidate({ title: "漂亮", trackNumber: 7 }),
    ];
    const remoteTracks = [
      makeTrackCandidate({ title: "小白很乖", trackNumber: 6 }),
      makeTrackCandidate({ title: "漂亮", trackNumber: 7 }),
    ];

    const score = await scoreRemoteTrackTitleCoverage(
      localTracks,
      [],
      remoteTracks,
      "musicbrainz",
    );

    expect(score.matched).toBe(2);
    expect(score.durationMatched).toBe(0);
    expect(score.coverage).toBe(1);
  });

  it("does not count ambiguous title containment toward release coverage", async () => {
    const localTracks = [
      makeTrackCandidate({ title: "Intro", trackNumber: 1 }),
    ];
    const remoteTracks = [
      makeTrackCandidate({ title: "Intro", trackNumber: 1 }),
      makeTrackCandidate({ title: "Intro Reprise", trackNumber: 2 }),
    ];

    const score = await scoreRemoteTrackTitleCoverage(
      localTracks,
      [],
      remoteTracks,
      "musicbrainz",
    );

    expect(score.matched).toBe(0);
    expect(score.skipped[0].reason).toBe("duplicate_ambiguous");
  });

  it("counts an LLM-cleaned filename title as alternate local evidence", async () => {
    const score = await scoreRemoteTrackTitleCoverage(
      [makeTrackCandidate({ title: "Unknown 06", trackNumber: 6 })],
      ["unstructured-file-name.flac"],
      [makeTrackCandidate({ title: "小白很乖", trackNumber: 6 })],
      "musicbrainz",
      { alternateTrackTitles: ["小白很乖"] },
    );

    expect(score.matched).toBe(1);
    expect(score.coverage).toBe(1);
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

  it("strips no-space Artist-Title prefix when artist evidence matches", () => {
    expect(cleanFilenameTitle("费玉清-不变的心.flac", ["费玉清"])).toBe("不变的心");
  });

  it("keeps hyphenated titles when artist evidence does not match", () => {
    expect(cleanFilenameTitle("费玉清-不变的心.flac", ["邓丽君"])).toBe("费玉清-不变的心");
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

describe("shouldReplacePollutedTitleWithApiTitle", () => {
  it("replaces when the API title is contained in a suffix-polluted local title", () => {
    expect(shouldReplacePollutedTitleWithApiTitle("微光(亚特兰提斯)(24bit-48Hz)", "微光")).toBe(true);
    expect(shouldReplacePollutedTitleWithApiTitle("Revolution(飞儿乐团)(24bit-48Hz)", "Revolution")).toBe(true);
  });

  it("replaces after a leading artist display prefix", () => {
    expect(shouldReplacePollutedTitleWithApiTitle("F.I.R飞儿乐团 - Blue Doors Ahead(爱歌姬)(16bit-44.1Hz)", "Blue Doors Ahead")).toBe(true);
  });

  it("does not replace punctuation-only differences", () => {
    expect(shouldReplacePollutedTitleWithApiTitle("Revolution", "Revolution")).toBe(false);
    expect(shouldReplacePollutedTitleWithApiTitle("Revolution!", "Revolution")).toBe(false);
  });

  it("does not replace unrelated titles", () => {
    expect(shouldReplacePollutedTitleWithApiTitle("错误标题(亚特兰提斯)(24bit-48Hz)", "微光")).toBe(false);
  });

  it("does not replace when the API title only appears in the suffix", () => {
    expect(shouldReplacePollutedTitleWithApiTitle("Say Hello(亚特兰提斯)(24bit-48Hz)", "亚特兰提斯")).toBe(false);
  });

  it("does not replace meaningful version qualifiers", () => {
    expect(shouldReplacePollutedTitleWithApiTitle("Song (Live)", "Song")).toBe(false);
    expect(shouldReplacePollutedTitleWithApiTitle("Song - 伴奏", "Song")).toBe(false);
  });
});

describe("replacementTitleForPollutedTitle", () => {
  it("preserves local prefix casing when the API title only differs by case", () => {
    expect(replacementTitleForPollutedTitle("I Can't Go On(无限)(24bit-48Hz)", "I can't go on")).toBe("I Can't Go On");
  });

  it("uses the API title when the match requires a Simplified/Traditional variant", () => {
    expect(replacementTitleForPollutedTitle("让爱重生(亚特兰提斯)(24bit-48Hz)", "讓愛重生", ["让爱重生"])).toBe("讓愛重生");
  });
});

describe("isPlaceholderTitle", () => {
  it("detects generic placeholder titles", () => {
    expect(isPlaceholderTitle("Track 01")).toBe(true);
    expect(isPlaceholderTitle("Track 1")).toBe(true);
    expect(isPlaceholderTitle("Track 08")).toBe(true);
    expect(isPlaceholderTitle("track 5")).toBe(true);
    expect(isPlaceholderTitle("Track 123")).toBe(true);
  });

  it("rejects non-placeholder titles", () => {
    expect(isPlaceholderTitle("Song Title")).toBe(false);
    expect(isPlaceholderTitle("Track A")).toBe(false);
    expect(isPlaceholderTitle("My Track")).toBe(false);
    expect(isPlaceholderTitle(null)).toBe(false);
    expect(isPlaceholderTitle(undefined)).toBe(false);
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

  it("trusts remote artist when a MusicBrainz title match is strong", async () => {
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

    expect(result.matchEvidence).toEqual(["tag-title"]);
    expect(result.tracks[0].artist).toBe("Remote Singer");
    expect(result.tracks[0].artists).toEqual(["Remote Singer"]);
  });

  it("trusts remote duet artist when matched local artist is a placeholder", async () => {
    const localTracks = [
      makeTrackCandidate({
        title: "身边",
        artist: "[momishi.com]",
        artists: ["[momishi.com]"],
      }),
    ];
    const remoteTracks = [
      makeTrackCandidate({
        title: "身边",
        artist: "品冠 vs 光良",
        artists: ["品冠", "光良"],
      }),
    ];

    const result = await matchRemoteCandidateTracks(
      localTracks,
      [],
      remoteTracks,
      "musicbrainz",
      { artistHints: ["品冠"] },
    );

    expect(result.matchEvidence).toEqual(["tag-title"]);
    expect(result.tracks[0].artist).toBe("品冠 vs 光良");
    expect(result.tracks[0].artists).toEqual(["品冠", "光良"]);
  });

  it("trusts remote title and artist when MusicBrainz track ID matches", async () => {
    const localTracks = [
      makeTrackCandidate({
        title: "06.小白很乖",
        artist: "[momishi.com]",
        artists: ["[momishi.com]"],
        musicbrainzTrackId: "96fd68c2-669e-4906-8d8e-041e48e3f78e",
      }),
    ];
    const remoteTracks = [
      makeTrackCandidate({
        title: "小白很乖",
        artist: "品冠",
        artists: ["品冠"],
        musicbrainzTrackId: "96fd68c2-669e-4906-8d8e-041e48e3f78e",
      }),
    ];

    const result = await matchRemoteCandidateTracks(
      localTracks,
      [],
      remoteTracks,
      "musicbrainz",
    );

    expect(result.matchEvidence).toEqual(["musicbrainz-track-id"]);
    expect(result.tracks[0].title).toBe("小白很乖");
    expect(result.tracks[0].artist).toBe("品冠");
    expect(result.tracks[0].artists).toEqual(["品冠"]);
  });

  it("matches a MusicBrainz recording title but writes the release-track title", async () => {
    const localTracks = [
      makeTrackCandidate({
        title: "Top of the World / 我站上全世界的屋顶",
        artist: "[momishi.com]",
        artists: ["[momishi.com]"],
        length: 232.7,
      }),
    ];
    const remoteTracks = [
      makeTrackCandidate({
        title: "站在世界之巔",
        matchTitles: ["Top of the World（我站上全世界的屋頂）"],
        artist: "品冠",
        artists: ["品冠"],
        musicbrainzTrackId: "df2eeddb-4c12-432a-a3b3-c3b170222a15",
        length: 232693,
      }),
    ];

    const result = await matchRemoteCandidateTracks(
      localTracks,
      ["品冠 - 12.Top of the World - 我站上全世界的屋顶.flac"],
      remoteTracks,
      "musicbrainz",
      { artistHints: ["品冠"] },
    );

    expect(result.matchEvidence).toEqual(["tag-title"]);
    expect(result.tracks[0].title).toBe("站在世界之巔");
    expect(result.tracks[0].artist).toBe("品冠");
    expect(result.tracks[0].musicbrainzTrackId).toBe(
      "df2eeddb-4c12-432a-a3b3-c3b170222a15",
    );
  });

  it("matches a unique bilingual component when translated word order differs", async () => {
    const localTracks = [
      makeTrackCandidate({
        title: "Have I Told You Lately / 最近我有没有跟妳说",
        artist: "[momishi.com]",
        artists: ["[momishi.com]"],
        length: 269.9,
      }),
    ];
    const remoteTracks = [
      makeTrackCandidate({
        title: "最近我有沒有告訴你",
        matchTitles: ["Have I Told You Lately（我最近有沒有跟妳說）"],
        artist: "品冠",
        artists: ["品冠"],
        musicbrainzTrackId: "0fd2a474-6e91-4271-84c9-334fef23c47a",
        length: 269933,
      }),
    ];

    const result = await matchRemoteCandidateTracks(
      localTracks,
      ["品冠 - 06.Have I Told You Lately - 最近我有没有跟妳说.flac"],
      remoteTracks,
      "musicbrainz",
      { artistHints: ["品冠"] },
    );

    expect(result.matchEvidence).toEqual(["tag-title"]);
    expect(result.tracks[0].title).toBe("最近我有沒有告訴你");
    expect(result.tracks[0].artist).toBe("品冠");
    expect(result.tracks[0].musicbrainzTrackId).toBe(
      "0fd2a474-6e91-4271-84c9-334fef23c47a",
    );
  });

  it("rejects a bilingual component shared by multiple remote tracks", async () => {
    const result = await matchRemoteCandidateTracks(
      [makeTrackCandidate({ title: "Shared English / 本地翻译" })],
      [],
      [
        makeTrackCandidate({ title: "远端甲", matchTitles: ["Shared English（译名甲）"] }),
        makeTrackCandidate({ title: "远端乙", matchTitles: ["Shared English（译名乙）"] }),
      ],
      "musicbrainz",
    );

    expect(result.stats.matched).toBe(0);
    expect(result.matchEvidence).toEqual([null]);
    expect(result.stats.skipped.some((skip) => skip.reason === "duplicate_ambiguous")).toBe(true);
  });

  it("updates local artist when remote enriches with featured artist", async () => {
    const localTracks = [
      makeTrackCandidate({
        title: "加油!",
        artist: "林俊傑",
        artists: ["林俊傑"],
      }),
    ];
    const remoteTracks = [
      makeTrackCandidate({
        title: "加油!",
        artist: "林俊傑 feat. MC HotDog",
        artists: ["林俊傑", "MC HotDog"],
      }),
    ];

    const result = await matchRemoteCandidateTracks(
      localTracks,
      [],
      remoteTracks,
      "musicbrainz",
    );

    expect(result.tracks[0].artist).toBe("林俊傑 feat. MC HotDog");
    expect(result.tracks[0].artists).toEqual(["林俊傑", "MC HotDog"]);
  });

  it("does NOT overwrite local artist when only positional fallback matched", async () => {
    const localTracks = [
      makeTrackCandidate({
        title: "Local Song",
        artist: "A",
        artists: ["A"],
      }),
    ];
    const remoteTracks = [
      makeTrackCandidate({
        title: "Remote Song",
        artist: "Adele feat. X",
        artists: ["Adele", "X"],
      }),
      makeTrackCandidate({
        title: "Another Remote Song",
        artist: "Adele",
        artists: ["Adele"],
      }),
    ];

    const result = await matchRemoteCandidateTracks(
      [
        ...localTracks,
        makeTrackCandidate({ title: "Other Local Song", artist: "B", artists: ["B"] }),
      ],
      [],
      remoteTracks,
      "musicbrainz",
    );

    expect(result.matchEvidence).toEqual(["position", "position"]);
    expect(result.tracks[0].artist).toBe("A");
    expect(result.tracks[0].artists).toEqual(["A"]);
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

  it("matches no-space Chinese Artist-Title filenames by known artist", async () => {
    const localTracks = [
      makeTrackCandidate({
        title: "Wrong Title",
        artist: "费玉清",
        artists: ["费玉清"],
      }),
    ];
    const filenames = ["费玉清-不变的心.flac"];

    const result = await matchRemoteCandidateTracks(
      localTracks,
      filenames,
      [makeTrackCandidate({ title: "不变的心" })],
      "musicbrainz",
      { artistHints: ["费玉清"] },
    );

    expect(result.stats.matched).toBe(1);
    expect(result.tracks[0].title).toBe("不变的心");
  });

  it("uses matched MusicBrainz API title when local title contains extra suffix pollution", async () => {
    const localTracks = [
      makeTrackCandidate({ title: "微光(亚特兰提斯)(24bit-48Hz)", trackNumber: 4 }),
      makeTrackCandidate({ title: "讓我們一起微笑吧", trackNumber: 5 }),
    ];
    const remoteTracks = [
      makeTrackCandidate({ title: "微光", trackNumber: 4 }),
      makeTrackCandidate({ title: "讓我們一起微笑吧", trackNumber: 5 }),
    ];

    const result = await matchRemoteCandidateTracks(
      localTracks,
      [],
      remoteTracks,
      "musicbrainz",
    );

    expect(result.isFullOrderedMatch).toBe(true);
    expect(result.tracks[0].title).toBe("微光");
    expect(result.tracks[1].title).toBe("讓我們一起微笑吧");
  });

  it("uses API title when polluted local title only contains a Simplified/Traditional variant", async () => {
    const localTracks = [
      makeTrackCandidate({ title: "让爱重生(亚特兰提斯)(24bit-48Hz)", trackNumber: 10 }),
      makeTrackCandidate({ title: "Say Hello", trackNumber: 1 }),
    ];
    const remoteTracks = [
      makeTrackCandidate({ title: "讓愛重生", trackNumber: 10 }),
      makeTrackCandidate({ title: "Say Hello", trackNumber: 1 }),
    ];

    const result = await matchRemoteCandidateTracks(
      localTracks,
      [],
      remoteTracks,
      "musicbrainz",
    );

    expect(result.isFullOrderedMatch).toBe(true);
    expect(result.tracks[0].title).toBe("讓愛重生");
  });

  it("uses matched Discogs API title when local title contains extra suffix pollution", async () => {
    const localTracks = [
      makeTrackCandidate({ title: "Revolution(飞儿乐团)(24bit-48Hz)", trackNumber: 1 }),
      makeTrackCandidate({ title: "Fly Away", trackNumber: 2 }),
    ];
    const remoteTracks = [
      makeTrackCandidate({ title: "Revolution", trackNumber: 1 }),
      makeTrackCandidate({ title: "Fly Away", trackNumber: 2 }),
    ];

    const result = await matchRemoteCandidateTracks(
      localTracks,
      [],
      remoteTracks,
      "discogs",
    );

    expect(result.isFullOrderedMatch).toBe(true);
    expect(result.tracks[0].title).toBe("Revolution");
    expect(result.tracks[1].title).toBe("Fly Away");
  });

  it("preserves local title casing when the matched API title differs only by case", async () => {
    const localTracks = [
      makeTrackCandidate({ title: "I Can't Go On(无限)(24bit-48Hz)", trackNumber: 1 }),
      makeTrackCandidate({ title: "Love3", trackNumber: 2 }),
    ];
    const remoteTracks = [
      makeTrackCandidate({ title: "I can't go on", trackNumber: 1 }),
      makeTrackCandidate({ title: "Love3", trackNumber: 2 }),
    ];

    const result = await matchRemoteCandidateTracks(
      localTracks,
      [],
      remoteTracks,
      "musicbrainz",
    );

    expect(result.isFullOrderedMatch).toBe(true);
    expect(result.tracks[0].title).toBe("I Can't Go On");
  });

  it("does not use API title when position matches but title containment does not", async () => {
    const localTracks = [
      makeTrackCandidate({ title: "错误标题(亚特兰提斯)(24bit-48Hz)", trackNumber: 4 }),
      makeTrackCandidate({ title: "另一首", trackNumber: 5 }),
    ];
    const remoteTracks = [
      makeTrackCandidate({ title: "微光", trackNumber: 4 }),
      makeTrackCandidate({ title: "不相干", trackNumber: 5 }),
    ];

    const result = await matchRemoteCandidateTracks(
      localTracks,
      [],
      remoteTracks,
      "musicbrainz",
    );

    expect(result.isFullOrderedMatch).toBe(true);
    expect(result.tracks[0].title).toBe("错误标题(亚特兰提斯)(24bit-48Hz)");
  });
});

// ── SC/TC matching ───────────────────────────────────────────────

describe("matchRemoteCandidateTracks — SC/TC matching", () => {
  it("matches Simplified vs Traditional Chinese (传奇 ↔ 傳奇)", async () => {
    const localTracks = [
      makeTrackCandidate({ title: "传奇", artist: "本地歌手", artists: ["本地歌手"], trackNumber: 1 }),
    ];
    const remoteTracks = [
      makeTrackCandidate({ title: "傳奇", artist: "遠端歌手", artists: ["遠端歌手"], trackNumber: 1 }),
    ];

    const result = await matchRemoteCandidateTracks(
      localTracks,
      [],
      remoteTracks,
      "musicbrainz",
    );

    expect(result.stats.matched).toBe(1);
    expect(result.matchEvidence).toEqual(["tag-title"]);
    expect(result.tracks[0].title).toBe("传奇"); // local preserved
    expect(result.tracks[0].artist).toBe("遠端歌手");
    expect(result.tracks[0].artists).toEqual(["遠端歌手"]);
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

  it("matches gendered 妳 to generic 你 (U+59B3)", async () => {
    // Real-world: MusicBrainz uses 妳 (female-you) while local files use 你 (generic-you)
    // e.g. 品冠「疼妳的責任」on MusicBrainz vs local "疼你的责任"
    const localTracks = [
      makeTrackCandidate({ title: "疼你的责任", trackNumber: 1 }),
      makeTrackCandidate({ title: "陪你一起老", trackNumber: 2 }),
    ];
    const remoteTracks = [
      makeTrackCandidate({ title: "疼妳的責任", trackNumber: 1, length: 264093 }),
      makeTrackCandidate({ title: "陪妳一起老", trackNumber: 2, length: 333000 }),
    ];

    const result = await matchRemoteCandidateTracks(
      localTracks,
      [],
      remoteTracks,
      "musicbrainz",
      { artistHints: ["品冠"] },
    );

    expect(result.stats.matched).toBe(2);
    expect(result.isFullOrderedMatch).toBe(true);
    expect(result.tracks[0].title).toBe("疼你的责任");
    expect(result.tracks[1].title).toBe("陪你一起老");
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

describe("matchRemoteCandidateTracks — positional fallback", () => {
  it("falls back to positional matching when 0 title matches but track counts are equal", async () => {
    // Local tracks with Chinese titles, remote tracks with English titles
    // (e.g. MusicBrainz for a Chinese album — no title overlap)
    const localTracks = [
      makeTrackCandidate({ title: "飞行器的执行周期", trackNumber: 1 }),
      makeTrackCandidate({ title: "不明下落", trackNumber: 2 }),
      makeTrackCandidate({ title: "在什么时候", trackNumber: 3 }),
    ];
    const remoteTracks = [
      makeTrackCandidate({ title: "The Lonely Planet of the Flying Object", trackNumber: 1 }),
      makeTrackCandidate({ title: "Missing", trackNumber: 2 }),
      makeTrackCandidate({ title: "At What Time", trackNumber: 3 }),
    ];

    const result = await matchRemoteCandidateTracks(
      localTracks,
      [],
      remoteTracks,
      "musicbrainz",
    );

    // Should have full ordered match via positional fallback
    expect(result.isFullOrderedMatch).toBe(true);
    expect(result.stats.matched).toBe(3);
    expect(result.stats.skipped.length).toBe(0);

    // Local titles preserved (not overwritten by remote)
    expect(result.tracks[0].title).toBe("飞行器的执行周期");
    expect(result.tracks[1].title).toBe("不明下落");
    expect(result.tracks[2].title).toBe("在什么时候");
  });

  it("falls back to positional matching when all tracks fail with no_title_match", async () => {
    const localTracks = [
      makeTrackCandidate({ title: "Song A", trackNumber: 1, length: 200 }),
      makeTrackCandidate({ title: "Song B", trackNumber: 2, length: 210 }),
    ];
    const remoteTracks = [
      makeTrackCandidate({ title: "Different A", trackNumber: 1, length: 200 }),
      makeTrackCandidate({ title: "Different B", trackNumber: 2, length: 210 }),
    ];

    const result = await matchRemoteCandidateTracks(
      localTracks,
      [],
      remoteTracks,
      "musicbrainz",
    );

    expect(result.isFullOrderedMatch).toBe(true);
    expect(result.stats.matched).toBe(2);
    expect(result.stats.skipped.length).toBe(0);

    // Local titles preserved
    expect(result.tracks[0].title).toBe("Song A");
    expect(result.tracks[1].title).toBe("Song B");
  });

  it("does not activate positional fallback when track counts differ", async () => {
    const localTracks = [
      makeTrackCandidate({ title: "Song A", trackNumber: 1 }),
    ];
    const remoteTracks = [
      makeTrackCandidate({ title: "Different A", trackNumber: 1 }),
      makeTrackCandidate({ title: "Different B", trackNumber: 2 }),
    ];

    const result = await matchRemoteCandidateTracks(
      localTracks,
      [],
      remoteTracks,
      "musicbrainz",
    );

    // No fallback — still 0 matches
    expect(result.isFullOrderedMatch).toBe(false);
    expect(result.stats.matched).toBe(0);
  });

  it("does not activate positional fallback when some title matches already succeeded", async () => {
    const localTracks = [
      makeTrackCandidate({ title: "Song A", trackNumber: 1 }),
      makeTrackCandidate({ title: "Unique B", trackNumber: 2 }),
    ];
    const remoteTracks = [
      makeTrackCandidate({ title: "Song A", trackNumber: 1 }),
      makeTrackCandidate({ title: "Song C", trackNumber: 2 }),
    ];

    const result = await matchRemoteCandidateTracks(
      localTracks,
      [],
      remoteTracks,
      "musicbrainz",
    );

    // First track matched by title, second fails — should NOT use positional fallback
    // because some matches succeeded (risking misalignment)
    expect(result.stats.matched).toBe(1);
    expect(result.isFullOrderedMatch).toBe(false);
  });

  it("applies remote track/disc numbers when positional fallback succeeds", async () => {
    const localTracks = [
      makeTrackCandidate({ title: "本地标题1", artist: "本地歌手", artists: ["本地歌手"], trackNumber: null, discNumber: null }),
      makeTrackCandidate({ title: "本地标题2", artist: null, artists: [], trackNumber: null, discNumber: null }),
    ];
    const remoteTracks = [
      makeTrackCandidate({ title: "Remote Title 1", artist: "Remote Singer 1", artists: ["Remote Singer 1"], trackNumber: 1, discNumber: 1 }),
      makeTrackCandidate({ title: "Remote Title 2", artist: "Remote Singer 2", artists: ["Remote Singer 2"], trackNumber: 2, discNumber: 1 }),
    ];

    const result = await matchRemoteCandidateTracks(
      localTracks,
      [],
      remoteTracks,
      "musicbrainz",
    );

    // isFullOrderedMatch = true enables remote track/disc field application
    expect(result.isFullOrderedMatch).toBe(true);
    expect(result.stats.matched).toBe(2);
    expect(result.matchEvidence).toEqual(["position", "position"]);

    // Remote track/disc numbers applied via the full-ordered-match path
    expect(result.tracks[0].trackNumber).toBe(1);
    expect(result.tracks[0].discNumber).toBe(1);
    expect(result.tracks[1].trackNumber).toBe(2);
    expect(result.tracks[1].discNumber).toBe(1);

    // Local titles preserved
    expect(result.tracks[0].title).toBe("本地标题1");
    expect(result.tracks[1].title).toBe("本地标题2");
    // Position-only evidence preserves non-empty local artists but can fill blanks.
    expect(result.tracks[0].artist).toBe("本地歌手");
    expect(result.tracks[0].artists).toEqual(["本地歌手"]);
    expect(result.tracks[1].artist).toBe("Remote Singer 2");
    expect(result.tracks[1].artists).toEqual(["Remote Singer 2"]);
  });

  it("matches Chinese titles with artist suffix against bilingual remote titles", async () => {
    // Real-world scenario: local files have "TITLE-ARTIST" pattern
    // MusicBrainz has bilingual titles like "想念 I Miss You"
    const localTracks = [
      makeTrackCandidate({ title: "Fly My Way", artist: "林宥嘉", trackNumber: 1 }),
      makeTrackCandidate({ title: "不换-林宥嘉", artist: "林宥嘉", trackNumber: 2 }),
      makeTrackCandidate({ title: "想念-林宥嘉", artist: "林宥嘉", trackNumber: 3 }),
      makeTrackCandidate({ title: "晚安-林宥嘉", artist: "林宥嘉", trackNumber: 8 }),
    ];
    const filenames = [
      "Fly My Way-林宥嘉.flac",
      "不换-林宥嘉.flac",
      "想念-林宥嘉.flac",
      "晚安-林宥嘉.flac",
    ];
    const remoteTracks = [
      makeTrackCandidate({ title: "Fly My Way", trackNumber: 11 }),
      makeTrackCandidate({ title: "不換 Going On My Way", trackNumber: 6 }),
      makeTrackCandidate({ title: "想念 I Miss You", trackNumber: 9 }),
      makeTrackCandidate({ title: "晚安 Good Night", trackNumber: 10 }),
    ];

    const result = await matchRemoteCandidateTracks(
      localTracks,
      filenames,
      remoteTracks,
      "musicbrainz",
      { artistHints: ["林宥嘉"] },
    );

    // All 4 tracks should match
    expect(result.stats.matched).toBe(4);
    expect(result.stats.skipped).toHaveLength(0);

    // Verify specific matches
    // Exact match: local title preserved
    expect(result.tracks[0].title).toBe("Fly My Way");
    // CJK prefix match via artist suffix stripping: local title preserved
    // (the stripped form "不换" matched via CJK prefix, but pollution detection
    // didn't fire because the full normalized form didn't match)
    expect(result.tracks[1].title).toBe("不换-林宥嘉");
    // CJK prefix match with pollution detection: remote title replaces local
    expect(result.tracks[2].title).toBe("想念 I Miss You");
    expect(result.tracks[3].title).toBe("晚安 Good Night");

    // All tracks have MusicBrainz track IDs
    expect(result.tracks[0].musicbrainzTrackId).toBeDefined();
    expect(result.tracks[1].musicbrainzTrackId).toBeDefined();
    expect(result.tracks[2].musicbrainzTrackId).toBeDefined();
    expect(result.tracks[3].musicbrainzTrackId).toBeDefined();
  });

  it("replaces placeholder titles like 'Track 01' with remote titles", async () => {
    // Real-world scenario: files have generic placeholder titles
    const localTracks = [
      makeTrackCandidate({ title: "Track 01", trackNumber: 1 }),
      makeTrackCandidate({ title: "Track 02", trackNumber: 2 }),
      makeTrackCandidate({ title: "Track 03", trackNumber: 3 }),
    ];
    const remoteTracks = [
      makeTrackCandidate({ title: "分分鐘需要你", trackNumber: 1 }),
      makeTrackCandidate({ title: "似夢迷離", trackNumber: 2 }),
      makeTrackCandidate({ title: "每一個日落", trackNumber: 3 }),
    ];

    const result = await matchRemoteCandidateTracks(
      localTracks,
      [],
      remoteTracks,
      "discogs",
    );

    // Positional fallback should match all tracks
    expect(result.stats.matched).toBe(3);
    expect(result.isFullOrderedMatch).toBe(true);

    // Placeholder titles should be replaced with remote titles
    expect(result.tracks[0].title).toBe("分分鐘需要你");
    expect(result.tracks[1].title).toBe("似夢迷離");
    expect(result.tracks[2].title).toBe("每一個日落");
  });
});
