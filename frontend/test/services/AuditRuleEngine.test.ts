// @vitest-environment node
import { describe, expect, it } from "vitest";
import {
  buildDeterministicAuditFindings,
  buildLlmReviewTargets,
} from "../../electron/services/AuditRuleEngine";

const baseTrack = {
  title: "Song",
  artist: "Artist",
  artists: ["Artist"],
  album: "Album",
  albumArtist: "Artist",
  albumArtists: ["Artist"],
  year: "2020",
  genre: "Pop",
  trackNumber: 1,
  trackTotal: 2,
  discNumber: null,
  discTotal: null,
};

describe("AuditRuleEngine", () => {
  it("marks clear core tag mismatches as deterministic auto-fixes", () => {
    const findings = buildDeterministicAuditFindings("Artist", "2020 - Album", [
      {
        ...baseTrack,
        title: "Wrong",
        artist: "Wrong Artist",
        artists: [],
        album: "Wrong Album",
        albumArtist: "Wrong Artist",
        albumArtists: ["Wrong Artist"],
        year: "2019",
        trackNumber: 9,
        trackTotal: null,
        discNumber: null,
      },
      { ...baseTrack, title: "Second Song", trackNumber: 2 },
    ], [
      "01. A & B - Song.flac",
      "02. Second Song.flac",
    ]);

    expect(findings).toEqual(expect.arrayContaining([
      expect.objectContaining({
        index: 0,
        field: "title",
        source: "deterministic",
        autoFixEligible: true,
        corrected: { title: "Song" },
      }),
      expect.objectContaining({
        index: 0,
        field: "artist",
        autoFixEligible: true,
        corrected: { artist: "A & B" },
      }),
      expect.objectContaining({
        index: 0,
        field: "artists",
        autoFixEligible: true,
        corrected: { artists: ["A", "B"] },
      }),
      expect.objectContaining({
        index: 0,
        field: "album",
        autoFixEligible: true,
        corrected: { album: "Album" },
      }),
      expect.objectContaining({
        index: 0,
        field: "albumArtist",
        autoFixEligible: true,
        corrected: { albumArtist: "Artist" },
      }),
      expect.objectContaining({
        index: 0,
        field: "year",
        autoFixEligible: true,
        corrected: { year: "2020" },
      }),
      expect.objectContaining({
        index: 0,
        field: "trackNumber",
        autoFixEligible: true,
        corrected: { trackNumber: 1, trackTotal: 2 },
      }),
    ]));
  });

  it("surfaces ambiguous structural issues for manual review instead of writing", () => {
    const findings = buildDeterministicAuditFindings("Artist", "Album", [
      {
        ...baseTrack,
        artist: "AC/DC",
        artists: [],
        year: null,
      },
    ], ["AC/DC - Thunderstruck.flac"]);

    expect(findings).toEqual(expect.arrayContaining([
      expect.objectContaining({
        index: 0,
        field: "artists",
        status: "warning",
        autoFixEligible: false,
        corrected: null,
      }),
    ]));
    expect(findings.some((f) => f.field === "trackNumber" && f.autoFixEligible)).toBe(false);
  });

  it("parses clear disc numbers from filenames and folders only", () => {
    const findings = buildDeterministicAuditFindings("Artist", "Disc 2", [
      {
        ...baseTrack,
        title: "Song",
        trackNumber: 1,
        discNumber: null,
      },
    ], ["02-01 Song.flac"]);

    expect(findings).toEqual(expect.arrayContaining([
      expect.objectContaining({
        field: "discNumber",
        autoFixEligible: true,
        corrected: { discNumber: 2 },
      }),
    ]));
  });

  it("does not force missing album artist on obvious single-artist albums", () => {
    const findings = buildDeterministicAuditFindings("Artist", "Album", [
      {
        ...baseTrack,
        albumArtist: null,
        albumArtists: [],
      },
    ], ["01. Song.flac"]);

    expect(findings.some((finding) => finding.field === "albumArtist")).toBe(false);
    expect(findings.some((finding) => finding.field === "albumArtists")).toBe(false);
  });

  it("does not produce findings or LLM review targets when core tags already match", () => {
    const tracks = [
      baseTrack,
      { ...baseTrack, title: "Second Song", trackNumber: 2 },
    ];
    const filenames = ["01. Song.flac", "02. Second Song.flac"];

    const findings = buildDeterministicAuditFindings("Artist", "2020 - Album", tracks, filenames);
    const targets = buildLlmReviewTargets(tracks, filenames, findings);

    expect(findings).toEqual([]);
    expect(targets).toEqual([]);
  });

  it("targets semantic genre review without making deterministic genre writes", () => {
    const findings = buildDeterministicAuditFindings("Artist", "Album", [
      { ...baseTrack, genre: null },
    ], ["01. Song.flac"]);
    const targets = buildLlmReviewTargets([{ ...baseTrack, genre: null }], ["01. Song.flac"], findings);

    expect(findings.some((f) => f.field === "genre" && f.autoFixEligible)).toBe(false);
    expect(targets).toEqual([
      expect.objectContaining({
        index: 0,
        field: "genre",
        current: "",
      }),
    ]);
  });
});
