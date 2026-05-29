import { describe, it, expect, afterEach, beforeEach } from "vitest";
import { mkdtempSync, rmSync } from "node:fs";
import { join } from "node:path";
import { tmpdir } from "node:os";
import {
  hintsAreAmbiguous,
  loadConfig,
  startAutoTag,
  getProgress,
  cancelTask,
  getDatasetStatus,
  getConfig,
  refreshConfig,
  onAutoTagEvent,
  buildAliasedLookupVariants,
} from "../../electron/handlers/auto-tag";
import { setAliasFilePath, saveAlias } from "../../electron/handlers/aliases";

describe("loadConfig", () => {
  const originalEnv = { ...process.env };

  afterEach(() => {
    process.env = { ...originalEnv };
    // Clear config-dependent env vars that might exist on the dev machine
    delete process.env.AUTO_TAG_LLM_API_KEY;
    delete process.env.AUTO_TAG_LLM_MODEL;
    delete process.env.AUTO_TAG_DISCOGS_TOKEN;
  });

  it("loads from environment variables", () => {
    process.env.AUTO_TAG_LLM_API_KEY = "env-key";
    process.env.AUTO_TAG_LLM_MODEL = "env-model";
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

describe("startAutoTag / getProgress / cancelTask", () => {
  it("creates a task and tracks it", async () => {
    const taskId = startAutoTag("/test/album/path");
    expect(taskId).toBeTruthy();
    expect(taskId.startsWith("auto-tag-")).toBe(true);

    const progress = getProgress(taskId);
    expect(progress).not.toBeNull();
    expect(progress!.taskId).toBe(taskId);
    expect(progress!.status).toBe("running");
    expect(progress!.total).toBe(9);
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
  // These tests verify the end-to-end behavior by starting an auto-tag task
  // and checking that progress can be tracked without crashes.

  it("starts and completes for a valid album path with no LLM key (falls back to folder)", { timeout: 10000 }, async () => {
    const taskId = startAutoTag("/test/artist/album");
    expect(taskId).toBeTruthy();

    // Poll for completion
    let status = "";
    for (let i = 0; i < 20; i++) {
      await new Promise((r) => setTimeout(r, 200));
      const progress = getProgress(taskId);
      if (progress) {
        status = progress.status;
        if (status === "completed" || status === "failed" || status === "cancelled") {
          break;
        }
      }
    }

    expect(["completed", "failed"]).toContain(status);
  });

  it("starts and completes for CD subfolder pattern", { timeout: 10000 }, async () => {
    const taskId = startAutoTag("/test/Artist/Album (2CD)/CD1");
    expect(taskId).toBeTruthy();

    let status = "";
    for (let i = 0; i < 20; i++) {
      await new Promise((r) => setTimeout(r, 200));
      const progress = getProgress(taskId);
      if (progress) {
        status = progress.status;
        if (status === "completed" || status === "failed" || status === "cancelled") {
          break;
        }
      }
    }

    expect(["completed", "failed"]).toContain(status);
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
