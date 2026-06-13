import { describe, expect, it } from "vitest";
import { FilenameTagInferenceService } from "../../electron/services/FilenameTagInferenceService";

describe("FilenameTagInferenceService", () => {
  it("infers per-track title and artist from the 上学嗨 mixtape filenames without using folder names", () => {
    const service = new FilenameTagInferenceService();
    const results = service.inferFromFilenames([
      "/Volumes/downloads/法老/Loose/Loose/flac/上学嗨mixtape/法老 - Cheeseburger.flac",
      "/Volumes/downloads/法老/Loose/Loose/flac/上学嗨mixtape/法老 - Ghost face.flac",
      "/Volumes/downloads/法老/Loose/Loose/flac/上学嗨mixtape/法老,戴思蒙德 - 丑陋鼠草.flac",
    ]);

    expect(results).toEqual([
      {
        trackPath: "/Volumes/downloads/法老/Loose/Loose/flac/上学嗨mixtape/法老 - Cheeseburger.flac",
        fields: { title: "Cheeseburger", artist: "法老", artists: ["法老"] },
        reason: "Parsed \"法老 - Cheeseburger.flac\" as artist-title filename",
      },
      {
        trackPath: "/Volumes/downloads/法老/Loose/Loose/flac/上学嗨mixtape/法老 - Ghost face.flac",
        fields: { title: "Ghost face", artist: "法老", artists: ["法老"] },
        reason: "Parsed \"法老 - Ghost face.flac\" as artist-title filename",
      },
      {
        trackPath: "/Volumes/downloads/法老/Loose/Loose/flac/上学嗨mixtape/法老,戴思蒙德 - 丑陋鼠草.flac",
        fields: { title: "丑陋鼠草", artist: "法老,戴思蒙德", artists: ["法老", "戴思蒙德"] },
        reason: "Parsed \"法老,戴思蒙德 - 丑陋鼠草.flac\" as artist-title filename",
      },
    ]);
  });

  it("skips filenames that do not have a clear artist-title separator", () => {
    const service = new FilenameTagInferenceService();

    expect(service.inferFromFilenames([
      "/music/上学嗨mixtape/Cheeseburger.flac",
    ])).toEqual([]);
  });

  it("parses compact artist-title with leading track number and prettifies underscores", () => {
    const service = new FilenameTagInferenceService();
    const results = service.inferFromFilenames(
      [
        "/Volumes/downloads/music/刺猬乐队/刺猬《神经元》2015 FLAC 分轨/110-hedgehog-you_are_so_famous.flac",
      ],
      { prettify: true },
    );

    expect(results).toEqual([
      {
        trackPath: "/Volumes/downloads/music/刺猬乐队/刺猬《神经元》2015 FLAC 分轨/110-hedgehog-you_are_so_famous.flac",
        fields: {
          title: "You Are So Famous",
          artist: "Hedgehog",
          artists: ["Hedgehog"],
        },
        reason: "Parsed \"110-hedgehog-you_are_so_famous.flac\" as compact artist-title filename",
      },
    ]);
  });

  it("parses compact artist-title without prettification", () => {
    const service = new FilenameTagInferenceService();
    const results = service.inferFromFilenames([
      "/music/刺猬乐队/刺猬《神经元》/05_hedgehog-you_are_so_famous.flac",
    ]);

    expect(results).toEqual([
      {
        trackPath: "/music/刺猬乐队/刺猬《神经元》/05_hedgehog-you_are_so_famous.flac",
        fields: {
          title: "you_are_so_famous",
          artist: "hedgehog",
          artists: ["hedgehog"],
        },
        reason: "Parsed \"05_hedgehog-you_are_so_famous.flac\" as compact artist-title filename",
      },
    ]);
  });

  it("does NOT parse compact dash without a leading track number (avoids false positives)", () => {
    const service = new FilenameTagInferenceService();
    const results = service.inferFromFilenames([
      "/music/SomeBand/Album/song-title-only.flac",
    ]);

    // Without a leading track number, compact dash is ambiguous
    // (could be a title with a dash, not artist-title), so skip.
    expect(results).toEqual([]);
  });
});
