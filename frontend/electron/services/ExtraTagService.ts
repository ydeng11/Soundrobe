/**
 * ExtraTagService — reusable extra tag operations.
 *
 * Shared between IPC handlers and the assistant runtime.
 * Preserves the reserved-key behavior from writer.ts.
 * Extra-tag changes are reversible by storing previous extra tags.
 */

import type { ExtraTagUpdate } from "../handlers/writer";
import { readExtraTags } from "../handlers/tracks";
import type { ExtraTag as TrackExtraTag } from "../handlers/tracks";
import { getDefaultWriteQueue } from "./TagWriteQueue";

export interface ExtraTagPlanInput {
  trackPath: string;
  upserts: ExtraTagUpdate[];
  removes: string[];
}

export interface ExtraTagAction {
  trackPath: string;
  key: string;
  operation: "upsert" | "remove";
  oldValue: string | null;
  newValue: string | null;
}

export interface ExtraTagPlanBatch {
  kind: "extra-tag-update";
  summary: string;
  actions: ExtraTagAction[];
  affectedTracks: number;
  reversible: boolean;
}

export interface ExtraTagApplyResult {
  trackPath: string;
  success: boolean;
  error?: string;
}

export class ExtraTagService {
  /**
   * Read all extra tags for a track.
   */
  async readExtraTags(trackPath: string): Promise<TrackExtraTag[]> {
    return readExtraTags(trackPath);
  }

  /**
   * Plan extra tag changes without applying them.
   */
  async planExtraTagUpdates(
    inputs: ExtraTagPlanInput[],
  ): Promise<ExtraTagPlanBatch> {
    const actions: ExtraTagAction[] = [];

    for (const input of inputs) {
      const currentTags = await readExtraTags(input.trackPath);
      const currentByKey = new Map<string, string>();
      for (const tag of currentTags) {
        currentByKey.set(tag.key.toLowerCase(), tag.value);
      }

      // Plan removals
      for (const key of input.removes) {
        const normalizedKey = key.trim().toLowerCase();
        const oldValue = currentByKey.get(normalizedKey) ?? null;
        actions.push({
          trackPath: input.trackPath,
          key: key.trim(),
          operation: "remove",
          oldValue,
          newValue: null,
        });
      }

      // Plan upserts
      for (const upsert of input.upserts) {
        const normalizedKey = upsert.key.trim().toLowerCase();
        const currentValue = currentByKey.get(normalizedKey) ?? null;
        // Skip if same value
        if (currentValue === upsert.value.trim()) continue;
        actions.push({
          trackPath: input.trackPath,
          key: upsert.key.trim(),
          operation: "upsert",
          oldValue: currentValue,
          newValue: upsert.value.trim(),
        });
      }
    }

    const affectedPaths = new Set(actions.map((a) => a.trackPath));

    return {
      kind: "extra-tag-update",
      summary: actions.length > 0
        ? `Update ${actions.length} extra tag fields across ${affectedPaths.size} track(s)`
        : "No extra tag changes needed",
      actions,
      affectedTracks: affectedPaths.size,
      reversible: true,
    };
  }

  /**
   * Apply extra tag writes to disk.
   *
   * Optimization: pre-checks each track's current tags and skips tracks
   * where the operation would be a no-op (removal of non-existent key,
   * upsert of same value). This avoids unnecessary file I/O and queue
   * submissions for unchanged tracks.
   */
  async applyExtraTagUpdates(
    inputs: ExtraTagPlanInput[],
  ): Promise<ExtraTagApplyResult[]> {
    const results: ExtraTagApplyResult[] = [];
    // Build write jobs (prepare tag sets, then submit batch through queue)
    const jobs: Array<{ filePath: string; extraTags: ExtraTagUpdate[] }> = [];

    for (const input of inputs) {
      try {
        const currentTags = await readExtraTags(input.trackPath);
        const removeKeys = new Set(input.removes.map((k) => k.trim().toLowerCase()));

        // Build current key → value map for pre-checks
        const currentByKey = new Map<string, string>();
        for (const tag of currentTags) {
          currentByKey.set(tag.key.toLowerCase(), tag.value);
        }

        // Pre-check removals: skip if key doesn't exist on this track
        const effectiveRemoves = new Set<string>();
        for (const key of removeKeys) {
          if (currentByKey.has(key)) {
            effectiveRemoves.add(key);
          }
        }

        // Pre-check upserts: filter out no-op upserts (same value already exists)
        const effectiveUpserts = input.upserts.filter((upsert) => {
          const normalizedKey = upsert.key.trim().toLowerCase();
          const currentValue = currentByKey.get(normalizedKey) ?? null;
          return currentValue !== upsert.value.trim();
        });

        // If no effective removes and no effective upserts, skip this track
        if (effectiveRemoves.size === 0 && effectiveUpserts.length === 0) {
          results.push({ trackPath: input.trackPath, success: true });
          continue;
        }

        const keptTags: ExtraTagUpdate[] = currentTags
          .filter((tag) => !effectiveRemoves.has(tag.key.toLowerCase()))
          .map((tag) => ({ key: tag.key, value: tag.value }));

        const upsertMap = new Map<string, string>();
        for (const tag of keptTags) {
          upsertMap.set(tag.key.toLowerCase(), tag.value);
        }
        for (const upsert of effectiveUpserts) {
          upsertMap.set(upsert.key.trim().toLowerCase(), upsert.value.trim());
        }

        // Build map from normalized key → original-cased key for output
        const originalKeyMap = new Map<string, string>();
        for (const tag of keptTags) {
          originalKeyMap.set(tag.key.toLowerCase(), tag.key);
        }
        for (const upsert of effectiveUpserts) {
          const normalized = upsert.key.trim().toLowerCase();
          if (!originalKeyMap.has(normalized)) {
            originalKeyMap.set(normalized, upsert.key.trim());
          }
        }

        const finalTags: ExtraTagUpdate[] = [];
        for (const [key, value] of upsertMap) {
          finalTags.push({ key: originalKeyMap.get(key) ?? key, value });
        }

        jobs.push({ filePath: input.trackPath, extraTags: finalTags });
      } catch (error) {
        // If reading fails, record the error
        results.push({
          trackPath: input.trackPath,
          success: false,
          error: error instanceof Error ? error.message : String(error),
        });
      }
    }

    // Submit all write jobs through the concurrent queue
    if (jobs.length > 0) {
      const writeResults = await getDefaultWriteQueue().submit(jobs);
      for (const wr of writeResults) {
        results.push({
          trackPath: wr.filePath,
          success: wr.success,
          error: wr.error,
        });
      }
    }

    return results;
  }

  /**
   * Build previous-extra-tags snapshot for undo purposes.
   */
  async buildUndoExtraTags(
    inputs: ExtraTagPlanInput[],
  ): Promise<Array<{ path: string; extraTags: ExtraTagUpdate[] }>> {
    const snapshots: Array<{ path: string; extraTags: ExtraTagUpdate[] }> = [];

    for (const input of inputs) {
      const currentTags = await readExtraTags(input.trackPath);
      snapshots.push({
        path: input.trackPath,
        extraTags: currentTags.map((t) => ({ key: t.key, value: t.value })),
      });
    }

    return snapshots;
  }
}
