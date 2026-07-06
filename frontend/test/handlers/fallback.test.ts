import { describe, it, expect, vi, afterEach } from "vitest";
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import {
  cleanFolderName,
  cleanAlbumFolderName,
  extractYearFromName,
  parseAlbumPath,
  candidateFromFolder,
  trackHintsFromPath,
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

  it("removes full date prefix", () => {
    expect(cleanFolderName("2007-09-28 F.I.R飞儿乐团 爱‧歌姬(24bit-48Hz)(WAV)")).toBe("F.I.R飞儿乐团 爱‧歌姬(24bit-48Hz)");
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

  it("strips CJK artist + English artist prefix from folder name", () => {
    expect(cleanAlbumFolderName("林子祥 - George Lam Ultimate Sound Vol. II", "林子祥")).toBe("Ultimate Sound Vol. II");
  });

  it("uses the parent artist to strip repeated artist/category prefixes", () => {
    expect(cleanAlbumFolderName("流行与摇滚  黄绮珊  小霞2.0", "黄绮珊")).toBe("小霞2.0");
    expect(cleanAlbumFolderName("黄绮珊.2019 - 出走【FLAC】", "黄绮珊")).toBe("出走");
  });

  it("does not strip legitimate album names that contain the artist", () => {
    expect(cleanAlbumFolderName("Queen II", "Queen")).toBe("Queen II");
    expect(parseAlbumPath("/music/Queen/Queen II").albumHint).toBe("Queen II");
  });

  it("does not misparse numeric album names like 100天 as 0天", () => {
    // Bug: DATE_PREFIX_RE matched "2009-10" from "2009-100天", leaving "0天"
    expect(cleanFolderName("2009-100天")).toBe("100天");
    expect(cleanAlbumFolderName("2009-100天", "林俊杰")).toBe("100天");
    const r = parseAlbumPath("/music/林俊杰/2009-100天");
    expect(r.artistHint).toBe("林俊杰");
    expect(r.albumHint).toBe("100天");
    expect(r.yearHint).toBe("2009");
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

  it("extracts trailing or artist-prefixed standalone year", () => {
    expect(extractYearFromName("黄绮珊《时光》2018 .wav")).toBe("2018");
    expect(extractYearFromName("黄绮珊.2019 - 出走【FLAC】")).toBe("2019");
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

  it("parses artist-prefixed Chinese album folders under the default artist/album layout", () => {
    const r = parseAlbumPath("/music/黄绮珊/流行与摇滚  黄绮珊  小霞2.0");
    expect(r.artistHint).toBe("黄绮珊");
    expect(r.albumHint).toBe("小霞2.0");
    expect(r.yearHint).toBeNull();
  });

  it("strips CJK artist + English artist prefix from folder", () => {
    const r = parseAlbumPath("/music/林子祥/林子祥 - George Lam Ultimate Sound Vol. II");
    expect(r.artistHint).toBe("林子祥");
    expect(r.albumHint).toBe("Ultimate Sound Vol. II");
  });

  it("strips space-dash-space separator between artist and album", () => {
    const r = parseAlbumPath("/music/Beatles/Beatles - Abbey Road");
    expect(r.artistHint).toBe("Beatles");
    expect(r.albumHint).toBe("Abbey Road");
  });

  it("parses Chinese bookmark album folders with trailing year", () => {
    const r = parseAlbumPath("/music/黄绮珊/黄绮珊《时光》2018 .wav");
    expect(r.artistHint).toBe("黄绮珊");
    expect(r.albumHint).toBe("时光");
    expect(r.yearHint).toBe("2018");
  });

  it("parses artist-dot-year album folders", () => {
    const r = parseAlbumPath("/music/黄绮珊/黄绮珊.2019 - 出走【FLAC】");
    expect(r.artistHint).toBe("黄绮珊");
    expect(r.albumHint).toBe("出走");
    expect(r.yearHint).toBe("2019");
  });

  it("parses full-date artist-prefixed F.I.R. WAV folders", () => {
    const r = parseAlbumPath("/music/F.I.R./2004-04-30 F.I.R飞儿乐团 飞儿乐团(24bit-48Hz)(WAV)");
    expect(r.artistHint).toBe("F.I.R.");
    expect(r.albumHint).toBe("飞儿乐团");
    expect(r.yearHint).toBe("2004");
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

describe("trackHintsFromPath", () => {
  const tmpRoots: string[] = [];

  afterEach(() => {
    for (const dir of tmpRoots.splice(0)) {
      rmSync(dir, { recursive: true, force: true });
    }
  });

  it("infers track number and title from filename when tags cannot be read", async () => {
    const tmpRoot = mkdtempSync(join(tmpdir(), "auto-tag-fallback-"));
    tmpRoots.push(tmpRoot);
    const albumDir = join(tmpRoot, "黄绮珊", "小霞2.0");
    mkdirSync(albumDir, { recursive: true });
    writeFileSync(join(albumDir, "05.我的美丽.flac"), Buffer.from("not-flac"));

    const tracks = await trackHintsFromPath(albumDir);

    expect(tracks).toHaveLength(1);
    expect(tracks[0].trackNumber).toBe(5);
    expect(tracks[0].title).toBe("我的美丽");
    expect(tracks[0].trackTotal).toBe(1);
  });

  it("uses artist-title filenames as fallback when tags cannot be read", async () => {
    const tmpRoot = mkdtempSync(join(tmpdir(), "auto-tag-fallback-"));
    tmpRoots.push(tmpRoot);
    const albumDir = join(tmpRoot, "F.I.R.", "2004-04-30 F.I.R飞儿乐团 飞儿乐团(24bit-48Hz)(WAV)");
    mkdirSync(albumDir, { recursive: true });
    writeFileSync(join(albumDir, "F.I.R飞儿乐团 - Revolution(飞儿乐团)(24bit-48Hz).wav"), Buffer.from("not-wav"));

    const tracks = await trackHintsFromPath(albumDir);

    expect(tracks).toHaveLength(1);
    expect(tracks[0].artist).toBe("F.I.R飞儿乐团");
    expect(tracks[0].artists).toEqual(["F.I.R飞儿乐团"]);
    expect(tracks[0].title).toBe("Revolution");
    expect(tracks[0].trackNumber).toBe(1);
  });

  it("preserves meaningful parenthesized title qualifiers in filename fallback", async () => {
    const tmpRoot = mkdtempSync(join(tmpdir(), "auto-tag-fallback-"));
    tmpRoots.push(tmpRoot);
    const albumDir = join(tmpRoot, "Artist", "Album");
    mkdirSync(albumDir, { recursive: true });
    writeFileSync(join(albumDir, "01 - Song (Live).flac"), Buffer.from("not-flac"));

    const tracks = await trackHintsFromPath(albumDir);

    expect(tracks).toHaveLength(1);
    expect(tracks[0].title).toBe("Song (Live)");
  });

  it("includes APE files when collecting album track hints", async () => {
    const tmpRoot = mkdtempSync(join(tmpdir(), "auto-tag-fallback-"));
    tmpRoots.push(tmpRoot);
    const albumDir = join(tmpRoot, "刺猬乐队", "刺猬乐队 - 幻象波谱星 APE");
    mkdirSync(albumDir, { recursive: true });
    writeFileSync(join(albumDir, "01 - 我们飞向太空.ape"), Buffer.from("not-ape"));

    const tracks = await trackHintsFromPath(albumDir);

    expect(tracks).toHaveLength(1);
    expect(tracks[0].trackNumber).toBe(1);
    expect(tracks[0].title).toBe("我们飞向太空");
    expect(tracks[0].trackTotal).toBe(1);
  });

  it("parses each audio file once when building album tags and track hints", async () => {
    const tmpRoot = mkdtempSync(join(tmpdir(), "auto-tag-fallback-"));
    tmpRoots.push(tmpRoot);
    const albumDir = join(tmpRoot, "刺猬乐队", "幻象波普星");
    mkdirSync(albumDir, { recursive: true });
    const first = join(albumDir, "01 - 我们飞向太空.ape");
    const second = join(albumDir, "02 - 白日梦蓝.ape");
    writeFileSync(first, Buffer.from("not-ape"));
    writeFileSync(second, Buffer.from("not-ape"));

    vi.resetModules();
    const readTrackMetadata = vi.fn(async (filePath: string) => ({
      path: filePath,
      title: filePath === first ? "我们飞向太空" : "白日梦蓝",
      artist: "刺猬",
      artists: ["刺猬"],
      album: "幻象波普星",
      albumArtist: null,
      albumArtists: [],
      trackNumber: null,
      trackTotal: null,
      discNumber: null,
      discTotal: null,
      year: "2014",
      genre: null,
      composer: null,
      comment: null,
      description: null,
      lyrics: null,
      compilation: null,
      musicbrainzTrackId: null,
      musicbrainzAlbumId: "mb-album",
      musicbrainzArtistId: "mb-artist",
      discogsArtistId: "discogs-artist",
      discogsReleaseId: "discogs-release",
      hasCover: false,
      sizeBytes: 1,
      bitrate: null,
      sampleRate: null,
      codec: "Monkey's Audio",
      duration: 180,
    }));
    vi.doMock("../../electron/handlers/tracks", () => ({ readTrackMetadata }));

    try {
      const { parseAlbumWithTags } = await import("../../electron/handlers/fallback");
      const request = await parseAlbumWithTags(albumDir);

      expect(readTrackMetadata).toHaveBeenCalledTimes(2);
      expect(request.artistHint).toBe("刺猬乐队");
      expect(request.albumHint).toBe("幻象波普星");
      expect(request.musicbrainzAlbumId).toBe("mb-album");
      expect(request.discogsReleaseId).toBe("discogs-release");
      expect(request.tracks.map((track) => track.title)).toEqual(["我们飞向太空", "白日梦蓝"]);
    } finally {
      vi.doUnmock("../../electron/handlers/tracks");
      vi.resetModules();
    }
  });
});
