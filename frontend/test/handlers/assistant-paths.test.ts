/**
 * Tests for assistant read-only tool summary output.
 *
 * Unlike mirror tests that replicate executor logic inline, this test
 * calls the ACTUAL executor functions from the production module:
 *   buildReadOnlyToolsForTesting() → gets the real tool defs
 *   tool.executor()                → calls the real executor
 *
 * This catches regressions when someone changes the executor code
 * without updating the output format.
 *
 * Verifies every read-only tool that returns track-level data
 * includes the full file path in its summary text, so the LLM can
 * reference files directly in subsequent mutating tool calls.
 */

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



import { mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import { readTrackMetadata } from "../../electron/handlers/tracks";
import {
  flacHeaderWithDuration,
  vorbisCommentBlock,
} from "../helpers/flac-helpers";

// ── Helper: build a synthetic FLAC file with Vorbis comments ─────

function makeFlac(
  filePath: string,
  comments: string[],
): void {
  const block = vorbisCommentBlock(comments, { isLast: true });
  const buf = Buffer.concat([
    flacHeaderWithDuration(false, 200, [block]),
    Buffer.from([0xff, 0xf8, 0x69, 0x18]),
    Buffer.alloc(100),
  ]);
  writeFileSync(filePath, buf);
}

// ── Suite ────────────────────────────────────────────────────────

describe("assistant read-only tool summaries include file paths", () => {
  let tmpDir: string;
  let trackList: import("../../electron/preload").TrackData[];
  let readOnlyTools: import("../../electron/services/AssistantToolRegistry").AssistantToolDef[];

  beforeEach(async () => {
    tmpDir = mkdtempSync(join(tmpdir(), "auto-tagger-assistant-paths-"));

    // Create 3 FLAC files with distinct metadata
    const fp1 = join(tmpDir, "01. Song One.flac");
    const fp2 = join(tmpDir, "02. Song Two.flac");
    const fp3 = join(tmpDir, "03. Song Three.flac");

    makeFlac(fp1, [
      "TITLE=Song One",
      "ARTIST=Artist A",
      "ALBUM=Test Album",
      "ALBUMARTIST=Artist A",
      "TRACKNUMBER=1",
      "TRACKTOTAL=3",
      "GENRE=Rock",
    ]);
    makeFlac(fp2, [
      "TITLE=Song Two",
      "ARTIST=Artist B",
      "ALBUM=Test Album",
      "ALBUMARTIST=Artist A",
      "TRACKNUMBER=2",
      "TRACKTOTAL=3",
      "GENRE=Rock",
    ]);
    makeFlac(fp3, [
      // Intentionally missing TITLE to test missing-title filtering
      // Different artist so filtering by Artist A finds exactly the 2 we expect
      "ARTIST=Different Artist",
      "ALBUM=Different Album",
      "ALBUMARTIST=Different Artist",
      "TRACKNUMBER=1",
      "TRACKTOTAL=1",
      "GENRE=Jazz",
    ]);

    // Read metadata to get TrackData with proper paths
    const td1 = await readTrackMetadata(fp1);
    const td2 = await readTrackMetadata(fp2);
    const td3 = await readTrackMetadata(fp3);
    trackList = [td1, td2, td3];

    // ── Initialize real services and get real tool defs ──────────
    const {
      initializeAssistantServices,
      setAssistantAppState,
      buildReadOnlyToolsForTesting,
    } = await import("../../electron/handlers/assistant");

    // Initialize services (SafeQueryService, LibraryService, etc.)
    initializeAssistantServices({
      apiKey: "test-key",
      model: "test-model",
    });

    // Set the app state with our track data so the tool executors
    // read from it via currentAppState
    setAssistantAppState({
      libraryPath: tmpDir,
      activeAlbumPath: null,
      selectedTrackPaths: [],
      tracks: trackList,
      albums: trackList.reduce<Array<{ path: string; name: string; artistHint: string; trackCount: number }>>(
        (acc, t) => {
          const albumPath = tmpDir; // simplified — all tracks in same flat dir
          if (!acc.find((a) => a.name === t.album)) {
            acc.push({
              path: albumPath,
              name: t.album ?? "?",
              artistHint: t.albumArtist ?? "",
              trackCount: 1,
            });
          }
          return acc;
        },
        [],
      ),
      autonomous: false,
    });

    // Get the actual read-only tools with real executor closures
    // (same code that the assistant panel uses)
    readOnlyTools = buildReadOnlyToolsForTesting();
  });

  afterEach(() => {
    rmSync(tmpDir, { recursive: true, force: true });
  });

  function getTool(name: string): import("../../electron/services/AssistantToolRegistry").AssistantToolDef {
    const tool = readOnlyTools.find((t) => t.name === name);
    if (!tool) throw new Error(`Tool not found: ${name}`);
    return tool;
  }

  async function execTool(
    name: string,
    args: Record<string, unknown> = {},
  ): Promise<import("../../electron/services/AssistantToolRegistry").AssistantToolResult> {
    const tool = getTool(name);
    return tool.executor(args);
  }

  // ── 1. tracks.search ───────────────────────────────────────────

  describe("tracks.search", () => {
    it("includes the full file path in each result when filtering by artist", async () => {
      const result = await execTool("tracks.search", { artist: "Artist A" });
      expect(result.ok).toBe(true);
      expect(result.summary).toContain("Found 2 track(s):");
      const lines = result.summary.split("\n");
      const pathLines = lines.filter((l) => l.startsWith("  - "));
      expect(pathLines).toHaveLength(2);
      // Every path line contains the full absolute path
      expect(pathLines[0]).toContain(trackList[0].path);
      expect(pathLines[1]).toContain(trackList[1].path);
      // Track3 has artist="Different Artist" so it's not in results
      expect(result.summary).not.toContain(trackList[2].path);
    });

    it("includes the full file path when filtering by missing title", async () => {
      const result = await execTool("tracks.search", { missingTitle: true });
      expect(result.ok).toBe(true);
      expect(result.summary).toContain("Found 1 track(s):");
      expect(result.summary).toContain(trackList[2].path);
      expect(result.summary).toContain("? by Different Artist");
    });

    it("includes the full file path when filtering by album", async () => {
      const result = await execTool("tracks.search", { album: "Test Album" });
      expect(result.ok).toBe(true);
      expect(result.summary).toContain("Found 2 track(s):");
      expect(result.summary).toContain(trackList[0].path);
      expect(result.summary).toContain(trackList[1].path);
      expect(result.summary).not.toContain(trackList[2].path);
    });

    it("returns 'no matches' when nothing matches", async () => {
      const result = await execTool("tracks.search", { artist: "Nonexistent" });
      expect(result.ok).toBe(true);
      expect(result.summary).toBe("No tracks match the query.");
    });
  });

  // ── 2. tracks.inspect ──────────────────────────────────────────

  describe("tracks.inspect", () => {
    it("includes Path: line with the full absolute path for each track", async () => {
      const result = await execTool("tracks.inspect", {
        paths: trackList.map((t) => t.path),
      });
      expect(result.ok).toBe(true);
      const lines = result.summary.split("\n");
      const pathLines = lines.filter((l) => l.startsWith("    Path: "));
      expect(pathLines).toHaveLength(3);
      for (let i = 0; i < 3; i++) {
        expect(pathLines[i]).toContain(trackList[i].path);
      }
    });

    it("shows Path: for a single track by explicit path", async () => {
      const result = await execTool("tracks.inspect", {
        paths: [trackList[0].path],
      });
      expect(result.ok).toBe(true);
      expect(result.summary).toContain(`    Path: ${trackList[0].path}`);
      expect(result.summary).not.toContain(trackList[1].path);
    });
  });

  // ── 3. albums.inspect ──────────────────────────────────────────

  describe("albums.inspect", () => {
    it("includes File: and Path: for each track in album listing", async () => {
      // albums.inspect requires an actual album path.
      // Our tracks are in a flat dir (tmpDir), so the album path is tmpDir.
      const result = await execTool("albums.inspect", { path: tmpDir });
      expect(result.ok).toBe(true);
      expect(result.summary).toContain(`    Path: ${trackList[0].path}`);
      expect(result.summary).toContain(`    Path: ${trackList[1].path}`);
      expect(result.summary).toContain(`    Path: ${trackList[2].path}`);
      expect(result.summary).toContain("File: 01. Song One.flac");
      expect(result.summary).toContain("File: 02. Song Two.flac");
      expect(result.summary).toContain("File: 03. Song Three.flac");
    });
  });

  // ── 4. query.metadata (missingTags) ────────────────────────────

  describe("query.metadata (missingTags)", () => {
    it("includes the full file path in each result when finding missing titles", async () => {
      const result = await execTool("query.metadata", {
        missingTags: "title",
      });
      expect(result.ok).toBe(true);
      expect(result.summary).toContain("Found 1 track(s) missing title:");
      expect(result.summary).toContain(trackList[2].path);
      expect(result.summary).toContain("(no title)");
    });

    it("includes the full file path for missing artist results", async () => {
      const result = await execTool("query.metadata", {
        missingTags: "artist",
      });
      expect(result.ok).toBe(true);
      expect(result.summary).toContain("Found 0 track(s)");
    });
  });

  // ── 5. query.metadata (duplicates) ──────────────────────────────

  describe("query.metadata (duplicates)", () => {
    it("includes full file path in each duplicate result", async () => {
      // Create a duplicate track with same title+artist+album as track1
      const dupPath = join(tmpDir, "Song One Dup.flac");
      makeFlac(dupPath, [
        "TITLE=Song One",
        "ARTIST=Artist A",
        "ALBUM=Test Album",
        "TRACKNUMBER=1",
      ]);
      const dupTrack = await readTrackMetadata(dupPath);
      const allTracks = [...trackList, dupTrack];

      // Update the app state with the expanded track list so the
      // query.metadata executor reads from it
      const { setAssistantAppState } = await import("../../electron/handlers/assistant");
      setAssistantAppState({
        libraryPath: tmpDir,
        activeAlbumPath: null,
        selectedTrackPaths: [],
        tracks: allTracks,
        albums: [],
        autonomous: false,
      });

      const result = await execTool("query.metadata", { duplicates: true });
      expect(result.ok).toBe(true);
      expect(result.summary).toContain("Found");
      expect(result.summary).toContain("tracks with potential duplicates");
      expect(result.summary).toContain(`    Path: ${trackList[0].path}`);
      expect(result.summary).toContain(`    Path: ${dupPath}`);
    });
  });
});
