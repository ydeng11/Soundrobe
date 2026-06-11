import { describe, it, expect, vi } from "vitest";
import {
  mapConcurrent,
  mapConcurrentContinue,
  forEachConcurrent,
  LOCAL_READ_CONCURRENCY,
  LOCAL_WRITE_CONCURRENCY,
  AUTO_TAG_ALBUM_CONCURRENCY,
  AUDIT_ALBUM_CONCURRENCY,
} from "../../electron/services/concurrency";

describe("constants", () => {
  it("LOCAL_READ_CONCURRENCY is between 4 and 12", () => {
    expect(LOCAL_READ_CONCURRENCY).toBeGreaterThanOrEqual(4);
    expect(LOCAL_READ_CONCURRENCY).toBeLessThanOrEqual(12);
  });

  it("LOCAL_WRITE_CONCURRENCY is between 1 and 4", () => {
    expect(LOCAL_WRITE_CONCURRENCY).toBeGreaterThanOrEqual(1);
    expect(LOCAL_WRITE_CONCURRENCY).toBeLessThanOrEqual(4);
  });

  it("AUTO_TAG_ALBUM_CONCURRENCY is 2", () => {
    expect(AUTO_TAG_ALBUM_CONCURRENCY).toBe(2);
  });

  it("AUDIT_ALBUM_CONCURRENCY is 2", () => {
    expect(AUDIT_ALBUM_CONCURRENCY).toBe(2);
  });
});

describe("mapConcurrent", () => {
  it("preserves input order", async () => {
    const input = [10, 20, 30, 40, 50];
    const results = await mapConcurrent(input, 3, async (n) => n * 2);
    expect(results).toEqual([20, 40, 60, 80, 100]);
  });

  it("caps the number of concurrent workers", async () => {
    let concurrent = 0;
    let maxConcurrent = 0;

    await mapConcurrent([1, 2, 3, 4, 5], 2, async (n) => {
      concurrent++;
      maxConcurrent = Math.max(maxConcurrent, concurrent);
      // Simulate work
      await new Promise((r) => setTimeout(r, 10));
      concurrent--;
      return n;
    });

    expect(maxConcurrent).toBeLessThanOrEqual(2);
  });

  it("handles empty input", async () => {
    const results = await mapConcurrent([], 3, async (n) => n);
    expect(results).toEqual([]);
  });

  it("propagates errors and stops remaining workers", async () => {
    const executed: number[] = [];

    await expect(
      mapConcurrent([1, 2, 3, 4, 5], 2, async (n) => {
        executed.push(n);
        if (n === 2) throw new Error("stop");
        return n;
      }),
    ).rejects.toThrow("stop");
  });

  it("throws on concurrency <= 0", async () => {
    await expect(
      mapConcurrent([1], 0, async (n) => n),
    ).rejects.toThrow("mapConcurrent: concurrency must be > 0");
  });

  it("limits concurrency to array length when array is smaller", async () => {
    const results = await mapConcurrent([1, 2], 100, async (n) => n * 10);
    expect(results).toEqual([10, 20]);
  });

  it("works with single-item array", async () => {
    const results = await mapConcurrent(["only"], 1, async (s) => s.toUpperCase());
    expect(results).toEqual(["ONLY"]);
  });

  it("calls fn with correct index", async () => {
    const indices: number[] = [];
    await mapConcurrent(["a", "b", "c"], 2, async (_, i) => {
      indices.push(i);
      return i;
    });
    expect(indices.sort()).toEqual([0, 1, 2]);
  });
});

describe("mapConcurrentContinue", () => {
  it("collects errors instead of stopping", async () => {
    const { results, errors } = await mapConcurrentContinue(
      [1, 2, 3, 4, 5],
      3,
      async (n) => {
        if (n % 2 === 0) throw new Error(`even: ${n}`);
        return n * 10;
      },
    );

    expect(results.sort()).toEqual([10, 30, 50]);
    expect(errors).toHaveLength(2);
    expect(errors[0].message).toBe("even: 2");
    expect(errors[1].message).toBe("even: 4");
  });

  it("returns empty arrays for all-failing tasks", async () => {
    const { results, errors } = await mapConcurrentContinue(
      [1, 2],
      2,
      async () => {
        throw new Error("fail");
      },
    );

    expect(results).toEqual([]);
    expect(errors).toHaveLength(2);
  });

  it("returns all results for all-passing tasks", async () => {
    const { results, errors } = await mapConcurrentContinue(
      [10, 20, 30],
      2,
      async (n) => n + 1,
    );

    expect(results).toEqual([11, 21, 31]);
    expect(errors).toEqual([]);
  });

  it("handles empty input", async () => {
    const { results, errors } = await mapConcurrentContinue([], 3, async () => 1);
    expect(results).toEqual([]);
    expect(errors).toEqual([]);
  });
});

describe("forEachConcurrent", () => {
  it("runs all items with bounded concurrency", async () => {
    const visited: number[] = [];

    await forEachConcurrent([1, 2, 3, 4], 2, async (n) => {
      visited.push(n);
    });

    expect(visited.sort()).toEqual([1, 2, 3, 4]);
  });

  it("handles empty input", async () => {
    await expect(forEachConcurrent([], 2, async () => {})).resolves.toBeUndefined();
  });
});
