// @vitest-environment node
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import fs from "fs";
import os from "os";
import path from "path";

vi.mock("electron", () => ({
  ipcMain: {
    handle: vi.fn(),
  },
  BrowserWindow: {
    getAllWindows: vi.fn(() => []),
  },
}));

import { collectAudioFilesForAudit, discoverAlbumDirs } from "../../electron/handlers/audit";

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
