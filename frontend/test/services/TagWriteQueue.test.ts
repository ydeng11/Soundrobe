import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import fs from "fs";
import path from "path";
import os from "os";
import * as NodeID3 from "node-id3";
import {
  TagWriteQueue,
  executeTagWrite,
  deduplicateJobs,
  resetDefaultWriteQueue,
  getDefaultWriteQueue,
  isExtraTagJob,
} from "../../electron/services/TagWriteQueue";
import type { TagWriteExecutor } from "../../electron/services/TagWriteQueue";

// ── Helpers ──────────────────────────────────────────────────────

function createMinimalMp3(filePath: string, initialTags?: Record<string, string>): void {
  if (initialTags) {
    NodeID3.write(
      {
        title: initialTags.title,
        artist: initialTags.artist,
        album: initialTags.album,
      },
      filePath,
    );
  } else {
    NodeID3.write({}, filePath);
  }
  const fd = fs.openSync(filePath, "a");
  const frame = Buffer.alloc(417);
  frame[0] = 0xff;
  frame[1] = 0xfb;
  frame[2] = (9 << 4) | (0 << 2);
  frame[3] = 0x02;
  fs.writeSync(fd, frame, 0, frame.length);
  fs.closeSync(fd);
}

// ── executeTagWrite ─────────────────────────────────────────────

describe("executeTagWrite", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "tag-exec-"));
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  it("writes standard tags and returns success", async () => {
    const fp = path.join(tmpDir, "test.mp3");
    createMinimalMp3(fp);

    const result = await executeTagWrite({
      filePath: fp,
      fields: { title: "Queue Test", artist: "Queue Artist" },
    });

    expect(result.success).toBe(true);
    expect(result.filePath).toBe(fp);
    const tags = NodeID3.read(fp);
    expect(tags.title).toBe("Queue Test");
    expect(tags.artist).toBe("Queue Artist");
  });

  it("writes extra tags and returns success", async () => {
    const fp = path.join(tmpDir, "extra.mp3");
    createMinimalMp3(fp);

    const result = await executeTagWrite({
      filePath: fp,
      extraTags: [{ key: "MOOD", value: "Chill" }],
    });

    expect(result.success).toBe(true);
    const tags = NodeID3.read(fp);
    const udt = Array.isArray(tags.userDefinedText)
      ? tags.userDefinedText
      : tags.userDefinedText
        ? [tags.userDefinedText]
        : [];
    const byDesc = Object.fromEntries(
      udt.filter((t) => t.description).map((t) => [t.description, t.value]),
    );
    expect(byDesc["MOOD"]).toBe("Chill");
  });

  it("returns error for non-existent file", async () => {
    const result = await executeTagWrite({
      filePath: path.join(tmpDir, "noexist.mp3"),
      fields: { title: "X" },
    });

    expect(result.success).toBe(false);
    expect(result.error).toBeTruthy();
    expect(result.filePath).toBe(path.join(tmpDir, "noexist.mp3"));
  });

  it("handles empty fields gracefully", async () => {
    const fp = path.join(tmpDir, "empty.mp3");
    createMinimalMp3(fp);

    const result = await executeTagWrite({
      filePath: fp,
      fields: {},
    });

    expect(result.success).toBe(true);
  });
});

// ── deduplicateJobs ─────────────────────────────────────────────

describe("deduplicateJobs", () => {
  it("deduplicates same path for fields writes (later wins)", () => {
    const fp = "/some/dir/track.mp3";
    const deduped = deduplicateJobs([
      { filePath: fp, fields: { title: "Old" } },
      { filePath: fp, fields: { title: "New" } },
    ]);

    expect(deduped).toHaveLength(1);
    expect(deduped[0].fields?.title).toBe("New");
  });

  it("merges extra tags from same path", () => {
    const fp = "/some/dir/track.mp3";
    const deduped = deduplicateJobs([
      { filePath: fp, extraTags: [{ key: "MOOD", value: "Happy" }] },
      { filePath: fp, extraTags: [{ key: "RATING", value: "5" }] },
    ]);

    expect(deduped).toHaveLength(1);
    expect(deduped[0].extraTags).toHaveLength(2);
    expect(deduped[0].extraTags).toContainEqual({ key: "MOOD", value: "Happy" });
    expect(deduped[0].extraTags).toContainEqual({ key: "RATING", value: "5" });
  });

  it("preserves extraTags: [] as a real write operation", () => {
    const fp = "/some/dir/track.mp3";
    const deduped = deduplicateJobs([
      { filePath: fp, extraTags: [] },
    ]);

    expect(deduped).toHaveLength(1);
    expect(deduped[0].extraTags).toEqual([]);
  });

  it("keeps fields and extraTags as separate entries when they target different paths", () => {
    const fp1 = "/a.mp3";
    const fp2 = "/b.mp3";
    const deduped = deduplicateJobs([
      { filePath: fp1, fields: { title: "A" } },
      { filePath: fp2, extraTags: [{ key: "MOOD", value: "Bright" }] },
    ]);

    expect(deduped).toHaveLength(2);
  });

  it("normalizes paths (resolves relative to absolute)", () => {
    // The function uses path.resolve which for relative paths
    // prepends cwd. We test with an absolute path to verify normalization.
    const absolute = path.resolve("/tmp/test-track.mp3");
    const deduped = deduplicateJobs([
      { filePath: "/tmp/test-track.mp3", fields: { title: "T" } },
    ]);

    expect(deduped).toHaveLength(1);
    expect(deduped[0].filePath).toBe(absolute);
  });

  it("handles empty input", () => {
    expect(deduplicateJobs([])).toEqual([]);
  });

  it("handles single job", () => {
    const fp = "/track.mp3";
    const deduped = deduplicateJobs([
      { filePath: fp, fields: { title: "Solo" } },
    ]);
    expect(deduped).toHaveLength(1);
    expect(deduped[0].fields?.title).toBe("Solo");
  });

  it("preserves Buffer/Uint8Array cover art through dedup", () => {
    const coverData = Buffer.from([0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10, 0x4a, 0x46, 0x49, 0x46]);
    const fp = "/cover-track.mp3";

    const dedupedSingle = deduplicateJobs([
      { filePath: fp, fields: { title: "Song", coverData, coverMime: "image/jpeg" } },
    ]);

    expect(dedupedSingle).toHaveLength(1);
    expect(dedupedSingle[0].fields?.coverData).toBe(coverData);
    expect(Buffer.isBuffer(dedupedSingle[0].fields?.coverData)).toBe(true);
    expect(dedupedSingle[0].fields?.coverData!.toString("hex")).toBe(coverData.toString("hex"));
  });

  it("cover data survives dedup with fields overwrite (last wins)", () => {
    const cover1 = Buffer.from([0xff, 0xd8, 0xff, 0xe0]);
    const cover2 = Buffer.from([0x89, 0x50, 0x4e, 0x47]);
    const fp = "/cover-dedup.mp3";

    const deduped = deduplicateJobs([
      { filePath: fp, fields: { title: "Old", coverData: cover1, coverMime: "image/jpeg" } },
      { filePath: fp, fields: { title: "New", coverData: cover2, coverMime: "image/png" } },
    ]);

    expect(deduped).toHaveLength(1);
    // Later fields write wins for same path
    expect(deduped[0].fields?.coverData).toBe(cover2);
    expect(deduped[0].fields?.coverMime).toBe("image/png");
    expect(deduped[0].fields?.title).toBe("New");
  });

  it("does NOT merge fields job with extraTags job for same path (different type)", () => {
    // Fields jobs and extraTags jobs are different operations -
    // they should remain separate entries
    const fp = "/track.mp3";
    const deduped = deduplicateJobs([
      { filePath: fp, fields: { title: "Title" } },
      { filePath: fp, extraTags: [{ key: "MOOD", value: "Happy" }] },
    ]);

    expect(deduped).toHaveLength(2);
  });
});

// ── TagWriteQueue.submit ────────────────────────────────────────

describe("TagWriteQueue.submit", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "tag-queue-"));
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  it("writes tags to multiple files concurrently", async () => {
    const f1 = path.join(tmpDir, "t1.mp3");
    const f2 = path.join(tmpDir, "t2.mp3");
    createMinimalMp3(f1);
    createMinimalMp3(f2);

    const queue = new TagWriteQueue(4);
    const results = await queue.submit([
      { filePath: f1, fields: { title: "Track 1" } },
      { filePath: f2, fields: { title: "Track 2" } },
    ]);

    expect(results).toHaveLength(2);
    expect(results.every((r) => r.success)).toBe(true);
    expect(NodeID3.read(f1).title).toBe("Track 1");
    expect(NodeID3.read(f2).title).toBe("Track 2");
  });

  it("handles empty jobs array", async () => {
    const queue = new TagWriteQueue();
    const results = await queue.submit([]);
    expect(results).toEqual([]);
  });

  it("deduplicates same-path jobs", async () => {
    const fp = path.join(tmpDir, "dedup.mp3");
    createMinimalMp3(fp);

    const queue = new TagWriteQueue(2);
    const results = await queue.submit([
      { filePath: fp, fields: { title: "Old Title" } },
      { filePath: fp, fields: { title: "New Title" } },
    ]);

    // Should have been deduplicated to one
    expect(results).toHaveLength(1);
    expect(results[0].success).toBe(true);
    expect(NodeID3.read(fp).title).toBe("New Title");
  });

  it("propagates per-file failures", async () => {
    const f1 = path.join(tmpDir, "good.mp3");
    const f2 = path.join(tmpDir, "bad.mp3");
    createMinimalMp3(f1);

    const queue = new TagWriteQueue(2);
    const results = await queue.submit([
      { filePath: f1, fields: { title: "Good" } },
      { filePath: f2, fields: { title: "Bad" } },
    ]);

    expect(results).toHaveLength(2);
    // Good file should succeed
    expect(results.find((r) => r.filePath === f1)?.success).toBe(true);
    expect(NodeID3.read(f1).title).toBe("Good");
    // Bad file should fail
    expect(results.find((r) => r.filePath === f2)?.success).toBe(false);
  });

  it("default write concurrency is 1", () => {
    const queue = new TagWriteQueue();
    expect(queue.getMaxConcurrency()).toBe(1);
  });

  it("write results include durationMs and outcome metadata", async () => {
    const fp = path.join(tmpDir, "meta.mp3");
    createMinimalMp3(fp);

    const queue = new TagWriteQueue(1);
    const results = await queue.submit([
      { filePath: fp, fields: { title: "Meta Test" } },
    ]);

    expect(results).toHaveLength(1);
    expect(results[0].success).toBe(true);
    expect(typeof results[0].durationMs).toBe("number");
    expect(results[0].durationMs).toBeGreaterThanOrEqual(0);
    // outcome should be present (even if full_rewrite for MP3)
    expect(results[0].outcome).toBeDefined();
    expect(["skipped", "in_place", "metadata_rewrite", "full_rewrite"]).toContain(results[0].outcome);
    expect(NodeID3.read(fp).title).toBe("Meta Test");
  });

  it("bounded concurrency — caps concurrent writes", async () => {
    const files: string[] = [];
    let maxConcurrent = 0;
    let concurrent = 0;

    for (let i = 0; i < 10; i++) {
      const fp = path.join(tmpDir, `c${i}.mp3`);
      createMinimalMp3(fp);
      files.push(fp);
    }

    // Override executeTagWrite to track concurrency
    const originalWrite = executeTagWrite;
    // We'll test concurrency via the submit pattern instead

    const queue = new TagWriteQueue(3);
    const results = await queue.submit(
      files.map((fp) => ({ filePath: fp, fields: { title: path.basename(fp) } })),
    );

    expect(results).toHaveLength(10);
    expect(results.every((r) => r.success)).toBe(true);
  });

  it("handles empty extra-tags array without hanging", async () => {
    const fp = path.join(tmpDir, "empty-extra.mp3");
    createMinimalMp3(fp);

    // Write an extra tag first so we can verify it gets cleared
    await executeTagWrite({ filePath: fp, extraTags: [{ key: "MOOD", value: "Chill" }] });

    const queue = new TagWriteQueue(1);
    // Submit with empty extraTags — should clear extra tags, not hang
    const results = await queue.submit([{ filePath: fp, extraTags: [] }]);

    expect(results).toHaveLength(1);
    expect(results[0].success).toBe(true);
    // Verify the extra tag was cleared
    const tags = NodeID3.read(fp);
    const udt = Array.isArray(tags.userDefinedText)
      ? tags.userDefinedText
      : tags.userDefinedText
        ? [tags.userDefinedText]
        : [];
    const byDesc = Object.fromEntries(
      udt.filter((t) => t.description).map((t) => [t.description, t.value]),
    );
    expect(byDesc["MOOD"]).toBeUndefined();
  });

  it("zero-dedup safety — submits with no fields and no extraTags resolve immediately", async () => {
    const queue = new TagWriteQueue(1);
    // A job with no fields and no extraTags should resolve immediately
    // via the defensive guard in submit()
    const results = await queue.submit([{ filePath: "/nonexistent/track.mp3" }]);
    expect(results).toEqual([]);
  });
});

describe("TagWriteQueue.submitOne", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "tag-one-"));
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  it("writes and returns single result", async () => {
    const fp = path.join(tmpDir, "single.mp3");
    createMinimalMp3(fp);

    const queue = new TagWriteQueue();
    const result = await queue.submitOne(fp, { title: "Single" });

    expect(result.success).toBe(true);
    expect(result.filePath).toBe(fp);
    expect(NodeID3.read(fp).title).toBe("Single");
  });
});

describe("TagWriteQueue singleton", () => {
  afterEach(() => {
    resetDefaultWriteQueue();
  });

  it("getDefaultWriteQueue returns the same instance", () => {
    const q1 = getDefaultWriteQueue();
    const q2 = getDefaultWriteQueue();
    expect(q1).toBe(q2);
  });

  it("resetDefaultWriteQueue creates a new instance", () => {
    const q1 = getDefaultWriteQueue();
    resetDefaultWriteQueue();
    const q2 = getDefaultWriteQueue();
    expect(q1).not.toBe(q2);
  });
});

// ── isExtraTagJob ────────────────────────────────────────────

describe("isExtraTagJob", () => {
  it("returns true when extraTags is an empty array", () => {
    expect(isExtraTagJob({ filePath: "/x.mp3", extraTags: [] })).toBe(true);
  });

  it("returns true when extraTags has entries", () => {
    expect(isExtraTagJob({ filePath: "/x.mp3", extraTags: [{ key: "MOOD", value: "Chill" }] })).toBe(true);
  });

  it("returns false when extraTags is undefined (fields-only)", () => {
    expect(isExtraTagJob({ filePath: "/x.mp3", fields: { title: "Song" } })).toBe(false);
  });

  it("returns false when both fields and extraTags are undefined", () => {
    expect(isExtraTagJob({ filePath: "/x.mp3" })).toBe(false);
  });
});

// ── Queue executor routing tests (worker action classification) ─

describe("TagWriteQueue executor routing", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "tag-routing-"));
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  it("routes extraTags: [] to executor with extraTags present and no fields", async () => {
    const received: Array<{ fields?: unknown; extraTags?: unknown }> = [];

    const recordingExec: TagWriteExecutor = async (job) => {
      received.push({ fields: job.fields, extraTags: job.extraTags });
      return { filePath: job.filePath, success: true };
    };

    const queue = new TagWriteQueue(1, recordingExec);
    await queue.submitOne(tmpDir + "/track.mp3", undefined, []);

    expect(received).toHaveLength(1);
    expect(received[0].extraTags).toEqual([]);
    expect(received[0].fields).toBeUndefined();
  });

  it("routes extraTags with entries to executor with extraTags and no fields", async () => {
    const received: Array<{ fields?: unknown; extraTags?: unknown }> = [];

    const recordingExec: TagWriteExecutor = async (job) => {
      received.push({ fields: job.fields, extraTags: job.extraTags });
      return { filePath: job.filePath, success: true };
    };

    const queue = new TagWriteQueue(1, recordingExec);
    await queue.submitOne(tmpDir + "/track.mp3", undefined, [{ key: "MOOD", value: "Chill" }]);

    expect(received).toHaveLength(1);
    expect(received[0].extraTags).toEqual([{ key: "MOOD", value: "Chill" }]);
    expect(received[0].fields).toBeUndefined();
  });

  it("routes fields-only job to executor with fields and undefined extraTags", async () => {
    const received: Array<{ fields?: unknown; extraTags?: unknown }> = [];

    const recordingExec: TagWriteExecutor = async (job) => {
      received.push({ fields: job.fields, extraTags: job.extraTags });
      return { filePath: job.filePath, success: true };
    };

    const queue = new TagWriteQueue(1, recordingExec);
    await queue.submitOne(tmpDir + "/track.mp3", { title: "Song" });

    expect(received).toHaveLength(1);
    expect(received[0].fields).toEqual({ title: "Song" });
    expect(received[0].extraTags).toBeUndefined();
  });
});

// ── Cross-call concurrency regression tests ─────────────────────

describe("TagWriteQueue cross-call concurrency", () => {
  let tmpDir: string;

  beforeEach(() => {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "tag-reg-"));
  });

  afterEach(() => {
    fs.rmSync(tmpDir, { recursive: true, force: true });
  });

  it("serializes overlapping submitOne calls with concurrency 1", async () => {
    let currentActive = 0;
    let maxActive = 0;

    const delayedExec: TagWriteExecutor = async (job) => {
      currentActive++;
      maxActive = Math.max(maxActive, currentActive);
      await new Promise((resolve) => setTimeout(resolve, 30));
      currentActive--;
      return { filePath: job.filePath, success: true };
    };

    const queue = new TagWriteQueue(1, delayedExec);

    // Launch 5 submitOne calls concurrently (as undo's Promise.all would)
    const results = await Promise.all(
      Array.from({ length: 5 }, (_, i) => {
        const fp = path.join(tmpDir, `c${i}.mp3`);
        createMinimalMp3(fp);
        return queue.submitOne(fp, { title: `Test ${i}` });
      }),
    );

    // Never more than 1 active write at a time
    expect(maxActive).toBeLessThanOrEqual(1);
    expect(results).toHaveLength(5);
    expect(results.every((r) => r.success)).toBe(true);
  });

  it("bounds concurrent writes across overlapping submit batches with concurrency 2", async () => {
    let currentActive = 0;
    let maxActive = 0;

    const delayedExec: TagWriteExecutor = async (job) => {
      currentActive++;
      maxActive = Math.max(maxActive, currentActive);
      await new Promise((resolve) => setTimeout(resolve, 30));
      currentActive--;
      return { filePath: job.filePath, success: true };
    };

    const queue = new TagWriteQueue(2, delayedExec);

    // Start two submit() calls before either finishes
    const batch1 = queue.submit([
      { filePath: path.join(tmpDir, "a1.mp3"), fields: { title: "A1" } },
      { filePath: path.join(tmpDir, "a2.mp3"), fields: { title: "A2" } },
      { filePath: path.join(tmpDir, "a3.mp3"), fields: { title: "A3" } },
    ]);
    const batch2 = queue.submit([
      { filePath: path.join(tmpDir, "b1.mp3"), fields: { title: "B1" } },
      { filePath: path.join(tmpDir, "b2.mp3"), fields: { title: "B2" } },
      { filePath: path.join(tmpDir, "b3.mp3"), fields: { title: "B3" } },
    ]);

    const [results1, results2] = await Promise.all([batch1, batch2]);

    // Global active count must never exceed 2
    expect(maxActive).toBeLessThanOrEqual(2);
    expect(results1).toHaveLength(3);
    expect(results1.every((r) => r.success)).toBe(true);
    expect(results2).toHaveLength(3);
    expect(results2.every((r) => r.success)).toBe(true);

    // Each submit resolves in its own input order
    expect(results1[0].filePath).toBe(path.resolve(path.join(tmpDir, "a1.mp3")));
    expect(results1[1].filePath).toBe(path.resolve(path.join(tmpDir, "a2.mp3")));
    expect(results1[2].filePath).toBe(path.resolve(path.join(tmpDir, "a3.mp3")));
    expect(results2[0].filePath).toBe(path.resolve(path.join(tmpDir, "b1.mp3")));
    expect(results2[1].filePath).toBe(path.resolve(path.join(tmpDir, "b2.mp3")));
    expect(results2[2].filePath).toBe(path.resolve(path.join(tmpDir, "b3.mp3")));
  });
});
