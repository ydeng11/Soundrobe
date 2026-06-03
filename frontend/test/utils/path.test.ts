import { describe, it, expect } from "vitest";
import {
  toPosixPath,
  dirname,
  basename,
  shortPath,
  isInsideDirectory,
} from "../../src/utils/path";

describe("toPosixPath", () => {
  it("leaves POSIX paths unchanged", () => {
    expect(toPosixPath("/Music/Album/file.flac")).toBe("/Music/Album/file.flac");
  });

  it("converts Windows backslashes to forward slashes", () => {
    expect(toPosixPath("C:\\Music\\Album\\file.flac")).toBe("C:/Music/Album/file.flac");
  });

  it("converts mixed separators", () => {
    expect(toPosixPath("C:\\Music/Album\\file.flac")).toBe("C:/Music/Album/file.flac");
  });
});

describe("dirname", () => {
  it("returns parent of a POSIX path", () => {
    expect(dirname("/Music/Album/file.flac")).toBe("/Music/Album");
  });

  it("returns parent of a Windows path", () => {
    expect(dirname("C:\\Music\\Album\\file.flac")).toBe("C:/Music/Album");
  });

  it("returns parent of a top-level path", () => {
    expect(dirname("/Music")).toBe("");
  });
});

describe("basename", () => {
  it("returns filename from a POSIX path", () => {
    expect(basename("/Music/Album/file.flac")).toBe("file.flac");
  });

  it("returns filename from a Windows path", () => {
    expect(basename("C:\\Music\\Album\\file.flac")).toBe("file.flac");
  });

  it("returns the last segment from a directory path", () => {
    expect(basename("/Music/Album")).toBe("Album");
  });
});

describe("shortPath", () => {
  it("returns last 4 segments with default depth", () => {
    expect(shortPath("/a/b/c/d/e/file.flac")).toBe("c/d/e/file.flac");
  });

  it("returns last N segments with custom depth", () => {
    expect(shortPath("/a/b/c/d/e/file.flac", 2)).toBe("e/file.flac");
  });

  it("handles Windows paths", () => {
    expect(shortPath("C:\\Users\\Music\\Album\\track.mp3")).toBe("Users/Music/Album/track.mp3");
  });

  it("handles fewer segments than depth", () => {
    expect(shortPath("/a/b.flac")).toBe("a/b.flac");
  });
});

describe("isInsideDirectory", () => {
  it("returns true for a direct child", () => {
    expect(isInsideDirectory("/lib/album/01.flac", "/lib/album")).toBe(true);
  });

  it("returns true for nested child", () => {
    expect(isInsideDirectory("/lib/album/sub/track.flac", "/lib/album")).toBe(true);
  });

  it("returns false for a sibling prefix match", () => {
    expect(isInsideDirectory("/lib/album deluxe/02.flac", "/lib/album")).toBe(false);
  });

  it("handles Windows-style paths", () => {
    expect(isInsideDirectory("C:\\Music\\Album\\track.flac", "C:\\Music\\Album")).toBe(true);
  });
});
