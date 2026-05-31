/**
 * ExtraTagService — reusable extra tag operations.
 *
 * Shared between IPC handlers and the assistant runtime.
 * Preserves the reserved-key behavior from writer.ts.
 * Extra-tag changes are reversible by storing previous extra tags.
 */

import { writeExtraTags, batchWriteExtraTags } from "../handlers/writer";
import type { ExtraTagUpdate } from "../handlers/writer";
import { readExtraTags, readTrackMetadata } from "../handlers/tracks";
import type { ExtraTag as TrackExtraTag } from "../handlers/tracks";
import type { TrackData } from "../handlers/tracks";

export interface ExtraTagPlanInput {
  trackPath: string;
  upserts: ExtraTagUpdate[];
  removes: string[]; // Keys to remove
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
   */
  async applyExtraTagUpdates(
    inputs: ExtraTagPlanInput[],
  ): Promise<ExtraTagApplyResult[]> {
    const results: ExtraTagApplyResult[] = [];

    for (const input of inputs) {
      try {
        // Build the full tag set: current tags minus removes, plus upserts
        const currentTags = await readExtraTags(input.trackPath);
        const removeKeys = new Set(input.removes.map((k) => k.trim().toLowerCase()));

        // Keep tags not in remove list and not in upsert list (will be re-added)
        const keptTags: ExtraTagUpdate[] = currentTags
          .filter((tag) => !removeKeys.has(tag.key.toLowerCase()))
          .map((tag) => ({ key: tag.key, value: tag.value }));

        // Merge upserts (overwriting any existing kept tag)
        const upsertMap = new Map<string, string>();
        for (const tag of keptTags) {
          upsertMap.set(tag.key.toLowerCase(), tag.value);
        }
        for (const upsert of input.upserts) {
          upsertMap.set(upsert.key.trim().toLowerCase(), upsert.value.trim());
        }

        // Build final list
        const finalTags: ExtraTagUpdate[] = [];
        for (const [key, value] of upsertMap) {
          // Find original key casing from kept tags or upserts
          const original = keptTags.find(
            (t) => t.key.toLowerCase() === key,
          )?.key
            ?? input.upserts.find(
              (u) => u.key.trim().toLowerCase() === key,
            )?.key.trim()
            ?? key;
          finalTags.push({ key: original, value });
        }

        await writeExtraTags(input.trackPath, finalTags);
        results.push({ trackPath: input.trackPath, success: true });
      } catch (error) {
        results.push({
          trackPath: input.trackPath,
          success: false,
          error: error instanceof Error ? error.message : String(error),
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
