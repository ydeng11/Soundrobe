/**
 * Integration test: group_by_album via FolderOrganizerService.
 *
 * Verifies that tracks with different album metadata are correctly
 * organized into album-named subdirectories.
 */

import { describe, it, expect, beforeEach, afterEach } from "vitest";
import fs from "fs";
import path from "path";
import os from "os";
import { FolderOrganizerService } from "../../electron/services/FolderOrganizerService";

function touchSync(filePath: string): void {
  fs.mkdirSync(path.dirname(filePath), { recursive: true });
  fs.writeFileSync(filePath, "dummy content", "utf-8");
}

describe("group_by_album via FolderOrganizerService", () => {
  let root: string;
  let service: FolderOrganizerService;

  beforeEach(() => {
    root = fs.mkdtempSync(path.join(os.tmpdir(), "grp-by-album-"));
    service = new FolderOrganizerService();
    service.setLibraryRoot(root);
  });

  afterEach(() => {
    fs.rmSync(root, { recursive: true, force: true });
  });

  it("groups 6 tracks across 3 albums into album folders under library root", async () => {
    const trackNames: Record<string, string[]> = {
      "Album A": ["Intro A", "Main A"],
      "Album B": ["Intro B", "Main B"],
      "Album C": ["Intro C", "Main C"],
    };
    const albums = Object.keys(trackNames);
    const albumTitleFn = (trackPath: string): string | null => {
      const dir = path.dirname(trackPath);
      const base = path.basename(trackPath, ".txt");
      for (const album of albums) {
        if (trackNames[album].includes(base)) return album;
      }
      return null;
    };

    // Create 6 flat files in a temp subdirectory
    const sourceDir = path.join(root, "Incoming");
    fs.mkdirSync(sourceDir, { recursive: true });
    const trackPaths: string[] = [];
    for (const album of albums) {
      for (const trackName of trackNames[album]) {
        const filePath = path.join(sourceDir, `${trackName}.txt`);
        touchSync(filePath);
        trackPaths.push(filePath);
      }
    }

    expect(trackPaths).toHaveLength(6);

    // Plan album grouping
    const plan = service.planGroupByAlbum({
      trackPaths,
      libraryRoot: root,
      albumTitleFn,
    });

    // Verify the plan
    expect(plan.kind).toBe("folder-move");
    expect(plan.moves).toHaveLength(6);
    expect(plan.affectedTracks).toBe(6);

    // Each move should target an album-named folder under library root
    for (const move of plan.moves) {
      const dir = path.dirname(move.destinationPath);
      expect(albums).toContain(path.basename(dir));
      expect(dir.startsWith(root)).toBe(true);
    }

    // Apply the moves
    const result = await service.applyMoves(plan);
    expect(result.results).toHaveLength(6);
    expect(result.results.every((r) => r.success)).toBe(true);

    // Verify folder structure
    for (const album of albums) {
      const albumDir = path.join(root, album);
      expect(fs.existsSync(albumDir), `Album folder "${album}" should exist`).toBe(true);

      for (const trackName of trackNames[album]) {
        const trackPath = path.join(albumDir, `${trackName}.txt`);
        expect(fs.existsSync(trackPath), `Track "${trackName}" should be in "${album}/"`).toBe(true);
      }
    }

    // Original flat files should be gone
    expect(fs.readdirSync(sourceDir).filter((f) => f.endsWith(".txt"))).toHaveLength(0);
  });

  it("skips tracks with missing album metadata", () => {
    const trackPaths = [
      path.join(root, "Incoming", "known.txt"),
      path.join(root, "Incoming", "unknown.txt"),
    ];
    touchSync(trackPaths[0]);
    touchSync(trackPaths[1]);

    const albumTitleFn = (trackPath: string): string | null => {
      return path.basename(trackPath).startsWith("known") ? "Known Album" : null;
    };

    const plan = service.planGroupByAlbum({
      trackPaths,
      libraryRoot: root,
      albumTitleFn,
    });

    expect(plan.moves).toHaveLength(1);
    expect(plan.skipped).toHaveLength(1);
    expect(plan.skipped[0].skipReason).toContain("Missing album title");
  });

  it("reports no-ops for files already in the correct album folder", () => {
    // Create a track already in its album folder
    const albumDir = path.join(root, "Existing Album");
    fs.mkdirSync(albumDir, { recursive: true });
    const existingTrack = path.join(albumDir, "song.txt");
    touchSync(existingTrack);

    const albumTitleFn = () => "Existing Album";

    const plan = service.planGroupByAlbum({
      trackPaths: [existingTrack],
      libraryRoot: root,
      albumTitleFn,
    });

    expect(plan.moves).toHaveLength(0);
    expect(plan.noops).toHaveLength(1);
    expect(plan.noops[0].skipReason).toContain("Already in correct album folder");
  });
});
