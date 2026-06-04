import { describe, expect, it } from "vitest";
import { hasBlankPerTrackFields } from "../../electron/handlers/assistant";

describe("assistant metadata guards", () => {
  it("rejects blank per-track fields because empty filename-derived edits can wipe tags", () => {
    expect(hasBlankPerTrackFields({ title: "", artist: "", artists: [] })).toBe(true);
  });

  it("allows non-empty per-track fields", () => {
    expect(hasBlankPerTrackFields({
      title: "Cheeseburger",
      artist: "法老",
      artists: ["法老"],
    })).toBe(false);
  });
});
