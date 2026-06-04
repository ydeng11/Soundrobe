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
});
