/**
 * Integration test: folder group by album (assistant tool path).
 *
 * Creates real temp audio files with known album metadata, uses the
 * FolderOrganizerService to plan and apply album-title groupings,
 * then verifies the files land in the correct directories.
 */

import { describe, it, expect, beforeAll, afterAll } from "vitest";
import fs from "fs";
import path from "path";
import { mkdtempSync } from "fs";
import { FolderOrganizerService } from "../../electron/services/FolderOrganizerService";

// ── Helpers ──────────────────────────────────────────────────────

let tempDir: string | null = null;

function createTempDir(): string {
  const dir = mkdtempSync("/tmp/auto-tagger-test-");
  tempDir = dir;
  return dir;
}

function touchSync(filePath: string): void {
  const dir = path.dirname(filePath);
  if (!fs.existsSync(dir)) {
    fs.mkdirSync(dir, { recursive: true });
  }
  fs.writeFileSync(filePath, "dummy audio content", "utf-8");
}

/** Album tag lookup table: track path => album title. */
type AlbumLookup = Record<string, string>;

function makeAlbumFn(lookup: AlbumLookup): (trackPath: string) => string | null {
  return (trackPath: string) => {
    const resolved = path.resolve(trackPath);
    return lookup[resolved] ?? null;
  };
}

/** Directory contents (basenames) sorted. */
function dirContents(dirPath: string): string[] {
  if (!fs.existsSync(dirPath)) return [];
  return fs.readdirSync(dirPath).sort();
}

// ── Test ─────────────────────────────────────────────────────────

describe("assistant folder-group integration", () => {
  const albumLookup: AlbumLookup = {};
  let libRoot: string;
  let folderOrg: FolderOrganizerService;

  // Track full paths for all test files
  let song1: string;
  let song2: string;
  let song3: string;
  let song4: string;
  let song5: string;
  let song6: string;
  let song7: string;

  beforeAll(() => {
    const base = createTempDir();
    libRoot = path.join(base, "library");

    // File layout:
    //   library/
    //     Artist - Album 1/
    //       song1.mp3  (album: "Album 1")
    //       song2.mp3  (album: "Album 1")
    //     Album 2/
    //       song3.mp3  (album: "Album 2")
    //     song4.mp3    (album: "Album 1"  → needs moving into Album 1/)
    //     song5.mp3    (album: "Album 3"  → needs new folder Album 3/)
    //     song6.mp3    (album: ""         → skipped, no album tag)
    //     song7.mp3    (album: "Album 2"  → needs moving into Album 2/)

    song1 = path.join(libRoot, "Artist - Album 1", "song1.mp3");
    song2 = path.join(libRoot, "Artist - Album 1", "song2.mp3");
    song3 = path.join(libRoot, "Album 2",          "song3.mp3");
    song4 = path.join(libRoot,                     "song4.mp3");
    song5 = path.join(libRoot,                     "song5.mp3");
    song6 = path.join(libRoot,                     "song6.mp3");
    song7 = path.join(libRoot,                     "song7.mp3");

    for (const f of [song1, song2, song3, song4, song5, song6, song7]) {
      touchSync(f);
    }

    // Album tag lookup (simulates reading track metadata)
    albumLookup[path.resolve(song1)] = "Album 1";
    albumLookup[path.resolve(song2)] = "Album 1";
    albumLookup[path.resolve(song3)] = "Album 2";
    albumLookup[path.resolve(song4)] = "Album 1";  // loose file → move into Album 1/
    albumLookup[path.resolve(song5)] = "Album 3";  // loose file → create Album 3/
    albumLookup[path.resolve(song6)] = "";          // empty   → skipped
    albumLookup[path.resolve(song7)] = "Album 2";  // loose file → move into Album 2/

    // Sanity check: all files created
    for (const f of [song1, song2, song3, song4, song5, song6, song7]) {
      expect(fs.existsSync(f)).toBe(true);
    }
  });

  afterAll(() => {
    if (tempDir && fs.existsSync(tempDir)) {
      fs.rmSync(tempDir, { recursive: true, force: true });
    }
  });

  it("initializes the service", () => {
    folderOrg = new FolderOrganizerService();
    folderOrg.setLibraryRoot(libRoot);
    expect(folderOrg).toBeInstanceOf(FolderOrganizerService);
  });

  it("plans the moves correctly", () => {
    const albumFn = makeAlbumFn(albumLookup);
    const allPaths = Object.keys(albumLookup);
    const plan = folderOrg.planGroupByAlbum({
      trackPaths: allPaths,
      libraryRoot: libRoot,
      albumTitleFn: albumFn,
    });

    expect(plan.kind).toBe("folder-move");
    expect(plan.reversible).toBe(true);

    // ── Moves (5) ────────────────────────────────────────────────
    // song1.mp3  Artist - Album 1/  → Album 1/  (album: "Album 1")
    // song2.mp3  Artist - Album 1/  → Album 1/  (album: "Album 1")
    // song4.mp3  root               → Album 1/  (album: "Album 1")
    // song5.mp3  root               → Album 3/  (album: "Album 3")
    // song7.mp3  root               → Album 2/  (album: "Album 2")
    expect(plan.moves).toHaveLength(5);

    const relMoves = plan.moves.map((m) => path.relative(libRoot, m.destinationPath));
    expect(relMoves).toContain(path.join("Album 1", "song1.mp3"));
    expect(relMoves).toContain(path.join("Album 1", "song2.mp3"));
    expect(relMoves).toContain(path.join("Album 1", "song4.mp3"));
    expect(relMoves).toContain(path.join("Album 3", "song5.mp3"));
    expect(relMoves).toContain(path.join("Album 2", "song7.mp3"));

    // ── Noops (1) ────────────────────────────────────────────────
    // song3.mp3 is already in Album 2/  (album: "Album 2" → sanitized "Album 2")
    expect(plan.noops).toHaveLength(1);
    expect(path.relative(libRoot, plan.noops[0].sourcePath)).toBe(path.join("Album 2", "song3.mp3"));
    expect(plan.noops[0].skipReason).toContain("Already in correct album folder");

    // ── Skipped (1) ──────────────────────────────────────────────
    // song6.mp3 has empty album tag
    expect(plan.skipped).toHaveLength(1);
    expect(plan.skipped[0].skipReason).toContain("Missing album title");
    expect(path.relative(libRoot, plan.skipped[0].sourcePath)).toBe("song6.mp3");

    // ── affectedTracks ───────────────────────────────────────────
    expect(plan.affectedTracks).toBe(5);
  });

  it("applies the moves", async () => {
    const albumFn = makeAlbumFn(albumLookup);
    const allPaths = Object.keys(albumLookup);
    const plan = folderOrg.planGroupByAlbum({
      trackPaths: allPaths,
      libraryRoot: libRoot,
      albumTitleFn: albumFn,
    });

    const { results, manifest } = await folderOrg.applyMoves(plan);

    // All 5 moves should succeed
    expect(results).toHaveLength(5);
    for (const r of results) {
      expect(r.success, `Move failed: ${r.sourcePath} → ${r.destinationPath}: ${r.error}`).toBe(true);
    }
    expect(manifest).toHaveLength(5);
  });

  it("verifies files are grouped correctly after moves", () => {
    // ── Album 1/ should contain song1, song2, song4 ──
    expect(dirContents(path.join(libRoot, "Album 1"))).toEqual(["song1.mp3", "song2.mp3", "song4.mp3"]);

    // ── Album 2/ should contain song3 (noop) + song7 (moved) ──
    expect(dirContents(path.join(libRoot, "Album 2"))).toEqual(["song3.mp3", "song7.mp3"]);

    // ── Album 3/ should contain song5 ──
    expect(dirContents(path.join(libRoot, "Album 3"))).toEqual(["song5.mp3"]);

    // ── song6 (skipped) should remain loose at root ──
    expect(fs.existsSync(path.join(libRoot, "song6.mp3"))).toBe(true);

    // ── Artist - Album 1/ should now be empty or gone ──
    // (songs were moved out, but the directory itself wasn't deleted)
    expect(dirContents(path.join(libRoot, "Artist - Album 1"))).toEqual([]);

    // ── Loose files should no longer exist at root ──
    expect(fs.existsSync(path.join(libRoot, "song4.mp3"))).toBe(false);
    expect(fs.existsSync(path.join(libRoot, "song5.mp3"))).toBe(false);
    expect(fs.existsSync(path.join(libRoot, "song7.mp3"))).toBe(false);
  });

  it("can revert the moves", async () => {
    // Construct reverse manifest from the original move state:
    // each entry: { from: currentLocation, to: originalLocation }
    const reverseManifest = [
      { from: path.join(libRoot, "song4.mp3"),         to: path.join(libRoot, "Album 1", "song4.mp3") },
      { from: path.join(libRoot, "song5.mp3"),         to: path.join(libRoot, "Album 3", "song5.mp3") },
      { from: path.join(libRoot, "song7.mp3"),         to: path.join(libRoot, "Album 2", "song7.mp3") },
      { from: path.join(libRoot, "Artist - Album 1", "song1.mp3"), to: path.join(libRoot, "Album 1", "song1.mp3") },
      { from: path.join(libRoot, "Artist - Album 1", "song2.mp3"), to: path.join(libRoot, "Album 1", "song2.mp3") },
    ];

    const revertResults = await folderOrg.revertMoves(reverseManifest);

    expect(revertResults).toHaveLength(5);
    for (const r of revertResults) {
      expect(r.success, `Revert failed: ${r.sourcePath} → ${r.destinationPath}: ${r.error}`).toBe(true);
    }

    // Verify files are back at original locations
    expect(fs.existsSync(song1)).toBe(true);
    expect(fs.existsSync(song2)).toBe(true);
    expect(fs.existsSync(song3)).toBe(true);
    expect(fs.existsSync(song4)).toBe(true);
    expect(fs.existsSync(song5)).toBe(true);
    expect(fs.existsSync(song6)).toBe(true);
    expect(fs.existsSync(song7)).toBe(true);

    // Verify album folders are empty (files moved out)
    expect(dirContents(path.join(libRoot, "Album 1"))).toEqual([]);
    expect(dirContents(path.join(libRoot, "Album 2"))).toEqual(["song3.mp3"]); // song3 was noop, stayed
    expect(dirContents(path.join(libRoot, "Album 3"))).toEqual([]);
  });
});
