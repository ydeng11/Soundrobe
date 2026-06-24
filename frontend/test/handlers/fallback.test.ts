import { describe, it, expect, vi } from "vitest";
import {
  cleanFolderName,
  cleanAlbumFolderName,
  extractYearFromName,
  parseAlbumPath,
  candidateFromFolder,
} from "../../electron/handlers/fallback";
import {
  makeLookupRequest,
  makeTrackCandidate,
} from "../../electron/handlers/candidates";

// ── Sync functions (no mocking needed) ──────────────────────────────

describe("cleanFolderName", () => {
  it("removes date prefix", () => {
    expect(cleanFolderName("2003-04《挚爱》")).toBe("挚爱");
  });

  it("removes year prefix", () => {
    expect(cleanFolderName("2017- Album Name")).toBe("Album Name");
  });

  it("removes bookmarks", () => {
    expect(cleanFolderName("《Album》")).toBe("Album");
  });

  it("removes edition keywords", () => {
    expect(cleanFolderName("Album[香港首版]")).toBe("Album");
  });

  it("removes format suffixes", () => {
    expect(cleanFolderName("Album [FLAC]")).toBe("Album");
  });

  it("removes disc count suffix", () => {
    expect(cleanFolderName("Album 2CD")).toBe("Album");
  });

  it("extracts from within bookmarks", () => {
    expect(cleanFolderName("Artist-《2011-Album》[FLAC]")).toBe("Album");
  });

  it("returns original if nothing to clean", () => {
    expect(cleanFolderName("SimpleAlbum")).toBe("SimpleAlbum");
  });
});

describe("cleanAlbumFolderName", () => {
  it("strips leading year-dash", () => {
    expect(cleanAlbumFolderName("2003-04《挚爱》")).toBe("挚爱");
  });

  it("returns cleaned name for simple case", () => {
    expect(cleanAlbumFolderName("Abbey Road")).toBe("Abbey Road");
  });
});

describe("extractYearFromName", () => {
  it("extracts from leading date prefix", () => {
    expect(extractYearFromName("2003-04《挚爱》")).toBe("2003");
  });

  it("extracts from inside Chinese bookmarks", () => {
    expect(extractYearFromName("Artist-《2011-Album》")).toBe("2011");
  });

  it("extracts from parenthesized year", () => {
    expect(extractYearFromName("Album (2011)")).toBe("2011");
  });

  it("returns null when no year found", () => {
    expect(extractYearFromName("Just An Album")).toBeNull();
  });
});

describe("parseAlbumPath", () => {
  it("parses standard Artist/Album path", () => {
    const r = parseAlbumPath("/music/Beatles/Abbey Road");
    expect(r.artistHint).toBe("Beatles");
    expect(r.albumHint).toBe("Abbey Road");
    expect(r.yearHint).toBeNull();
  });

  it("parses file path within album directory", () => {
    const r = parseAlbumPath("/music/Beatles/Abbey Road/01 Come Together.mp3");
    expect(r.artistHint).toBe("Beatles");
    expect(r.albumHint).toBe("Abbey Road");
  });

  it("extracts year from album name", () => {
    const r = parseAlbumPath("/music/Artist/1969-Abbey Road");
    expect(r.albumHint).toBe("Abbey Road");
    expect(r.yearHint).toBe("1969");
  });

  it("treats dotted album folder names as directories, not file paths", () => {
    const r = parseAlbumPath("/music/郭富城/1992-跳不完.爱不完.唱不完");
    expect(r.artistHint).toBe("郭富城");
    expect(r.albumHint).toBe("跳不完.爱不完.唱不完");
    expect(r.yearHint).toBe("1992");
  });

  it("parses CD subfolder", () => {
    const r = parseAlbumPath("/music/Artist/Album (2CD)/CD1");
    expect(r.artistHint).toBe("Artist");
    // albumHint comes from parent "Album (2CD)"
    expect(r.albumHint).toBe("Album");
  });

  it("handles flat album path", () => {
    const r = parseAlbumPath("/music/Some Album");
    expect(r.artistHint).toBe("music");
    expect(r.albumHint).toBe("Some Album");
  });
});

describe("candidateFromFolder", () => {
  it("builds candidate from a lookup request", () => {
    const req = makeLookupRequest({
      path: "/music/Artist/Album",
      artistHint: "Artist",
      albumHint: "Album",
      yearHint: "2000",
      tracks: [makeTrackCandidate({ title: "Track 1", trackNumber: 1 })],
    });
    const c = candidateFromFolder(req);
    expect(c.artist).toBe("Artist");
    expect(c.album).toBe("Album");
    expect(c.year).toBe("2000");
    expect(c.source).toBe("folder");
    expect(c.tracks).toHaveLength(1);
    expect(c.verification).toBeNull();
  });

  it("handles null hints", () => {
    const req = makeLookupRequest({ path: "/music/Unknown" });
    const c = candidateFromFolder(req);
    expect(c.artist).toBeNull();
    expect(c.album).toBeNull();
    expect(c.source).toBe("folder");
  });
});
