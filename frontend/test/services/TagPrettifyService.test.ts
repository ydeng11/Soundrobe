import { describe, expect, it } from "vitest";
import { prettifyTag, prettifyTags } from "../../electron/services/TagPrettifyService";

describe("prettifyTag", () => {
  it("converts underscore-separated lowercase to title case", () => {
    expect(prettifyTag("you_are_so_famous")).toBe("You Are So Famous");
  });

  it("strips leading track number and converts underscores", () => {
    expect(prettifyTag("110-hedgehog-you_are_so_famous")).toBe("Hedgehog You Are So Famous");
  });

  it("handles leading track number with dot separator", () => {
    expect(prettifyTag("01.hello_world")).toBe("Hello World");
  });

  it("handles leading track number with underscore separator", () => {
    expect(prettifyTag("05_never_gonna_give_you_up")).toBe("Never Gonna Give You Up");
  });

  it("preserves CJK characters without inserting spaces", () => {
    expect(prettifyTag("枯れゆく花の下で-live_at_budokan-")).toBe("枯れゆく花の下で Live At Budokan");
  });

  it("converts all-uppercase words to title case", () => {
    expect(prettifyTag("NEVER GONNA GIVE YOU UP")).toBe("Never Gonna Give You Up");
  });

  it("preserves dotted acronyms like F.I.R.", () => {
    expect(prettifyTag("F.I.R.")).toBe("F.I.R.");
  });

  it("passes through already-pretty text unchanged", () => {
    expect(prettifyTag("Already Pretty")).toBe("Already Pretty");
  });

  it("handles mixed CJK and latin with underscore separators", () => {
    expect(prettifyTag("刺猬-you_are_so_famous")).toBe("刺猬 You Are So Famous");
  });

  it("handles Chinese parent folder with underscore title", () => {
    // Simulating: basename "110-hedgehog-you_are_so_famous"
    expect(prettifyTag("01_千年之恋")).toBe("千年之恋");
  });

  it("handles 'disc' prefix in leading track number", () => {
    expect(prettifyTag("disc1-track2-song_name")).toBe("Track 2 Song Name");
  });

  it("preserves parenthetical suffixes", () => {
    expect(prettifyTag("song_name_(feat._someone)")).toBe("Song Name (Feat. Someone)");
  });

  it("replaces hyphens with spaces", () => {
    expect(prettifyTag("this-is-a-test")).toBe("This Is A Test");
  });

  it("handles empty string", () => {
    expect(prettifyTag("")).toBe("");
  });

  it("handles null or undefined input gracefully", () => {
    expect(prettifyTag(null as unknown as string)).toBe("");
    expect(prettifyTag(undefined as unknown as string)).toBe("");
  });

  it("handles single word", () => {
    expect(prettifyTag("hello")).toBe("Hello");
  });

  it("handles all-numeric string", () => {
    expect(prettifyTag("12345")).toBe("12345");
  });

  it("handles only separators", () => {
    expect(prettifyTag("_-_-_")).toBe("");
  });

  it("properly capitalizes only the first letter of each word", () => {
    expect(prettifyTag("a b c d")).toBe("A B C D");
  });
});

describe("prettifyTags", () => {
  it("prettifies multiple fields from an object", () => {
    const result = prettifyTags({
      artist: "hedgehog",
      title: "you_are_so_famous",
    });
    expect(result).toEqual({
      artist: "Hedgehog",
      title: "You Are So Famous",
    });
  });

  it("handles empty fields object", () => {
    expect(prettifyTags({})).toEqual({});
  });

  it("returns null for null/undefined values without crashing", () => {
    const result = prettifyTags({
      artist: null,
      title: "you_are_so_famous",
      album: undefined,
    });
    expect(result.artist).toBeNull();
    expect(result.title).toBe("You Are So Famous");
    expect(result.album).toBeUndefined();
  });
});
