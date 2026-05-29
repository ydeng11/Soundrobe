// @vitest-environment node
/**
 * E2E integration test for the auto-tag pipeline against a synthetic compilation album.
 *
 * Creates tiny synthetic FLACs under a "Compilations" folder, runs the actual
 * auto-tag pipeline (real MusicBrainz/Discogs API calls, plus LLM if configured),
 * and verifies that existing metadata is preserved, compilation detection works,
 * and &-separated collaboration artists are correctly split into ARTISTS.
 *
 * Requires at minimum a Discogs token (in ~/.auto-tagger/config.yaml or env).
 * LLM API key is optional — the pipeline skips LLM steps if absent.
 *
 * Run:   npx vitest run test/integration/auto-tag-compilation-e2e.test.ts
 */

import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";

// Increase timeout for E2E tests (real API calls with possible rate limiting)
vi.setConfig({ testTimeout: 180_000 });

import fs from "fs";
import path from "path";
import os from "os";
import { parseFile } from "music-metadata";
import { flacHeaderWithDuration, vorbisCommentBlock } from "../helpers/flac-helpers";

// Mock SQLite-backed modules BEFORE any handler imports.
// better-sqlite3 is compiled for Electron's ABI and won't load under system Node.
vi.mock("../../electron/handlers/cache", () => ({
  MatchCache: class {
    constructor() {}
    get() { return null; }
    set() {}
    close() {}
  },
}));

vi.mock("../../electron/handlers/dataset", () => ({
  DatasetReader: class {
    constructor() {}
    isAvailable() { return false; }
    hasLookupTable() { return false; }
    getPath() { return ""; }
    queryAlbum() { return []; }
    close() {}
    getStatus() { return { available: false, musicbrainz: false, totalRecords: 0, lastUpdated: null }; }
  },
}));

import type { AutoTagEvent } from "../../electron/handlers/auto-tag";
import { UndoManager } from "../../src/state/UndoManager";
import type { TrackData } from "../../electron/handlers/tracks";
import type { WriteFields } from "../../electron/handlers/writer";

// ── Synthetic FLAC builder ─────────────────────────────────────────

function syntheticFlac(tags: string[]): Buffer {
  const block = vorbisCommentBlock(tags, { isLast: true });
  return Buffer.concat([
    flacHeaderWithDuration(false, 200, [block]),
    Buffer.from([0xff, 0xf8, 0x69, 0x18]),
    Buffer.alloc(100),
  ]);
}

function writeSyntheticFlac(dir: string, filename: string, tags: string[]): string {
  const fp = path.join(dir, filename);
  fs.writeFileSync(fp, syntheticFlac(tags));
  return fp;
}

function sleep(ms: number): Promise<void> {
  return new Promise((r) => setTimeout(r, ms));
}

// ── Test ────────────────────────────────────────────────────────────

describe("auto-tag compilation E2E", () => {
  let tmpRoot: string;
  let albumDir: string;
  const savedEnv: Record<string, string | undefined> = {};

  beforeEach(() => {
    // Snapshot env vars
    savedEnv.AUTO_TAG_LLM_API_KEY = process.env.AUTO_TAG_LLM_API_KEY;
    savedEnv.AUTO_TAG_LLM_MODEL = process.env.AUTO_TAG_LLM_MODEL;
    savedEnv.AUTO_TAG_DISCOGS_TOKEN = process.env.AUTO_TAG_DISCOGS_TOKEN;
    savedEnv.AUTO_TAG_REMOTE_LOOKUP = process.env.AUTO_TAG_REMOTE_LOOKUP;
    savedEnv.HOME = process.env.HOME;

    // Temp dir with Compilations folder structure to trigger compilation detection
    tmpRoot = fs.mkdtempSync(path.join(os.tmpdir(), "auto-tag-e2e-"));
    albumDir = path.join(tmpRoot, "Compilations", "2013-友情岁月 3CD E2E");
    fs.mkdirSync(albumDir, { recursive: true });

    // Track 1 — single artist (郑伊健)
    writeSyntheticFlac(albumDir, "01. 友情岁月.flac", [
      "TITLE=友情岁月",
      "ARTIST=郑伊健",
      "ALBUM=友情岁月 3CD",
      "ALBUMARTIST=郑伊健&陈小春",
      "TRACKNUMBER=1",
      "DISCNUMBER=1",
      "DATE=2013",
    ]);

    // Track 2 — single artist (陈小春)
    writeSyntheticFlac(albumDir, "02. 战无不胜.flac", [
      "TITLE=战无不胜",
      "ARTIST=陈小春",
      "ALBUM=友情岁月 3CD",
      "ALBUMARTIST=郑伊健&陈小春",
      "TRACKNUMBER=2",
      "DISCNUMBER=1",
      "DATE=2013",
    ]);

    // Track 3 — collaboration with &-separated artist
    // ARTISTS is set to the unsplit &-separated value to test the pipeline split
    writeSyntheticFlac(albumDir, "03. 古古惑惑.flac", [
      "TITLE=古古惑惑",
      "ARTIST=谢天华&朱永棠&林晓峰",
      "ARTISTS=谢天华&朱永棠&林晓峰",
      "ALBUM=友情岁月 3CD",
      "ALBUMARTIST=郑伊健&陈小春",
      "TRACKNUMBER=3",
      "DISCNUMBER=1",
      "DATE=2013",
    ]);
  });

  afterEach(() => {
    // Restore env
    for (const [k, v] of Object.entries(savedEnv)) {
      if (v === undefined) delete process.env[k];
      else process.env[k] = v;
    }

    // Clean up
    if (tmpRoot && fs.existsSync(tmpRoot)) {
      fs.rmSync(tmpRoot, { recursive: true, force: true });
    }
  });

  it("preserves existing tags and splits collaboration ARTISTS", async () => {
    // ── Prerequisites ───────────────────────────────────────────────
    // The pipeline needs real API keys to run. Check minimum setup.
    // MusicBrainz: no key needed (free API).
    // Discogs: needs token (from env or config file).
    // LLM: optional — pipeline gracefully skips LLM steps when absent.
    const hasDiscogsToken =
      !!(process.env.AUTO_TAG_DISCOGS_TOKEN ?? "") ||
      fs.existsSync(path.join(os.homedir(), ".auto-tagger", "config.yaml"));

    if (!hasDiscogsToken) {
      console.warn("[e2e] No Discogs token found — API lookups will fail, folder fallback will be used");
    }

    // ── 1. Verify initial state ─────────────────────────────────────
    const t1Before = await parseFile(path.join(albumDir, "01. 友情岁月.flac"));
    expect(t1Before.common.title).toBe("友情岁月");
    expect(t1Before.common.artist).toBe("郑伊健");

    const t3Before = await parseFile(path.join(albumDir, "03. 古古惑惑.flac"));
    expect(t3Before.common.title).toBe("古古惑惑");
    expect(t3Before.common.artist).toBe("谢天华&朱永棠&林晓峰");
    expect(t3Before.common.artists).toEqual(["谢天华&朱永棠&林晓峰"]);

    // ── 2. Ensure config picks up real env ──────────────────────────
    // Use the real HOME so config file keys are discovered
    process.env.HOME = os.homedir();

    const { refreshConfig, setDebugMode } = await import("../../electron/handlers/auto-tag");
    refreshConfig();
    setDebugMode(true);

    // ── 3. Run the pipeline ─────────────────────────────────────────
    const { startAutoTag, onAutoTagEvent, getProgress } =
      await import("../../electron/handlers/auto-tag");

    // Collect events for diagnostics
    const events: AutoTagEvent[] = [];
    const unsub = onAutoTagEvent((evt) => {
      events.push(evt);
    });

    const taskId = startAutoTag(albumDir);
    expect(taskId).toBeTruthy();
    console.log(`\n[e2e] Task ${taskId} started — waiting for pipeline...`);

    // ── 4. Poll until completion (max 2 min) ────────────────────────
    const deadline = Date.now() + 120_000;
    let completed = false;
    let failed = false;
    let lastMsg = "";

    while (Date.now() < deadline) {
      await sleep(1000);
      const p = getProgress(taskId);
      if (!p) break;
      if (p.message !== lastMsg) {
        console.log(`[e2e]  ${p.progress}/${p.total}  ${p.message}`);
        lastMsg = p.message;
      }
      if (p.status === "completed") { completed = true; break; }
      if (p.status === "failed") { failed = true; break; }
    }

    unsub();

    // ── Diagnostics ─────────────────────────────────────────────────
    if (!completed) {
      console.log("[e2e] Pipeline did not complete. Events:");
      for (const e of events) {
        console.log(`  ${e.type}: ${e.message}`);
      }
    }

    expect(completed).toBe(true);
    if (failed) throw new Error("Pipeline failed — check diagnostics above");

    // ── 5. Read files after pipeline ────────────────────────────────
    const t1 = await parseFile(path.join(albumDir, "01. 友情岁月.flac"));
    const t2 = await parseFile(path.join(albumDir, "02. 战无不胜.flac"));
    const t3 = await parseFile(path.join(albumDir, "03. 古古惑惑.flac"));

    // ── 6. Assertions ───────────────────────────────────────────────

    // 6a. Title unchanged — pipeline must never corrupt existing titles
    expect(t1.common.title).toBe("友情岁月");
    expect(t2.common.title).toBe("战无不胜");
    expect(t3.common.title).toBe("古古惑惑");

    // 6b. Artist unchanged
    expect(t1.common.artist).toBe("郑伊健");
    expect(t2.common.artist).toBe("陈小春");
    expect(t3.common.artist).toBe("谢天华&朱永棠&林晓峰");

    // 6c. ARTISTS split for collaboration track
    expect(t3.common.artists).toEqual(["谢天华", "朱永棠", "林晓峰"]);

    // 6d. Album artist is "Various Artists" (compilation folder detection)
    expect(t1.common.albumartist).toBe("Various Artists");
    expect(t2.common.albumartist).toBe("Various Artists");
    expect(t3.common.albumartist).toBe("Various Artists");

    // 6e. Album is set (either preserved from original or resolved)
    expect(t1.common.album).toBeTruthy();
    expect(t2.common.album).toBeTruthy();
    expect(t3.common.album).toBeTruthy();

    // 6f. Year preserved
    expect(t1.common.year).toBe(2013);
    expect(t2.common.year).toBe(2013);
    expect(t3.common.year).toBe(2013);

    // 6g. Track numbers preserved
    expect(t1.common.track?.no).toBe(1);
    expect(t2.common.track?.no).toBe(2);
    expect(t3.common.track?.no).toBe(3);

    // 6h. Single-artist tracks have matching ARTISTS (single element)
    expect(t1.common.artists).toEqual(["郑伊健"]);
    expect(t2.common.artists).toEqual(["陈小春"]);

    console.log("\n[e2e] ✓ All assertions passed!");
  });

  it("supports undo round-trip: snapshot → auto-tag → revert → verify", async () => {
    // ── Prerequisites ───────────────────────────────────────────────
    const hasDiscogsToken =
      !!(process.env.AUTO_TAG_DISCOGS_TOKEN ?? "") ||
      fs.existsSync(path.join(os.homedir(), ".auto-tagger", "config.yaml"));

    if (!hasDiscogsToken) {
      console.warn("[e2e-undo] No Discogs token — API lookups will fail, folder fallback will be used");
    }

    const { refreshConfig, setDebugMode } = await import("../../electron/handlers/auto-tag");
    refreshConfig();
    setDebugMode(true);

    const { readTrackMetadata } = await import("../../electron/handlers/tracks");
    const { writeTags } = await import("../../electron/handlers/writer");

    // ── 1. Read initial metadata into snapshots ────────────────────
    // This mirrors the exact pattern from App.tsx handleAutoTag
    const trackFiles = [
      path.join(albumDir, "01. 友情岁月.flac"),
      path.join(albumDir, "02. 战无不胜.flac"),
      path.join(albumDir, "03. 古古惑惑.flac"),
    ];

    const initialTracks: TrackData[] = [];
    for (const fp of trackFiles) {
      initialTracks.push(await readTrackMetadata(fp));
    }

    expect(initialTracks).toHaveLength(3);

    // Build snapshots using the exact field mapping from handleAutoTag
    const snapshots = initialTracks.map((t) => ({
      path: t.path,
      fields: {
        title: t.title,
        artist: t.artist,
        artists: t.artists,
        album: t.album,
        albumArtist: t.albumArtist,
        albumArtists: t.albumArtists,
        year: t.year,
        trackNumber: t.trackNumber,
        trackTotal: t.trackTotal,
        discNumber: t.discNumber,
        discTotal: t.discTotal,
        genre: t.genre,
        composer: t.composer,
        comment: t.comment ?? null,
        musicbrainzTrackId: t.musicbrainzTrackId,
        musicbrainzAlbumId: t.musicbrainzAlbumId,
        musicbrainzArtistId: t.musicbrainzArtistId,
      },
    }));

    // ── 2. Push snapshots to UndoManager ───────────────────────────
    const undoManager = new UndoManager();
    undoManager.push("Auto-tag", snapshots);
    expect(undoManager.canUndo).toBe(true);
    expect(undoManager.length).toBe(1);

    // ── 3. Verify initial state before pipeline ────────────────────
    expect(initialTracks[0].title).toBe("友情岁月");
    expect(initialTracks[0].artist).toBe("郑伊健");
    expect(initialTracks[0].trackNumber).toBe(1);
    expect(initialTracks[0].discNumber).toBe(1);
    expect(initialTracks[0].year).toBe("2013");

    expect(initialTracks[2].artist).toBe("谢天华&朱永棠&林晓峰");
    expect(initialTracks[2].artists).toEqual(["谢天华&朱永棠&林晓峰"]);

    // ── 4. Run the auto-tag pipeline ───────────────────────────────
    process.env.HOME = os.homedir();
    const { startAutoTag, onAutoTagEvent, getProgress } =
      await import("../../electron/handlers/auto-tag");

    const taskId = startAutoTag(albumDir);
    expect(taskId).toBeTruthy();
    console.log(`\n[e2e-undo] Task ${taskId} started — waiting for pipeline...`);

    const deadline = Date.now() + 120_000;
    let completed = false;
    let failed = false;
    let lastMsg = "";

    while (Date.now() < deadline) {
      await sleep(1000);
      const p = getProgress(taskId);
      if (!p) break;
      if (p.message !== lastMsg) {
        console.log(`[e2e-undo]  ${p.progress}/${p.total}  ${p.message}`);
        lastMsg = p.message;
      }
      if (p.status === "completed") { completed = true; break; }
      if (p.status === "failed") { failed = true; break; }
    }

    expect(completed).toBe(true);
    if (failed) throw new Error("Pipeline failed");

    // ── 5. Verify pipeline actually changed something ──────────────
    const afterPipeline = await Promise.all(
      trackFiles.map((fp) => readTrackMetadata(fp)),
    );

    // Album artist should now be "Various Artists" (compilation detection)
    expect(afterPipeline[0].albumArtist).toBe("Various Artists");
    expect(afterPipeline[1].albumArtist).toBe("Various Artists");
    expect(afterPipeline[2].albumArtist).toBe("Various Artists");

    // Collaboration track's ARTISTS should be split
    expect(afterPipeline[2].artists).toEqual(["谢天华", "朱永棠", "林晓峰"]);

    // ── 6. Pop undo and restore snapshots ──────────────────────────
    expect(undoManager.canUndo).toBe(true);
    const op = undoManager.pop();
    expect(op).not.toBeNull();
    expect(op!.snapshots).toHaveLength(3);

    // Write back snapshot fields — same as handleRevert does
    for (const snap of op!.snapshots) {
      await writeTags(snap.path, snap.fields as WriteFields);
    }

    // ── 7. Re-read and verify everything restored to original ──────
    const afterUndo = await Promise.all(
      trackFiles.map((fp) => readTrackMetadata(fp)),
    );

    // Title restored
    expect(afterUndo[0].title).toBe("友情岁月");
    expect(afterUndo[1].title).toBe("战无不胜");
    expect(afterUndo[2].title).toBe("古古惑惑");

    // Single artist restored
    expect(afterUndo[0].artist).toBe("郑伊健");
    expect(afterUndo[1].artist).toBe("陈小春");

    // Collaboration artist restored to original unsplit form
    expect(afterUndo[2].artist).toBe("谢天华&朱永棠&林晓峰");
    expect(afterUndo[2].artists).toEqual(["谢天华&朱永棠&林晓峰"]);

    // Album artist restored to original
    expect(afterUndo[0].albumArtist).toBe("郑伊健&陈小春");
    expect(afterUndo[1].albumArtist).toBe("郑伊健&陈小春");
    expect(afterUndo[2].albumArtist).toBe("郑伊健&陈小春");

    // Track numbers restored
    expect(afterUndo[0].trackNumber).toBe(1);
    expect(afterUndo[1].trackNumber).toBe(2);
    expect(afterUndo[2].trackNumber).toBe(3);

    // Disc numbers restored
    expect(afterUndo[0].discNumber).toBe(1);
    expect(afterUndo[1].discNumber).toBe(1);
    expect(afterUndo[2].discNumber).toBe(1);

    // Year restored
    expect(afterUndo[0].year).toBe("2013");
    expect(afterUndo[1].year).toBe("2013");
    expect(afterUndo[2].year).toBe("2013");

    // Album restored
    expect(afterUndo[0].album).toBe("友情岁月 3CD");
    expect(afterUndo[1].album).toBe("友情岁月 3CD");
    expect(afterUndo[2].album).toBe("友情岁月 3CD");

    // Undo stack is now empty after pop
    expect(undoManager.canUndo).toBe(false);
    expect(undoManager.length).toBe(0);

    console.log("\n[e2e-undo] ✓ Undo round-trip verified!");
  });
});
