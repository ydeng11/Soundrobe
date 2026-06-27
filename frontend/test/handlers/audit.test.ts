// @vitest-environment node
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import fs from "fs";
import os from "os";
import path from "path";

const { parseFileMock } = vi.hoisted(() => ({
  parseFileMock: vi.fn(),
}));

vi.mock("electron", () => ({
  ipcMain: {
    handle: vi.fn(),
  },
  BrowserWindow: {
    getAllWindows: vi.fn(() => []),
  },
}));

vi.mock("music-metadata", () => ({
  parseFile: parseFileMock,
}));

vi.mock("../../electron/handlers/auto-tag", async () => {
  const actual = await vi.importActual<typeof import("../../electron/handlers/auto-tag")>("../../electron/handlers/auto-tag");
  return {
    ...actual,
    loadConfig: vi.fn(() => ({
      remoteLookupEnabled: true,
      discogsEnabled: true,
    })),
  };
});

import { applyAuditFixes, auditAlbum, auditSpecificAlbums, collectAudioFilesForAudit, discoverAlbumDirs } from "../../electron/handlers/audit";
import { getDefaultWriteQueue } from "../../electron/services/TagWriteQueue";

vi.mock("../../electron/services/TagWriteQueue", () => ({
  getDefaultWriteQueue: vi.fn(),
}));

describe("discoverAlbumDirs", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "audit-discovery-"));
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  it("treats the selected root as an album when audio files are at the root", () => {
    fs.writeFileSync(path.join(tmpDir, "01 song.flac"), "data");

    expect(discoverAlbumDirs(tmpDir)).toEqual([tmpDir]);
  });

  it("discovers direct album folders and nested artist/album folders", () => {
    const directAlbum = path.join(tmpDir, "Direct Album");
    const nestedAlbum = path.join(tmpDir, "Artist", "Nested Album");
    fs.mkdirSync(directAlbum, { recursive: true });
    fs.mkdirSync(nestedAlbum, { recursive: true });
    fs.writeFileSync(path.join(directAlbum, "01 direct.mp3"), "data");
    fs.writeFileSync(path.join(nestedAlbum, "01 nested.flac"), "data");

    expect(discoverAlbumDirs(tmpDir)).toEqual([nestedAlbum, directAlbum]);
  });
});

describe("collectAudioFilesForAudit", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "audit-audio-files-"));
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  it("returns absolute file paths so metadata parsing can open the files", () => {
    const filePath = path.join(tmpDir, "01 song.flac");
    fs.writeFileSync(filePath, "data");

    expect(collectAudioFilesForAudit(tmpDir)).toEqual([filePath]);
  });
});

describe("applyAuditFixes", () => {
  afterEach(() => {
    vi.clearAllMocks();
  });

  it("writes only audit findings explicitly marked auto-fix eligible", async () => {
    const submit = vi.fn().mockResolvedValue([{ filePath: "/tmp/one.flac", success: true }]);
    vi.mocked(getDefaultWriteQueue).mockReturnValue({ submit } as any);

    const results = [
      {
        index: 0,
        field: "title",
        status: "error" as const,
        message: "Title mismatch",
        corrected: { title: "Correct" },
        autoFixEligible: true,
      },
      {
        index: 1,
        field: "album",
        status: "warning" as const,
        message: "Album might be wrong",
        suggestion: "Maybe",
        corrected: { album: "Maybe" },
        autoFixEligible: false,
      },
    ];

    const fixed = await applyAuditFixes(["/tmp/one.flac", "/tmp/two.flac"], results);

    expect(fixed).toBe(1);
    expect(submit).toHaveBeenCalledWith([
      { filePath: "/tmp/one.flac", fields: { title: "Correct" } },
    ]);
    expect(results[0].autoFixed).toBe(true);
    expect(results[1].autoFixed).toBe(false);
  });

  it("merges multiple eligible findings for one file into one write job", async () => {
    const submit = vi.fn().mockResolvedValue([{ filePath: "/tmp/one.flac", success: true }]);
    vi.mocked(getDefaultWriteQueue).mockReturnValue({ submit } as any);

    const results = [
      {
        index: 0,
        field: "title",
        status: "error" as const,
        message: "Title mismatch",
        corrected: { title: "Correct" },
        autoFixEligible: true,
      },
      {
        index: 0,
        field: "album",
        status: "error" as const,
        message: "Album mismatch",
        corrected: { album: "Album" },
        autoFixEligible: true,
      },
    ];

    const fixed = await applyAuditFixes(["/tmp/one.flac"], results);

    expect(fixed).toBe(1);
    expect(submit).toHaveBeenCalledWith([
      { filePath: "/tmp/one.flac", fields: { title: "Correct", album: "Album" } },
    ]);
    expect(results.every((result) => result.autoFixed)).toBe(true);
  });

  it("marks only successful file jobs as auto-fixed", async () => {
    const submit = vi.fn().mockResolvedValue([
      { filePath: "/tmp/one.flac", success: false, error: "write failed" },
      { filePath: "/tmp/two.flac", success: true },
    ]);
    vi.mocked(getDefaultWriteQueue).mockReturnValue({ submit } as any);

    const results = [
      {
        index: 0,
        field: "title",
        status: "error" as const,
        message: "Title mismatch",
        corrected: { title: "Correct" },
        autoFixEligible: true,
      },
      {
        index: 1,
        field: "album",
        status: "error" as const,
        message: "Album mismatch",
        corrected: { album: "Album" },
        autoFixEligible: true,
      },
    ];

    const fixed = await applyAuditFixes(["/tmp/one.flac", "/tmp/two.flac"], results);

    expect(fixed).toBe(1);
    expect(results[0].autoFixed).toBe(false);
    expect(results[1].autoFixed).toBe(true);
  });
});

describe("auditAlbum orchestration", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "audit-album-"));
    vi.mocked(getDefaultWriteQueue).mockReturnValue({
      submit: vi.fn().mockImplementation(async (jobs: Array<{ filePath: string }>) =>
        jobs.map((job) => ({ filePath: job.filePath, success: true })),
      ),
    } as any);
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
    vi.clearAllMocks();
  });

  it("auto-fixes deterministic findings without calling the LLM", async () => {
    const albumPath = path.join(tmpDir, "Artist", "2020 - Album");
    fs.mkdirSync(albumPath, { recursive: true });
    fs.writeFileSync(path.join(albumPath, "01. Song.flac"), "data");
    parseFileMock.mockResolvedValue({
      common: {
        title: "Wrong",
        artist: "Artist",
        artists: ["Artist"],
        album: "Wrong Album",
        albumartist: "Artist",
        year: 2019,
        genre: ["Pop"],
        track: { no: 9, of: null },
        disk: { no: null, of: null },
      },
    });
    const client = { completeJson: vi.fn() };

    const results = await auditAlbum(client as any, albumPath);

    expect(client.completeJson).not.toHaveBeenCalled();
    expect(results).toEqual(expect.arrayContaining([
      expect.objectContaining({
        field: "title",
        source: "deterministic",
        autoFixEligible: true,
        autoFixed: true,
      }),
      expect.objectContaining({
        field: "album",
        source: "deterministic",
        autoFixEligible: true,
        autoFixed: true,
      }),
    ]));
  });

  it("does not treat a disc folder as the album when fixing selected-track audits", async () => {
    const albumPath = path.join(tmpDir, "Artist", "Album", "Disc 1");
    fs.mkdirSync(albumPath, { recursive: true });
    fs.writeFileSync(path.join(albumPath, "01. Song.flac"), "data");
    parseFileMock.mockResolvedValue({
      common: {
        title: "Song",
        artist: "Artist",
        artists: ["Artist"],
        album: "Album",
        albumartist: "Artist",
        genre: ["Pop"],
        track: { no: 1, of: null },
        disk: { no: null, of: null },
      },
    });

    const results = await auditAlbum(null, albumPath);

    expect(results.some((result) => result.field === "album")).toBe(false);
    expect(results.some((result) => result.field === "albumArtist")).toBe(false);
    expect(results).toEqual(expect.arrayContaining([
      expect.objectContaining({
        field: "discNumber",
        autoFixEligible: true,
        autoFixed: true,
        corrected: { discNumber: 1 },
      }),
    ]));
  });

  it("surfaces unresolved semantic review targets as warnings when no LLM client is configured", async () => {
    const submit = vi.fn().mockResolvedValue([]);
    vi.mocked(getDefaultWriteQueue).mockReturnValue({ submit } as any);
    const albumPath = path.join(tmpDir, "Artist", "Album");
    fs.mkdirSync(albumPath, { recursive: true });
    fs.writeFileSync(path.join(albumPath, "01. Song.flac"), "data");
    parseFileMock.mockResolvedValue({
      common: {
        title: "Song",
        artist: "Artist",
        artists: ["Artist"],
        album: "Album",
        albumartist: "Artist",
        track: { no: 1, of: null },
        disk: { no: null, of: null },
      },
    });

    const results = await auditAlbum(null, albumPath);

    expect(results).toEqual(expect.arrayContaining([
      expect.objectContaining({
        field: "genre",
        status: "warning",
        message: "Genre tag is empty.",
        autoFixEligible: false,
        autoFixed: false,
      }),
    ]));
    expect(submit).not.toHaveBeenCalled();
  });

  it("calls the LLM only with targeted suspicious fields and downgrades low-confidence corrections", async () => {
    const albumPath = path.join(tmpDir, "Artist", "Album");
    fs.mkdirSync(albumPath, { recursive: true });
    fs.writeFileSync(path.join(albumPath, "Song.flac"), "data");
    parseFileMock.mockResolvedValue({
      common: {
        title: "Song",
        artist: "AC/DC",
        artists: [],
        album: "Album",
        albumartist: "Artist",
        genre: ["Rock"],
        track: { no: null, of: null },
        disk: { no: null, of: null },
      },
    });
    const client = {
      completeJson: vi.fn().mockResolvedValue({
        data: {
          tracks: [
            {
              index: 0,
              field: "artists",
              status: "error",
              message: "Split artist",
              corrected: { artists: ["AC", "DC"], title: "Other" },
              confidence: 0.99,
            },
            {
              index: 0,
              field: "genre",
              status: "error",
              message: "Genre missing",
              corrected: { genre: "Hard Rock" },
              confidence: 0.5,
            },
          ],
        },
      }),
    };

    const results = await auditAlbum(client as any, albumPath);
    const payload = JSON.parse(client.completeJson.mock.calls[0][0][1].content);
    const schema = client.completeJson.mock.calls[0][2];

    expect(payload.review_targets.map((target: { field: string }) => target.field)).toEqual(["artists"]);
    expect(schema.properties.tracks.items.required).toContain("confidence");
    expect(results).toEqual(expect.arrayContaining([
      expect.objectContaining({
        field: "artists",
        source: "llm",
        corrected: { artists: ["AC", "DC"] },
        autoFixEligible: false,
        autoFixed: false,
      }),
      expect.objectContaining({
        field: "genre",
        source: "llm",
        autoFixEligible: false,
        autoFixed: false,
      }),
    ]));
  });

  it("returns per-album results so the renderer is not dependent on event timing", async () => {
    const albumPath = path.join(tmpDir, "Artist", "2020 - Album");
    fs.mkdirSync(albumPath, { recursive: true });
    fs.writeFileSync(path.join(albumPath, "01. Song.flac"), "data");
    parseFileMock.mockResolvedValue({
      common: {
        title: "Wrong",
        artist: "Artist",
        artists: ["Artist"],
        album: "Wrong Album",
        albumartist: "Artist",
        year: 2019,
        genre: ["Pop"],
        track: { no: 9, of: null },
        disk: { no: null, of: null },
      },
    });

    const summary = await auditSpecificAlbums(null, [albumPath]);

    expect(summary.albums).toBe(1);
    expect(summary.issues).toBeGreaterThan(0);
    expect(summary.albumResults).toEqual([
      expect.objectContaining({
        albumPath,
        results: expect.arrayContaining([
          expect.objectContaining({
            field: "title",
            autoFixed: true,
          }),
        ]),
      }),
    ]);
  });
});
