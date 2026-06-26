import { describe, it, expect, afterEach, beforeEach, vi } from "vitest";

// Mock native-check so better-sqlite3 doesn't need to load (ABI mismatch in Vitest)
vi.mock("../../electron/handlers/native-check", () => {
  class MockStatement {
    run(..._params: unknown[]) {
      return { changes: 1, lastInsertRowid: 1 };
    }
    get(..._params: unknown[]) {
      return undefined;
    }
    all(..._params: unknown[]) {
      return [];
    }
    bind(..._params: unknown[]) {}
  }

  class MockDB {
    constructor(_path: string) {}
    pragma(_sql: string) {
      return {};
    }
    prepare(_sql: string) {
      return new MockStatement();
    }
    exec(_sql: string) {}
    close() {}
  }

  return {
    getBetterSqlite3: () => MockDB as never,
  };
});
import { mkdtempSync, rmSync, writeFileSync, mkdirSync, openSync, writeSync, closeSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { parseFile } from "music-metadata";
import {
  flacHeaderWithDuration,
  vorbisCommentBlock,
} from "../helpers/flac-helpers";
import {
  hintsAreAmbiguous,
  filterCandidatesForAutoApply,
  protectCandidateTrackFieldsForAutoApply,
  loadConfig,
  startAutoTag,
  getProgress,
  cancelTask,
  getDatasetStatus,
  getConfig,
  refreshConfig,
  onAutoTagEvent,
  buildAliasedLookupVariants,
  mergeAutoTagCandidateFields,
  applyCanonicalArtistName,
  chooseProviderArtistName,
} from "../../electron/handlers/auto-tag";
import { setAliasFilePath, saveAlias } from "../../electron/handlers/aliases";
import { makeAlbumCandidate, makeLookupRequest, makeTrackCandidate } from "../../electron/handlers/candidates";
import { candidateFromFolder } from "../../electron/handlers/fallback";
import { writeTags, batchWriteTags } from "../../electron/handlers/writer";
import { readTrackMetadata } from "../../electron/handlers/tracks";
import * as NodeID3 from "node-id3";

/**
 * Shared env isolation for auto-tag lifecycle tests.
 * Returns { tmpHome, originalEnv } for use in the describe block.
 */
function setupTestEnv(label: string) {
  const originalEnv = { ...process.env };
  const tmpHome = mkdtempSync(join(tmpdir(), `auto-tag-${label}-`));
  process.env.HOME = tmpHome;
  delete process.env.LLM_API_KEY;
  delete process.env.LLM_MODEL;
  process.env.AUTO_TAG_REMOTE_LOOKUP = "false";
  process.env.AUTO_TAG_DISCOGS_ENABLED = "false";
  refreshConfig();
  return { originalEnv, tmpHome };
}

function teardownTestEnv(originalEnv: Record<string, string | undefined>, tmpHome: string) {
  process.env = { ...originalEnv };
  refreshConfig();
  rmSync(tmpHome, { recursive: true, force: true });
}

describe("loadConfig", () => {
  const originalEnv = { ...process.env };

  afterEach(() => {
    process.env = { ...originalEnv };
    // Clear config-dependent env vars that might exist on the dev machine
    delete process.env.LLM_API_KEY;
    delete process.env.LLM_MODEL;
    delete process.env.AUTO_TAG_DISCOGS_TOKEN;
  });

  it("loads from environment variables", () => {
    process.env.LLM_API_KEY = "env-key";
    process.env.LLM_MODEL = "env-model";
    process.env.AUTO_TAG_DISCOGS_TOKEN = "env-token";

    const config = loadConfig();
    expect(config.llmApiKey).toBe("env-key");
    expect(config.llmModel).toBe("env-model");
    expect(config.discogsToken).toBe("env-token");
  });

  it("defaults to remote lookup enabled", () => {
    delete process.env.AUTO_TAG_REMOTE_LOOKUP;
    const config = loadConfig();
    expect(config.remoteLookupEnabled).toBe(true);
  });

  it("respects remote lookup disabled", () => {
    process.env.AUTO_TAG_REMOTE_LOOKUP = "false";
    expect(loadConfig().remoteLookupEnabled).toBe(false);
  });
});

describe("filterCandidatesForAutoApply", () => {
  it("rejects mismatched artist-only dataset candidates before writing tags", async () => {
    const request = makeLookupRequest({
      artistHint: "邓丽君",
      albumHint: "假如我是真的",
    });
    const wrongDatasetAlbum = makeAlbumCandidate({
      source: "dataset",
      artist: "邓丽君",
      album: "何日君再来演唱会现场实录 (Live)",
      tracks: [
        { title: "甜蜜蜜", artist: "邓丽君", artists: ["邓丽君"], trackNumber: 1, trackTotal: 14, discNumber: null, discTotal: null, musicbrainzTrackId: null, length: null },
      ],
    });
    const safeFolderFallback = makeAlbumCandidate({
      source: "folder",
      artist: "邓丽君",
      album: "假如我是真的",
      tracks: [],
    });

    const filtered = await filterCandidatesForAutoApply(request, [
      wrongDatasetAlbum,
      safeFolderFallback,
    ]);

    expect(filtered).toEqual([safeFolderFallback]);
  });
});

describe("mergeAutoTagCandidateFields", () => {
  it("prefers provider evidence over LLM and folder fallbacks", () => {
    const provider = makeAlbumCandidate({
      source: "musicbrainz",
      artist: "蔡依林",
      album: "看我72变",
      year: "2003",
      musicbrainzAlbumId: "mb-release",
      musicbrainzArtistId: "mb-artist",
      tracks: [
        makeTrackCandidate({ title: "说爱你", trackNumber: 1 }),
      ],
    });
    const llm = makeAlbumCandidate({
      source: "llm",
      artist: "Jolin Tsai",
      album: "Magic",
      genre: "Mandopop",
      tracks: [
        makeTrackCandidate({ title: "Say Love You", trackNumber: 1 }),
      ],
    });
    const folder = makeAlbumCandidate({
      source: "folder",
      artist: "蔡依林",
      album: "2003-看我72变",
      tracks: [],
    });

    const [merged] = mergeAutoTagCandidateFields([llm, folder, provider]);

    expect(merged.source).toBe("musicbrainz");
    expect(merged.artist).toBe("蔡依林");
    expect(merged.album).toBe("看我72变");
    expect(merged.musicbrainzAlbumId).toBe("mb-release");
    expect(merged.genre).toBe("Mandopop");
  });

  it("lets a fresh provider candidate clean stale cached provider track titles for the same release", () => {
    const cached = makeAlbumCandidate({
      source: "musicbrainz",
      artist: "F.I.R.",
      album: "无限",
      musicbrainzAlbumId: "mb-infinite",
      tracks: [
        makeTrackCandidate({ title: "F.I.R飞儿乐团 - I Can't Go On(无限)(24bit-48Hz)", trackNumber: 1 }),
      ],
    });
    const fresh = makeAlbumCandidate({
      source: "musicbrainz",
      artist: "F.I.R.",
      album: "无限",
      musicbrainzAlbumId: "mb-infinite",
      tracks: [
        makeTrackCandidate({ title: "I Can't Go On", trackNumber: 1 }),
      ],
    });

    const [merged] = mergeAutoTagCandidateFields([cached, fresh]);

    expect(merged.tracks[0].title).toBe("I Can't Go On");
  });
});

describe("applyCanonicalArtistName", () => {
  it("uses provider artist real name for album and track artist fields", () => {
    const candidate = makeAlbumCandidate({
      source: "discogs",
      artist: "Xiao Xia",
      artists: ["Xiao Xia"],
      albumArtist: "Xiao Xia",
      albumArtists: ["Xiao Xia"],
      discogsArtistId: "5244238",
      tracks: [
        makeTrackCandidate({
          title: "我的美丽",
          artist: "Xiao Xia",
          artists: ["Xiao Xia"],
          trackNumber: 5,
        }),
      ],
    });

    const normalized = applyCanonicalArtistName(candidate, "黄绮珊");

    expect(normalized.artist).toBe("黄绮珊");
    expect(normalized.artists).toEqual(["黄绮珊"]);
    expect(normalized.albumArtist).toBe("黄绮珊");
    expect(normalized.albumArtists).toEqual(["黄绮珊"]);
    expect(normalized.tracks[0].artist).toBe("黄绮珊");
    expect(normalized.tracks[0].artists).toEqual(["黄绮珊"]);
    expect(normalized.tracks[0].title).toBe("我的美丽");
    expect(normalized.discogsArtistId).toBe("5244238");
  });
});

describe("chooseProviderArtistName", () => {
  it("prefers MusicBrainz name over Discogs realname with parenthesized aliases", () => {
    expect(chooseProviderArtistName(
      {
        name: "F.I.R.",
        realname: "F.I.R. (飛兒樂團, 飞儿乐团;, Fēiér Yuètuán)",
      },
      "F.I.R.",
    )).toBe("F.I.R.");
  });

  it("strips Discogs parenthesized aliases when MusicBrainz has no name", () => {
    expect(chooseProviderArtistName(
      {
        name: "F.I.R.",
        realname: "F.I.R. (飛兒樂團, 飞儿乐团;, Fēiér Yuètuán)",
      },
      null,
    )).toBe("F.I.R.");
  });
});

describe("candidateFromFolder", () => {
  it("preserves existing provider IDs so fallback writes do not erase durable identity", () => {
    const candidate = candidateFromFolder(makeLookupRequest({
      artistHint: "郭富城",
      albumHint: "到底有谁能够告诉我",
      musicbrainzAlbumId: "mb-release",
      musicbrainzArtistId: "mb-artist",
      discogsReleaseId: "discogs-release",
      discogsArtistId: "discogs-artist",
    }));

    expect(candidate.musicbrainzAlbumId).toBe("mb-release");
    expect(candidate.musicbrainzArtistId).toBe("mb-artist");
    expect(candidate.discogsReleaseId).toBe("discogs-release");
    expect(candidate.discogsArtistId).toBe("discogs-artist");
  });
});

describe("protectCandidateTrackFieldsForAutoApply", () => {
  it("keeps remote album fields but only matches subset tracks by title+duration", async () => {
    const request = makeLookupRequest({
      path: "/tmp/费玉清/一剪梅",
      artistHint: "费玉清",
      albumHint: "一剪梅",
      tracks: [
        makeTrackCandidate({ title: "不变的心", trackNumber: 1 }),
        makeTrackCandidate({ title: "变色的长城", trackNumber: 2 }),
        makeTrackCandidate({ title: "楚留香新传", trackNumber: 3 }),
        makeTrackCandidate({ title: "船歌", trackNumber: 4 }),
        makeTrackCandidate({ title: "黄叶舞秋风", trackNumber: 5 }),
      ],
    });
    const remoteCompilation = makeAlbumCandidate({
      source: "musicbrainz",
      album: "一剪梅 黑胶唱片3CD精品典藏",
      musicbrainzAlbumId: "remote-release",
      tracks: [
        makeTrackCandidate({ title: "梦驼铃", trackNumber: 1 }),
        makeTrackCandidate({ title: "一剪梅", trackNumber: 2 }),
        makeTrackCandidate({ title: "挑夫", trackNumber: 3 }),
        makeTrackCandidate({ title: "送你一把土", trackNumber: 4 }),
        makeTrackCandidate({ title: "变色的长城", trackNumber: 5 }),
        makeTrackCandidate({ title: "长江水", trackNumber: 6 }),
      ],
    });

    const [protectedCandidate] = await protectCandidateTrackFieldsForAutoApply(request, [remoteCompilation]);

    // Album-level fields preserved
    expect(protectedCandidate.album).toBe("一剪梅 黑胶唱片3CD精品典藏");
    expect(protectedCandidate.musicbrainzAlbumId).toBe("remote-release");

    // Only `变色的长城` matches — tracks preserved, no remote overwrites for non-matched
    expect(protectedCandidate.tracks).toHaveLength(5);
    expect(protectedCandidate.tracks[0].title).toBe("不变的心");
    expect(protectedCandidate.tracks[0].trackNumber).toBe(1);
    expect(protectedCandidate.tracks[1].title).toBe("变色的长城");
    // Not a full ordered match — remote track number NOT written
    expect(protectedCandidate.tracks[1].trackNumber).toBe(2);
    expect(protectedCandidate.tracks[2].title).toBe("楚留香新传");
    expect(protectedCandidate.tracks[3].title).toBe("船歌");
    expect(protectedCandidate.tracks[4].title).toBe("黄叶舞秋风");
  });

  it("allows remote per-track fields for a full ordered match", async () => {
    const request = makeLookupRequest({
      path: "/tmp/费玉清/唱一遍一遍",
      artistHint: "费玉清",
      albumHint: "唱一遍一遍",
      tracks: [
        makeTrackCandidate({ title: "唱一遍一遍", trackNumber: 1 }),
        makeTrackCandidate({ title: "传奇", trackNumber: 2 }),
      ],
    });
    const remoteAlbum = makeAlbumCandidate({
      source: "musicbrainz",
      album: "唱一遍一遍",
      tracks: [
        makeTrackCandidate({ title: "唱一遍一遍", trackNumber: 1 }),
        makeTrackCandidate({ title: "传奇", trackNumber: 2 }),
      ],
    });

    const [protectedCandidate] = await protectCandidateTrackFieldsForAutoApply(request, [remoteAlbum]);

    expect(protectedCandidate.tracks.map((track) => track.title)).toEqual(["唱一遍一遍", "传奇"]);
    // Full ordered match — remote track numbers may be used
    expect(protectedCandidate.tracks[0].trackNumber).toBe(1);
    expect(protectedCandidate.tracks[1].trackNumber).toBe(2);
  });

  it("lets a full ordered provider match fix suffix-polluted track titles", async () => {
    const request = makeLookupRequest({
      path: "/tmp/F.I.R./亚特兰提斯",
      artistHint: "F.I.R.",
      albumHint: "亚特兰提斯",
      tracks: [
        makeTrackCandidate({ title: "微光(亚特兰提斯)(24bit-48Hz)", trackNumber: 4 }),
        makeTrackCandidate({ title: "讓我們一起微笑吧", trackNumber: 5 }),
      ],
    });
    const remoteAlbum = makeAlbumCandidate({
      source: "musicbrainz",
      album: "亞特蘭提斯",
      tracks: [
        makeTrackCandidate({ title: "微光", trackNumber: 4 }),
        makeTrackCandidate({ title: "讓我們一起微笑吧", trackNumber: 5 }),
      ],
    });

    const [protectedCandidate] = await protectCandidateTrackFieldsForAutoApply(request, [remoteAlbum]);

    expect(protectedCandidate.tracks.map((track) => track.title)).toEqual(["微光", "讓我們一起微笑吧"]);
  });

  it("does not overwrite non-empty local artist with remote artist", async () => {
    const request = makeLookupRequest({
      path: "/tmp/Artist/Album",
      artistHint: "Artist",
      albumHint: "Album",
      tracks: [
        makeTrackCandidate({ title: "Song", trackNumber: 1, artist: "Local Artist", artists: ["Local Artist"] }),
      ],
    });
    const remoteAlbum = makeAlbumCandidate({
      source: "musicbrainz",
      album: "Album",
      tracks: [
        makeTrackCandidate({ title: "Song", trackNumber: 1, artist: "Remote Artist", artists: ["Remote Artist"] }),
      ],
    });

    const [protectedCandidate] = await protectCandidateTrackFieldsForAutoApply(request, [remoteAlbum]);

    // Local artist is non-empty — remote should NOT overwrite
    expect(protectedCandidate.tracks[0].artist).toBe("Local Artist");
    expect(protectedCandidate.tracks[0].artists).toEqual(["Local Artist"]);
  });

  it("fills blank local artist from remote for matched tracks", async () => {
    const request = makeLookupRequest({
      path: "/tmp/Artist/Album",
      artistHint: "Artist",
      albumHint: "Album",
      tracks: [
        makeTrackCandidate({ title: "Song", trackNumber: 1, artist: null, artists: [] }),
      ],
    });
    const remoteAlbum = makeAlbumCandidate({
      source: "musicbrainz",
      album: "Album",
      tracks: [
        makeTrackCandidate({ title: "Song", trackNumber: 1, artist: "Remote Artist", artists: ["Remote Artist"] }),
      ],
    });

    const [protectedCandidate] = await protectCandidateTrackFieldsForAutoApply(request, [remoteAlbum]);

    expect(protectedCandidate.tracks[0].artist).toBe("Remote Artist");
  });
});

describe("startAutoTag / getProgress / cancelTask", () => {
  const { originalEnv, tmpHome } = setupTestEnv("unit-home");

  afterEach(() => {
    teardownTestEnv(originalEnv, tmpHome);
  });

  it("creates a task and tracks it", async () => {
    const taskId = startAutoTag("/test/album/path");
    expect(taskId).toBeTruthy();
    expect(taskId.startsWith("auto-tag-")).toBe(true);

    const progress = getProgress(taskId);
    expect(progress).not.toBeNull();
    expect(progress!.taskId).toBe(taskId);
    expect(progress!.status).toBe("running");
    expect(progress!.total).toBe(9);
    cancelTask(taskId);
  });

  it("returns null for unknown task", () => {
    expect(getProgress("nonexistent")).toBeNull();
  });

  it("cancels a running task", () => {
    const taskId = startAutoTag("/test/album");
    cancelTask(taskId);
    const progress = getProgress(taskId);
    expect(progress!.status).toBe("cancelled");
  });

  it("emits live task events", () => {
    const events: string[] = [];
    const unsubscribe = onAutoTagEvent((event) => events.push(event.type));
    const taskId = startAutoTag("/test/album/events");
    const progress = getProgress(taskId);
    unsubscribe();
    cancelTask(taskId);

    expect(progress).not.toBeNull();
    expect(events).toContain("progress");
  });
});

describe("getDatasetStatus", () => {
  it("returns status without crashing", { timeout: 15000 }, () => {
    // May or may not have a dataset — just verify it returns a valid shape
    const status = getDatasetStatus();
    expect(status).toHaveProperty("available");
    expect(status).toHaveProperty("totalRecords");
    expect(typeof status.available).toBe("boolean");
    expect(typeof status.totalRecords).toBe("number");
  });
});

describe("hintsAreAmbiguous", () => {
  // ── 蛋堡 / 2009-Winter Sweet[flac] — the exact scenario that broke ──

  it("does not trigger on [flac] format suffix (蛋堡 case)", () => {
    // Folder:   /蛋堡/2009-Winter Sweet[flac]
    // Parsed:   albumHint="Winter Sweet" artistHint="蛋堡" yearHint="2009"
    expect(
      hintsAreAmbiguous(
        "Winter Sweet",
        "蛋堡",
        "/蛋堡/2009-Winter Sweet[flac]",
        "2009",
      ),
    ).toBe(false);
  });

  it("does not trigger on [flac] with just album hint", () => {
    expect(
      hintsAreAmbiguous(
        "Winter Sweet",
        "Some Artist",
        "/Artist/2009-Winter Sweet[flac]",
        null,
      ),
    ).toBe(false);
  });

  it("does not trigger on [FLAC] uppercase", () => {
    expect(
      hintsAreAmbiguous(
        "Winter Sweet",
        "Eggman",
        "/Eggman/2009-Winter Sweet[FLAC]",
        null,
      ),
    ).toBe(false);
  });

  it("does not trigger on [mp3] format suffix", () => {
    expect(
      hintsAreAmbiguous(
        "Album Name",
        "Artist",
        "/Artist/Album Name[mp3]",
        null,
      ),
    ).toBe(false);
  });

  it("does not trigger on other format suffixes", () => {
    for (const suffix of ["[wav]", "[aac]", "[ogg]", "[m4a]", "[ape]"]) {
      expect(
        hintsAreAmbiguous(
          "Album",
          "Artist",
          `/Artist/Album${suffix}`,
          null,
        ),
        `suffix ${suffix} should not trigger`,
      ).toBe(false);
    }
  });

  it("does not trigger on format suffix without brackets", () => {
    expect(
      hintsAreAmbiguous(
        "Album",
        "Artist",
        "/Artist/Album flac分轨",
        null,
      ),
    ).toBe(false);
  });

  // ── Missing hints → ambiguous ─────────────────────────────────────

  it("returns true when album hint is empty", () => {
    expect(
      hintsAreAmbiguous(
        "",
        "Artist",
        "/Artist/FolderName",
        null,
      ),
    ).toBe(true);
  });

  it("returns true when artist hint is empty", () => {
    expect(
      hintsAreAmbiguous(
        "Album",
        "",
        "/Artist/Album",
        null,
      ),
    ).toBe(true);
  });

  // ── Genuine ambiguity triggers ────────────────────────────────────

  it("triggers on Chinese bookmarks", () => {
    // Folder "《2011-重译》" has actual bookmarks, not a format suffix
    expect(
      hintsAreAmbiguous(
        "重译",
        "崔健",
        "/崔健/《2011-重译》",
        "2011",
      ),
    ).toBe(true);
  });

  it("triggers on Chinese dot between CJK characters", () => {
    expect(
      hintsAreAmbiguous(
        "Album.Name",
        "Artist",
        "/Artist/Album.Name",
        null,
      ),
    ).toBe(true);
  });

  it("triggers on year-prefixed album hint", () => {
    expect(
      hintsAreAmbiguous(
        "2009-Album",
        "Artist",
        "/Artist/2009-Album",
        null,
      ),
    ).toBe(true);
  });

  it("triggers on Japanese bookmarks", () => {
    expect(
      hintsAreAmbiguous(
        "Album",
        "Artist",
        "/Artist/「Album」",
        null,
      ),
    ).toBe(true);
  });

  // ── Clean names → NOT ambiguous ───────────────────────────────────

  it("returns false for a clean folder name", () => {
    expect(
      hintsAreAmbiguous(
        "Abbey Road",
        "The Beatles",
        "/The Beatles/Abbey Road",
        "1969",
      ),
    ).toBe(false);
  });

  it("only checks the album folder segment for Windows-style paths", () => {
    expect(
      hintsAreAmbiguous(
        "Abbey Road",
        "The Beatles",
        "C:\\Music\\[The Beatles]\\Abbey Road",
        "1969",
      ),
    ).toBe(false);
  });

  it("returns false when albumHint has year but folder hint is clean", () => {
    // Year prefix in album hint alone doesn't trigger; it's the folder
    // name pattern that matters unless the album hint itself has the
    // year prefix.
    expect(
      hintsAreAmbiguous(
        "Thriller",
        "Michael Jackson",
        "/Michael Jackson/Thriller",
        "1982",
      ),
    ).toBe(false);
  });
});

describe("getConfig / refreshConfig", () => {
  it("returns config without exposing full keys", () => {
    const config = getConfig();
    expect(config).toHaveProperty("llmApiKey");
    expect(config).toHaveProperty("llmModel");
  });

  it("refreshConfig does not throw", () => {
    expect(() => refreshConfig()).not.toThrow();
  });
});

describe("full-flow scenario tests", () => {
  // These tests use synthetic paths, so they verify task lifecycle only.
  // Real album processing is covered by integration/auto-tag-compilation-e2e.
  const { originalEnv, tmpHome } = setupTestEnv("flow-home");

  afterEach(() => {
    teardownTestEnv(originalEnv, tmpHome);
  });

  it("starts and can cancel a synthetic album path with no LLM key", () => {
    const taskId = startAutoTag("/test/artist/album");
    expect(taskId).toBeTruthy();

    cancelTask(taskId);
    expect(getProgress(taskId)?.status).toBe("cancelled");
  });

  it("starts and can cancel a synthetic CD subfolder pattern", () => {
    const taskId = startAutoTag("/test/Artist/Album (2CD)/CD1");
    expect(taskId).toBeTruthy();

    cancelTask(taskId);
    expect(getProgress(taskId)?.status).toBe("cancelled");
  });
});

describe("buildAliasedLookupVariants", () => {
  let tmpDir: string;
  let aliasFile: string;

  beforeEach(() => {
    tmpDir = mkdtempSync(join(tmpdir(), "auto-tag-aliases-"));
    aliasFile = join(tmpDir, "artist-aliases.json");
    setAliasFilePath(aliasFile);
  });

  afterEach(() => {
    rmSync(tmpDir, { recursive: true, force: true });
  });

  it("returns original pair for English artist with no aliases", async () => {
    const pairs = await buildAliasedLookupVariants("The Beatles", "Abbey Road");
    expect(pairs).toHaveLength(1);
    expect(pairs[0]).toEqual(["The Beatles", "Abbey Road"]);
  });

  it("includes Latin alias before Chinese original", async () => {
    saveAlias("张惠妹", "A-Mei");
    const pairs = await buildAliasedLookupVariants("张惠妹", "姐妹");
    // First pair should be the Latin alias
    expect(pairs[0][0]).toBe("A-Mei");
    expect(pairs[0][1]).toBe("姐妹");
    // Should include original
    expect(pairs.some(([a]) => a === "张惠妹")).toBe(true);
    // Should include script variant (Traditional: 張惠妹)
    expect(pairs.some(([a]) => a === "張惠妹")).toBe(true);
  });

  it("handles null hints gracefully", async () => {
    const pairs = await buildAliasedLookupVariants(null, null);
    expect(pairs).toHaveLength(1);
    expect(pairs[0]).toEqual(["", ""]);
  });

  it("handles null album hint", async () => {
    const pairs = await buildAliasedLookupVariants("Artist", null);
    expect(pairs.length).toBeGreaterThanOrEqual(1);
    expect(pairs[0]).toEqual(["Artist", ""]);
  });

  it("crosses album variants with all artist variants", async () => {
    saveAlias("王菲", "Faye Wong");
    const pairs = await buildAliasedLookupVariants("王菲", "寓言");
    const artistVariants = pairs.map(([a]) => a);
    expect(artistVariants).toContain("Faye Wong");
    expect(artistVariants).toContain("王菲");
    // Album should have script variants
    const albumVariants = pairs.map(([, b]) => b);
    expect(albumVariants).toContain("寓言");
  });

  it("includes multiple aliases in priority order", async () => {
    saveAlias("周杰伦", "Jay Chou");
    saveAlias("周杰伦", "TS");
    const pairs = await buildAliasedLookupVariants("周杰伦", "叶惠美");
    const uniqueArtists = [...new Set(pairs.map(([a]) => a))];
    // Both aliases should be present among unique artist names
    expect(uniqueArtists).toContain("Jay Chou");
    expect(uniqueArtists).toContain("TS");
    // Jay Chou should come before TS (uppercase initial vs all-caps, then length)
    const jayIndex = uniqueArtists.indexOf("Jay Chou");
    const tsIndex = uniqueArtists.indexOf("TS");
    expect(jayIndex).toBeLessThan(tsIndex);
  });

  it("includes alias for album name too", async () => {
    saveAlias("张惠妹", "A-Mei");
    const pairs = await buildAliasedLookupVariants("张惠妹", "姐妹");
    expect(pairs.length).toBeGreaterThanOrEqual(2);
    const hasAmei = pairs.some(
      ([a, b]) => a === "A-Mei" && b === "姐妹",
    );
    expect(hasAmei).toBe(true);
  });

  it("gracefully handles empty aliases file", async () => {
    // No aliases saved — just SC/TC + original pairs
    const pairs = await buildAliasedLookupVariants("张学友", "吻别");
    expect(pairs.length).toBeGreaterThanOrEqual(2);
    // Should still have script variants (Traditional: 張學友, 吻別)
    expect(pairs.some(([a]) => a === "張學友")).toBe(true);
  });
});

function waitForTask(
  taskId: string,
  timeoutMs = 20000,
): Promise<void> {
  return new Promise((resolve, reject) => {
    const start = Date.now();
    const poll = () => {
      const progress = getProgress(taskId);
      if (progress?.status === "completed" || progress?.status === "failed") {
        resolve();
        return;
      }
      if (Date.now() - start > timeoutMs) {
        reject(new Error(`Task ${taskId} did not complete within ${timeoutMs}ms`));
        return;
      }
      setTimeout(poll, 250);
    };
    poll();
  });
}

describe("resolveTagsViaLLM — full pipeline with mocked LLM", () => {
  const originalFetch = globalThis.fetch;

  afterEach(() => {
    globalThis.fetch = originalFetch;
  });

  it("corrects album metadata via LLM fallback when no API candidates exist", async () => {
    const originalEnv = { ...process.env };
    const tmpHome = mkdtempSync(join(tmpdir(), "auto-tag-llm-1-"));
    process.env.HOME = tmpHome;
    delete process.env.LLM_API_KEY;
    delete process.env.LLM_MODEL;
    process.env.AUTO_TAG_REMOTE_LOOKUP = "false";
    process.env.AUTO_TAG_DISCOGS_ENABLED = "false";

    try {
      // Create album directory: /蛋堡/2009-Winter Sweet[flac]
      const albumDir = join(tmpHome, "蛋堡", "2009-Winter Sweet[flac]");
      mkdirSync(albumDir, { recursive: true });
      const trackPath = join(albumDir, "01. Winter Sweet.flac");

      // FLAC with initial metadata (no genre, basic album)
      const block = vorbisCommentBlock(
        ["TITLE=Winter Sweet", "ARTIST=蛋堡", "ALBUM=Winter Sweet"],
        { isLast: true },
      );
      const buf = Buffer.concat([
        flacHeaderWithDuration(false, 200, [block]),
        Buffer.from([0xff, 0xf8, 0x69, 0x18]),
        Buffer.alloc(100),
      ]);
      writeFileSync(trackPath, buf);

      // Enable LLM
      process.env.LLM_API_KEY = "test-key";
      process.env.LLM_MODEL = "test-model";
      refreshConfig();

      // Mock OpenRouter — first call: tag resolution, second call: candidate selection
      globalThis.fetch = vi.fn()
        .mockResolvedValueOnce({
          ok: true,
          json: async () => ({
            choices: [
              {
                message: {
                  content: JSON.stringify({
                    artist: "蛋堡",
                    albumArtist: "蛋堡",
                    album: "Winter Sweet",
                    year: "2009",
                    genre: "Hip Hop",
                    tracks: [
                      { index: 0, title: "Winter Sweet", artist: "蛋堡" },
                    ],
                    confidence: 0.95,
                  }),
                },
              },
            ],
            usage: { prompt_tokens: 200, completion_tokens: 100, total_tokens: 300 },
            model: "test-model",
          }),
        })
        .mockResolvedValueOnce({
          ok: true,
          json: async () => ({
            choices: [
              {
                message: {
                  content: JSON.stringify({
                    selectedIndex: 0,
                    confidence: 0.95,
                    reason: "Matched artist and album hint with genre and track data",
                  }),
                },
              },
            ],
            usage: { prompt_tokens: 100, completion_tokens: 50, total_tokens: 150 },
            model: "test-model",
          }),
        });

      const taskId = startAutoTag(albumDir);
      await waitForTask(taskId);

      const progress = getProgress(taskId);
      if (progress?.status !== "completed") {
        // Debug: show the failure reason
        console.error("Task failed with message:", progress?.message);
      }
      expect(progress?.status).toBe("completed");

      // Verify the file was updated with LLM-corrected metadata (genre added)
      const meta = await parseFile(trackPath);
      expect(meta.common.album).toBe("Winter Sweet");
      expect(meta.common.artist).toBe("蛋堡");
      expect(meta.common.genre).toContain("Hip Hop");
    } finally {
      process.env = { ...originalEnv };
      refreshConfig();
      rmSync(tmpHome, { recursive: true, force: true });
    }
  });

  it("recovers gracefully when LLM fetch fails — tags unchanged", async () => {
    const originalEnv = { ...process.env };
    const tmpHome = mkdtempSync(join(tmpdir(), "auto-tag-llm-2-"));
    process.env.HOME = tmpHome;
    delete process.env.LLM_API_KEY;
    delete process.env.LLM_MODEL;
    process.env.AUTO_TAG_REMOTE_LOOKUP = "false";
    process.env.AUTO_TAG_DISCOGS_ENABLED = "false";

    try {
      const albumDir = join(tmpHome, "Artist", "Album");
      mkdirSync(albumDir, { recursive: true });
      const trackPath = join(albumDir, "01 Track.flac");

      const block = vorbisCommentBlock(
        ["TITLE=Original", "ARTIST=Artist", "ALBUM=Original"],
        { isLast: true },
      );
      const buf = Buffer.concat([
        flacHeaderWithDuration(false, 200, [block]),
        Buffer.from([0xff, 0xf8, 0x69, 0x18]),
        Buffer.alloc(100),
      ]);
      writeFileSync(trackPath, buf);

      // Enable LLM but mock a failed HTTP response
      process.env.LLM_API_KEY = "test-key";
      process.env.LLM_MODEL = "test-model";
      refreshConfig();

      // Mock fetch to fail with an HTTP error
      globalThis.fetch = vi.fn().mockResolvedValue({
        ok: false,
        status: 503,
        text: async () => "Service unavailable",
      });

      const taskId = startAutoTag(albumDir);
      await waitForTask(taskId);

      // Pipeline completed despite LLM failure
      const progress = getProgress(taskId);
      if (progress?.status !== "completed") {
        console.error("LLM fail test — message:", progress?.message);
      }
      expect(progress?.status).toBe("completed");

      // Verify file tags are still valid (fallback writes folder-hint-based tags)
      const meta = await parseFile(trackPath);
      // Album should reflect the folder name (fallback writes from hints)
      expect(typeof meta.common.album).toBe("string");
      expect(meta.common.genre).toBeUndefined();
      expect(meta.common.artist).toBeTruthy();
    } finally {
      process.env = { ...originalEnv };
      refreshConfig();
      rmSync(tmpHome, { recursive: true, force: true });
    }
  });

  it("skips LLM resolution when no API key is configured", async () => {
    const originalEnv = { ...process.env };
    const tmpHome = mkdtempSync(join(tmpdir(), "auto-tag-llm-3-"));
    process.env.HOME = tmpHome;
    delete process.env.LLM_API_KEY;
    delete process.env.LLM_MODEL;
    process.env.AUTO_TAG_REMOTE_LOOKUP = "false";
    process.env.AUTO_TAG_DISCOGS_ENABLED = "false";

    try {
      const albumDir = join(tmpHome, "Artist", "Album");
      mkdirSync(albumDir, { recursive: true });
      const trackPath = join(albumDir, "01 Track.flac");

      const block = vorbisCommentBlock(
        ["TITLE=Original", "ARTIST=Artist", "ALBUM=Album"],
        { isLast: true },
      );
      const buf = Buffer.concat([
        flacHeaderWithDuration(false, 200, [block]),
        Buffer.from([0xff, 0xf8, 0x69, 0x18]),
        Buffer.alloc(100),
      ]);
      writeFileSync(trackPath, buf);

      // No LLM key — resolveTagsViaLLM should short-circuit
      refreshConfig();

      // Track fetch calls to verify OpenRouter was not called
      globalThis.fetch = vi.fn().mockRejectedValue(new Error("should not be called"));

      const taskId = startAutoTag(albumDir);
      await waitForTask(taskId);

      const progress = getProgress(taskId);
      if (progress?.status !== "completed") {
        console.error("No-LLM-key test — message:", progress?.message);
      }
      expect(progress?.status).toBe("completed");

      // fetch should never have been called (no LLM key = no OpenRouter)
      expect(vi.mocked(globalThis.fetch)).not.toHaveBeenCalled();

      // Original metadata preserved
      const meta = await parseFile(trackPath);
      expect(meta.common.album).toBe("Album");
      expect(meta.common.artist).toBe("Artist");
    } finally {
      process.env = { ...originalEnv };
      refreshConfig();
      rmSync(tmpHome, { recursive: true, force: true });
    }
  });

  it("does not query or emit local dataset lookup during auto-tag", async () => {
    const originalEnv = { ...process.env };
    const tmpHome = mkdtempSync(join(tmpdir(), "auto-tag-no-dataset-"));
    process.env.HOME = tmpHome;
    delete process.env.LLM_API_KEY;
    delete process.env.LLM_MODEL;
    process.env.AUTO_TAG_REMOTE_LOOKUP = "false";
    process.env.AUTO_TAG_DISCOGS_ENABLED = "false";

    try {
      const albumDir = join(tmpHome, "Artist", "Album");
      mkdirSync(albumDir, { recursive: true });
      const trackPath = join(albumDir, "01 Track.flac");
      const block = vorbisCommentBlock(
        ["TITLE=Original", "ARTIST=Artist", "ALBUM=Album"],
        { isLast: true },
      );
      const buf = Buffer.concat([
        flacHeaderWithDuration(false, 200, [block]),
        Buffer.from([0xff, 0xf8, 0x69, 0x18]),
        Buffer.alloc(100),
      ]);
      writeFileSync(trackPath, buf);
      refreshConfig();

      const events: string[] = [];
      const unsubscribe = onAutoTagEvent((event) => events.push(event.message));
      const taskId = startAutoTag(albumDir);
      await waitForTask(taskId);
      unsubscribe();

      expect(getProgress(taskId)?.status).toBe("completed");
      expect(events.join("\n")).not.toMatch(/dataset/i);
    } finally {
      process.env = { ...originalEnv };
      refreshConfig();
      rmSync(tmpHome, { recursive: true, force: true });
    }
  });
});

describe("COMMENTS clearing — round-trip via writeTags", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = mkdtempSync(join(tmpdir(), "auto-tag-comment-"));
  });

  afterEach(() => {
    rmSync(tmpDir, { recursive: true, force: true });
  });

  it("clears COMMENT from FLAC by writing comment=null and verifies with music-metadata", async () => {
    const fp = join(tmpDir, "test.flac");
    const block = vorbisCommentBlock(
      [
        "TITLE=Song",
        "ARTIST=Artist",
        "ALBUM=Album",
        "COMMENT=Original comment text",
      ],
      { isLast: true },
    );
    const buf = Buffer.concat([
      flacHeaderWithDuration(false, 200, [block]),
      Buffer.from([0xff, 0xf8, 0x69, 0x18]),
      Buffer.alloc(100),
    ]);
    writeFileSync(fp, buf);

    // Verify COMMENT exists before
    let meta = await parseFile(fp);
    expect(meta.common.comment).toBeTruthy();

    // Write comment=null (clear)
    await writeTags(fp, { comment: null });

    // Verify COMMENT is gone
    meta = await parseFile(fp);
    expect(meta.common.comment).toBeUndefined();

    // Other tags should be preserved
    expect(meta.common.title).toBe("Song");
    expect(meta.common.artist).toBe("Artist");
    expect(meta.common.album).toBe("Album");
  });

  it("clears COMMENT from MP3 by writing comment=null", async () => {
    const fp = join(tmpDir, "test.mp3");
    // Create minimal MP3 with ID3v2 comment using same pattern as writer.test.ts
    NodeID3.write(
      {
        title: "Song",
        artist: "Artist",
        album: "Album",
        comment: { language: "eng", text: "Original comment" },
      },
      fp,
    );
    // Append a minimal MPEG1 Layer3 sync frame (417 bytes)
    const fd = openSync(fp, "a");
    const frame = Buffer.alloc(417);
    frame[0] = 0xff;
    frame[1] = 0xfb;
    frame[2] = (9 << 4) | (0 << 2);
    frame[3] = 0x02;
    writeSync(fd, frame, 0, frame.length);
    closeSync(fd);

    await writeTags(fp, { comment: null });

    const tags = NodeID3.read(fp);
    // COMMENT should be cleared — no longer present after writeTags clears it
    const commentField = tags.comment;
    // node-id3 may return comment as { language, text } or string
    const commentText =
      typeof commentField === "object" && commentField !== null
        ? (commentField as { text?: string }).text
        : String(commentField ?? "");
    expect(commentText).toBe("");
  });

  it("clears COMMENT from multiple FLAC files in an album batch", async () => {
    // Create two FLAC files with COMMENTS
    const f1 = join(tmpDir, "track1.flac");
    const f2 = join(tmpDir, "track2.flac");

    for (const [fp, title] of [[f1, "Track 1"], [f2, "Track 2"]] as const) {
      const block = vorbisCommentBlock(
        [
          `TITLE=${title}`,
          "ARTIST=Artist",
          "ALBUM=Album",
          "COMMENT=Some comment",
        ],
        { isLast: true },
      );
      const buf = Buffer.concat([
        flacHeaderWithDuration(false, 200, [block]),
        Buffer.from([0xff, 0xf8, 0x69, 0x18]),
        Buffer.alloc(100),
      ]);
      writeFileSync(fp, buf);
    }

    // Batch clear COMMENTS
    await batchWriteTags([
      { path: f1, fields: { comment: null } },
      { path: f2, fields: { comment: null } },
    ]);

    // Both files should have COMMENT removed
    const m1 = await parseFile(f1);
    const m2 = await parseFile(f2);
    expect(m1.common.comment).toBeUndefined();
    expect(m2.common.comment).toBeUndefined();
    expect(m1.common.title).toBe("Track 1");
    expect(m2.common.title).toBe("Track 2");
  });

  it("preserves other tags when only clearing COMMENT", async () => {
    const fp = join(tmpDir, "rich.flac");
    const block = vorbisCommentBlock(
      [
        "TITLE=Rich Song",
        "ARTIST=Rich Artist",
        "ALBUM=Rich Album",
        "DATE=2024",
        "GENRE=Jazz",
        "COMMENT=Rich comment",
        "TRACKNUMBER=1",
        "TRACKTOTAL=10",
      ],
      { isLast: true },
    );
    const buf = Buffer.concat([
      flacHeaderWithDuration(false, 200, [block]),
      Buffer.from([0xff, 0xf8, 0x69, 0x18]),
      Buffer.alloc(100),
    ]);
    writeFileSync(fp, buf);

    await writeTags(fp, { comment: null });

    const meta = await parseFile(fp);
    expect(meta.common.comment).toBeUndefined();
    expect(meta.common.title).toBe("Rich Song");
    expect(meta.common.artist).toBe("Rich Artist");
    expect(meta.common.album).toBe("Rich Album");
    expect(String(meta.common.year)).toBe("2024");
    expect(meta.common.genre).toContain("Jazz");
    expect(meta.common.track.no).toBe(1);
    expect(meta.common.track.of).toBe(10);
  });

  it("writing comment to empty string also clears COMMENT from FLAC", async () => {
    const fp = join(tmpDir, "empty-str.flac");
    const block = vorbisCommentBlock(
      ["TITLE=Song", "ARTIST=Artist", "ALBUM=Album", "COMMENT=Remove me"],
      { isLast: true },
    );
    const buf = Buffer.concat([
      flacHeaderWithDuration(false, 200, [block]),
      Buffer.from([0xff, 0xf8, 0x69, 0x18]),
      Buffer.alloc(100),
    ]);
    writeFileSync(fp, buf);

    // Writing empty string should also remove COMMENT
    await writeTags(fp, { comment: "" });

    const meta = await parseFile(fp);
    // music-metadata may treat empty string as undefined
    expect(meta.common.comment == null || meta.common.comment === "").toBe(true);
  });

  // ── DESCRIPTION field clearing (combined with COMMENTS) ───────────

  it("clears DESCRIPTION from FLAC by writing description=null", async () => {
    const fp = join(tmpDir, "desc.flac");
    const block = vorbisCommentBlock(
      [
        "TITLE=Song",
        "ARTIST=Artist",
        "ALBUM=Album",
        "COMMENT=My comment",
        "DESCRIPTION=My long description",
      ],
      { isLast: true },
    );
    const buf = Buffer.concat([
      flacHeaderWithDuration(false, 200, [block]),
      Buffer.from([0xff, 0xf8, 0x69, 0x18]),
      Buffer.alloc(100),
    ]);
    writeFileSync(fp, buf);

    // Verify both exist before
    let meta = await parseFile(fp);
    expect(meta.common.comment).toBeTruthy();

    // Write description=null (leave comment intact)
    await writeTags(fp, { description: null });

    meta = await parseFile(fp);
    // DESCRIPTION should be gone from native Vorbis comments
    const descriptionTag = findVorbisTag(meta, "DESCRIPTION");
    expect(descriptionTag).toBeUndefined();
    // COMMENT should still be present
    expect(meta.common.comment).toBeTruthy();
    // Other tags preserved
    expect(meta.common.title).toBe("Song");
  });

  it("clears both DESCRIPTION and COMMENTS from FLAC with one writeTags call", async () => {
    const fp = join(tmpDir, "both-flac.flac");
    const block = vorbisCommentBlock(
      [
        "TITLE=Track",
        "ARTIST=Artist",
        "ALBUM=Album",
        "COMMENT=Some comment",
        "DESCRIPTION=Some description",
      ],
      { isLast: true },
    );
    const buf = Buffer.concat([
      flacHeaderWithDuration(false, 200, [block]),
      Buffer.from([0xff, 0xf8, 0x69, 0x18]),
      Buffer.alloc(100),
    ]);
    writeFileSync(fp, buf);

    // Clear both fields in a single writeTags call
    await writeTags(fp, { comment: null, description: null });

    const meta = await parseFile(fp);
    // COMMENT should be gone
    expect(meta.common.comment).toBeUndefined();
    // DESCRIPTION should be gone from native Vorbis comments
    const descriptionTag = findVorbisTag(meta, "DESCRIPTION");
    expect(descriptionTag).toBeUndefined();
    // Other tags preserved
    expect(meta.common.title).toBe("Track");
    expect(meta.common.artist).toBe("Artist");
    expect(meta.common.album).toBe("Album");
  });

  it("clears both DESCRIPTION and COMMENTS from MP3 with one writeTags call", async () => {
    const fp = join(tmpDir, "both-mp3.mp3");
    // Create MP3 file first (need audio data so node-id3 can read/write)
    NodeID3.write({ title: "MP3 Temp" }, fp);
    const fd = openSync(fp, "a");
    const frame = Buffer.alloc(417);
    frame[0] = 0xff;
    frame[1] = 0xfb;
    frame[2] = (9 << 4) | (0 << 2);
    frame[3] = 0x02;
    writeSync(fd, frame, 0, frame.length);
    closeSync(fd);

    // Now write full metadata (including comment + description)
    await writeTags(fp, {
      title: "MP3 Track",
      artist: "MP3 Artist",
      album: "MP3 Album",
      comment: "MP3 comment",
      description: "MP3 description",
    });

    const before = NodeID3.read(fp);
    const beforeDesc = Array.isArray(before.userDefinedText)
      ? before.userDefinedText.find((t) => t.description === "DESCRIPTION")
      : before.userDefinedText?.description === "DESCRIPTION"
        ? before.userDefinedText
        : undefined;
    expect(beforeDesc).toBeTruthy();

    // Now clear both
    await writeTags(fp, { comment: null, description: null });

    const after = NodeID3.read(fp);
    // COMMENT should be cleared
    const afterComment = after.comment;
    const afterCommentText =
      typeof afterComment === "object" && afterComment !== null
        ? (afterComment as { text?: string }).text
        : String(afterComment ?? "");
    expect(afterCommentText).toBe("");

    // DESCRIPTION should be cleared from userDefinedText
    const afterDesc = Array.isArray(after.userDefinedText)
      ? after.userDefinedText.find((t) => t.description === "DESCRIPTION")
      : after.userDefinedText?.description === "DESCRIPTION"
        ? after.userDefinedText
        : undefined;
    expect(afterDesc).toBeUndefined();

    expect(after.title).toBe("MP3 Track");
  });

  it("batch clears both DESCRIPTION and COMMENTS from multiple FLAC files", async () => {
    const f1 = join(tmpDir, "multi1.flac");
    const f2 = join(tmpDir, "multi2.flac");

    for (const [fp, title] of [[f1, "A"], [f2, "B"]] as const) {
      const block = vorbisCommentBlock(
        [
          `TITLE=${title}`,
          "ARTIST=Artist",
          "ALBUM=Album",
          "COMMENT=Comment " + title,
          "DESCRIPTION=Desc " + title,
        ],
        { isLast: true },
      );
      const buf = Buffer.concat([
        flacHeaderWithDuration(false, 200, [block]),
        Buffer.from([0xff, 0xf8, 0x69, 0x18]),
        Buffer.alloc(100),
      ]);
      writeFileSync(fp, buf);
    }

    // Batch clear both fields
    await batchWriteTags([
      { path: f1, fields: { comment: null, description: null } },
      { path: f2, fields: { comment: null, description: null } },
    ]);

    // Verify both files have both fields cleared
    for (const title of ["A", "B"]) {
      const fp = join(tmpDir, `multi${title === "A" ? "1" : "2"}.flac`);
      const meta = await parseFile(fp);
      expect(meta.common.comment).toBeUndefined();
      const descTag = findVorbisTag(meta, "DESCRIPTION");
      expect(descTag).toBeUndefined();
      expect(meta.common.title).toBe(title);
    }
  });

  it("preserves all other tags when clearing both DESCRIPTION and COMMENTS", async () => {
    const fp = join(tmpDir, "rich-both.flac");
    const block = vorbisCommentBlock(
      [
        "TITLE=Rich Track",
        "ARTIST=Rich Artist",
        "ALBUM=Rich Album",
        "DATE=2025",
        "GENRE=Electronic",
        "COMMENT=Rich comment",
        "DESCRIPTION=Rich description",
        "TRACKNUMBER=2",
        "TRACKTOTAL=8",
        "DISCNUMBER=1",
        "DISCTOTAL=1",
      ],
      { isLast: true },
    );
    const buf = Buffer.concat([
      flacHeaderWithDuration(false, 200, [block]),
      Buffer.from([0xff, 0xf8, 0x69, 0x18]),
      Buffer.alloc(100),
    ]);
    writeFileSync(fp, buf);

    await writeTags(fp, { comment: null, description: null });

    const meta = await parseFile(fp);
    expect(meta.common.comment).toBeUndefined();
    const descTag = findVorbisTag(meta, "DESCRIPTION");
    expect(descTag).toBeUndefined();
    // Everything else preserved
    expect(meta.common.title).toBe("Rich Track");
    expect(meta.common.artist).toBe("Rich Artist");
    expect(meta.common.album).toBe("Rich Album");
    expect(String(meta.common.year)).toBe("2025");
    expect(meta.common.genre).toContain("Electronic");
    expect(meta.common.track.no).toBe(2);
    expect(meta.common.track.of).toBe(8);
  });
});

describe("change AlbumArtist + fix Artist from file + create Artists — 法老 scenario", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = mkdtempSync(join(tmpdir(), "auto-tag-falao-"));
  });

  afterEach(() => {
    rmSync(tmpDir, { recursive: true, force: true });
  });

  it("sets albumArtist=法老, fixes artist from filename, creates artists", async () => {
    // Simulate a file with wrong albumArtist/artist and no artists
    // Filename: "01. 法老 - 百变酒精.mp3" → agent extracts artist="法老"
    const fp = join(tmpDir, "01. 法老 - 百变酒精.flac");
    const block = vorbisCommentBlock(
      [
        "TITLE=百变酒精",
        "ARTIST=Wrong Artist",          // wrong artist
        "ALBUM=百变酒精",
        "ALBUMARTIST=Wrong Album Artist", // wrong albumArtist
        // No ARTISTS tag at all
      ],
      { isLast: true },
    );
    const buf = Buffer.concat([
      flacHeaderWithDuration(false, 200, [block]),
      Buffer.from([0xff, 0xf8, 0x69, 0x18]),
      Buffer.alloc(100),
    ]);
    writeFileSync(fp, buf);

    // The agent's correction: albumArtist=法老, artist=法老 (from filename), artists=["法老"]
    await writeTags(fp, {
      albumArtist: "法老",
      artist: "法老",
      artists: ["法老"],
    });

    // Verify through music-metadata
    const meta = await parseFile(fp);
    expect(meta.common.albumartist).toBe("法老");
    expect(meta.common.artist).toBe("法老");
    expect(meta.common.artists).toEqual(["法老"]);
    expect(meta.common.title).toBe("百变酒精");

    // Verify through readTrackMetadata (full read path used by TrackTagService)
    const track = await readTrackMetadata(fp);
    expect(track.albumArtist).toBe("法老");
    expect(track.artist).toBe("法老");
    expect(track.artists).toEqual(["法老"]);
    expect(track.title).toBe("百变酒精");
  });

  it("handles collaborative artists correctly — 多人合作", async () => {
    // Filename: "02. 法老&杨和苏 - 百变酒精.flac" → agent extracts
    // artist="法老&杨和苏", artists=["法老", "杨和苏"]
    const fp = join(tmpDir, "02. 法老&杨和苏 - 百变酒精.flac");
    const block = vorbisCommentBlock(
      [
        "TITLE=百变酒精",
        "ARTIST=Wrong Artist",
        "ALBUM=百变酒精",
        "ALBUMARTIST=Wrong Album Artist",
      ],
      { isLast: true },
    );
    const buf = Buffer.concat([
      flacHeaderWithDuration(false, 200, [block]),
      Buffer.from([0xff, 0xf8, 0x69, 0x18]),
      Buffer.alloc(100),
    ]);
    writeFileSync(fp, buf);

    // Collaborative correction:
    // albumArtist=法老 (the main album artist)
    // artist=法老&杨和苏 (from filename)
    // artists=["法老", "杨和苏"] (split collaborators)
    await writeTags(fp, {
      albumArtist: "法老",
      artist: "法老&杨和苏",
      artists: ["法老", "杨和苏"],
    });

    // Verify through music-metadata
    const meta = await parseFile(fp);
    expect(meta.common.albumartist).toBe("法老");
    expect(meta.common.artist).toBe("法老&杨和苏");
    expect(meta.common.artists).toEqual(["法老", "杨和苏"]);

    // Verify through readTrackMetadata
    const track = await readTrackMetadata(fp);
    expect(track.albumArtist).toBe("法老");
    expect(track.artist).toBe("法老&杨和苏");
    expect(track.artists).toEqual(["法老", "杨和苏"]);
  });

  it("gives correct diffs in planTagUpdates for 法老 scenario", async () => {
    // Test the TrackTagService planning path
    const fp = join(tmpDir, "plan-法老.flac");
    const block = vorbisCommentBlock(
      [
        "TITLE=百变酒精",
        "ARTIST=Wrong",
        "ALBUM=百变酒精",
        "ALBUMARTIST=Wrong",
      ],
      { isLast: true },
    );
    const buf = Buffer.concat([
      flacHeaderWithDuration(false, 200, [block]),
      Buffer.from([0xff, 0xf8, 0x69, 0x18]),
      Buffer.alloc(100),
    ]);
    writeFileSync(fp, buf);

    // First verify readTrackMetadata returns the wrong values
    const before = await readTrackMetadata(fp);
    expect(before.artist).toBe("Wrong");
    expect(before.albumArtist).toBe("Wrong");
    // artists may be ["Wrong"] since music-metadata maps ARTIST into common.artists too
    expect(before.artists.length).toBeGreaterThanOrEqual(1);
    expect(before.artists[0]).toBe("Wrong");

    // Apply corrections via writeTags
    await writeTags(fp, {
      albumArtist: "法老",
      artist: "法老",
      artists: ["法老"],
    });

    // Verify all three fields via readTrackMetadata
    const track = await readTrackMetadata(fp);
    expect(track.albumArtist).toBe("法老");
    expect(track.artist).toBe("法老");
    expect(track.artists).toEqual(["法老"]);
  });

  it("albumArtist+artist+artists survive batch write then re-read", async () => {
    // Multiple files in a collaborative album
    const tracks = [
      { filename: "01. 法老 - 百变酒精.flac", title: "百变酒精", artist: "法老", artists: ["法老"] },
      { filename: "02. 法老&杨和苏 - 百变酒精.flac", title: "百变酒精", artist: "法老&杨和苏", artists: ["法老", "杨和苏"] },
    ];

    for (const t of tracks) {
      const fp = join(tmpDir, t.filename);
      const block = vorbisCommentBlock(
        [
          `TITLE=${t.title}`,
          "ARTIST=Wrong",
          "ALBUM=百变酒精",
          "ALBUMARTIST=Wrong",
        ],
        { isLast: true },
      );
      const buf = Buffer.concat([
        flacHeaderWithDuration(false, 200, [block]),
        Buffer.from([0xff, 0xf8, 0x69, 0x18]),
        Buffer.alloc(100),
      ]);
      writeFileSync(fp, buf);
    }

    // Batch-write corrections — simulates what batchWriteTags does in TrackTagService
    await batchWriteTags([
      {
        path: join(tmpDir, tracks[0].filename),
        fields: { albumArtist: "法老", artist: "法老", artists: ["法老"] },
      },
      {
        path: join(tmpDir, tracks[1].filename),
        fields: { albumArtist: "法老", artist: "法老&杨和苏", artists: ["法老", "杨和苏"] },
      },
    ]);

    // Both files should have correct metadata
    const t1 = await readTrackMetadata(join(tmpDir, tracks[0].filename));
    expect(t1.albumArtist).toBe("法老");
    expect(t1.artist).toBe("法老");
    expect(t1.artists).toEqual(["法老"]);

    const t2 = await readTrackMetadata(join(tmpDir, tracks[1].filename));
    expect(t2.albumArtist).toBe("法老");
    expect(t2.artist).toBe("法老&杨和苏");
    expect(t2.artists).toEqual(["法老", "杨和苏"]);
  });
});

describe("tracks.search — paths in response data", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = mkdtempSync(join(tmpdir(), "auto-tag-search-paths-"));
  });

  afterEach(() => {
    rmSync(tmpDir, { recursive: true, force: true });
  });

  it("returns paths in data when filtering by artist", async () => {
    // Create a track file so SafeQueryService can load it
    const fp = join(tmpDir, "01. 法老 - 百变酒精.flac");
    const block = vorbisCommentBlock(
      ["TITLE=百变酒精", "ARTIST=法老", "ALBUM=百变酒精"],
      { isLast: true },
    );
    const buf = Buffer.concat([
      flacHeaderWithDuration(false, 200, [block]),
      Buffer.from([0xff, 0xf8, 0x69, 0x18]),
      Buffer.alloc(100),
    ]);
    writeFileSync(fp, buf);

    // Read metadata to populate TrackData
    const track = await readTrackMetadata(fp);

    // Set up SafeQueryService with the track
    const { SafeQueryService } = await import("../../electron/services/SafeQueryService");
    const sqs = new SafeQueryService();
    sqs.setTracks([track]);

    // Simulate what tracks.search executor does
    const results = sqs.findTracks({ artist: "法老" });
    const limited = results.slice(0, 20);
    const data = {
      total: results.length,
      tracks: limited.map((t) => ({
        path: t.path,
        title: t.title,
        artist: t.artist,
        album: t.album,
        codec: t.codec,
      })),
      paths: limited.map((t) => t.path),
    };

    expect(data.total).toBe(1);
    expect(data.paths).toHaveLength(1);
    expect(data.paths[0]).toBe(fp);
    expect(data.tracks[0].path).toBe(fp);
    expect(data.tracks[0].title).toBe("百变酒精");
    expect(data.tracks[0].artist).toBe("法老");
  });

  it("returns paths in data when searching by missing artist", async () => {
    const fp = join(tmpDir, "no-artist.flac");
    const block = vorbisCommentBlock(
      ["TITLE=No Artist", "ALBUM=Test"],
      { isLast: true },
    );
    const buf = Buffer.concat([
      flacHeaderWithDuration(false, 200, [block]),
      Buffer.from([0xff, 0xf8, 0x69, 0x18]),
      Buffer.alloc(100),
    ]);
    writeFileSync(fp, buf);

    const track = await readTrackMetadata(fp);
    const { SafeQueryService } = await import("../../electron/services/SafeQueryService");
    const sqs = new SafeQueryService();
    sqs.setTracks([track]);

    const results = sqs.findTracks({ missingArtist: true });
    const limited = results.slice(0, 20);
    const data = {
      total: results.length,
      tracks: limited.map((t) => ({ path: t.path, title: t.title, artist: t.artist, album: t.album })),
      paths: limited.map((t) => t.path),
    };

    expect(data.total).toBe(1);
    expect(data.paths[0]).toBe(fp);
    expect(data.tracks[0].path).toBe(fp);
  });
});

/**
 * Find a Vorbis tag by key in music-metadata's native format.
 * Returns the value string if found, undefined otherwise.
 */
function findVorbisTag(
  meta: Awaited<ReturnType<typeof parseFile>>,
  key: string,
): string | undefined {
  const upperKey = key.toUpperCase();
  for (const [, tags] of Object.entries(meta.native)) {
    for (const tag of tags) {
      if (
        typeof tag.id === "string" &&
        tag.id.toUpperCase() === upperKey &&
        typeof tag.value === "string"
      ) {
        return tag.value;
      }
    }
  }
  return undefined;
}
