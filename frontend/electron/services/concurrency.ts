/**
 * Shared concurrency utilities for auto-tagger.
 *
 * Provides bounded-Promise concurrency building blocks and internal defaults
 * for local reads, local writes, auto-tag album processing, and audit album
 * processing. All defaults are computed from availableParallelism() and are
 * not user-configurable.
 */

import { availableParallelism } from "node:os";

// ── Internal defaults ──────────────────────────────────────────────

/** Max concurrent local file reads (clamped 4-12). */
export const LOCAL_READ_CONCURRENCY = clamp(availableParallelism(), 4, 12);

/** Max concurrent local file writes (clamped 1-4, half of available CPUs). */
export const LOCAL_WRITE_CONCURRENCY = clamp(
  Math.floor(availableParallelism() / 2),
  1,
  4,
);

/** Max concurrent album auto-tag operations. */
export const AUTO_TAG_ALBUM_CONCURRENCY = 2;

/** Max concurrent album audit operations. */
export const AUDIT_ALBUM_CONCURRENCY = 2;

// ── Helpers ────────────────────────────────────────────────────────

function clamp(value: number, min: number, max: number): number {
  return Math.max(min, Math.min(max, value));
}

// ── Bounded concurrent map ─────────────────────────────────────────

/**
 * Run an async function over each item with bounded concurrency.
 * Preserves input order in the results array.
 * Propagates the first rejection immediately; remaining items are not started.
 *
 * @param items - Array of input items.
 * @param concurrency - Maximum number of concurrent `fn` invocations (> 0).
 * @param fn - Async mapping function, called with (item, index).
 * @returns Results in the same order as `items`.
 */
export async function mapConcurrent<T, U>(
  items: T[],
  concurrency: number,
  fn: (item: T, index: number) => Promise<U>,
): Promise<U[]> {
  if (concurrency <= 0) {
    throw new Error("mapConcurrent: concurrency must be > 0");
  }
  if (items.length === 0) return [];

  const results: U[] = new Array(items.length);
  let nextIndex = 0;
  let rejected = false;

  const worker = async (): Promise<void> => {
    while (nextIndex < items.length) {
      if (rejected) return;
      const i = nextIndex++;
      try {
        results[i] = await fn(items[i], i);
      } catch (err) {
        rejected = true;
        throw err;
      }
    }
  };

  const poolSize = Math.min(concurrency, items.length);
  const workers = Array.from({ length: poolSize }, () => worker());
  await Promise.all(workers);
  return results;
}

// ── Bounded concurrent forEach ─────────────────────────────────────

/**
 * Run an async function over each item with bounded concurrency.
 * Rejects on first error; remaining items are not started.
 */
export async function forEachConcurrent<T>(
  items: T[],
  concurrency: number,
  fn: (item: T, index: number) => Promise<void>,
): Promise<void> {
  await mapConcurrent(items, concurrency, fn);
}

// ── Bounded concurrent map (collect errors) ────────────────────────

/**
 * Run an async function over each item with bounded concurrency.
 * Collects errors instead of stopping on the first one.
 * Preserves input order for successful results.
 *
 * @returns Separated successful results and collected errors.
 */
export async function mapConcurrentContinue<T, U>(
  items: T[],
  concurrency: number,
  fn: (item: T, index: number) => Promise<U>,
): Promise<{ results: U[]; errors: Error[] }> {
  const tagged = await mapConcurrent(
    items,
    concurrency,
    async (item: T, index: number) => {
      try {
        const value = await fn(item, index);
        return { ok: true as const, value };
      } catch (err) {
        return {
          ok: false as const,
          error: err instanceof Error ? err : new Error(String(err)),
        };
      }
    },
  );

  const results: U[] = [];
  const errors: Error[] = [];
  for (const t of tagged) {
    if (t.ok) {
      results.push(t.value);
    } else {
      errors.push(t.error);
    }
  }
  return { results, errors };
}
