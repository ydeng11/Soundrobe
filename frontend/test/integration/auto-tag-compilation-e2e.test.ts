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
});
