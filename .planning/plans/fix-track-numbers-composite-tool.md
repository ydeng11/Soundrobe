# Plan: New Composite Macro Tool — `auto_numbering_tracks`

## Context

The assistant panel has several composite macro tools (e.g., `edit_metadata`, `infer_tags_from_filenames`, `organize_files`, `group_by_album`, `run_library_task`) but none that specifically normalises/fixes track numbering. Users may have albums where tracks are missing `trackNumber`, have gaps in the sequence, or have inconsistent `trackTotal`. The new `fix_track_numbers` tool will inspect and repair these issues **without any API calls** — purely local metadata computation and preview.

## Approach

Add one new composite macro tool `auto_numbering_tracks` to the existing `buildMutatingTools()` function in `assistant.ts`. The tool:

1. Accepts `target_scope` (same pattern as other macros) plus optional parameters for what to fix.
2. Reads current metadata via the existing `readTrackMetadata()` / `currentAppState.tracks`.
3. Computes the fix plan entirely locally with NO API calls.
4. Creates a preview action batch like other composite tools.
5. The preview shows per-track changes (old → new trackNumber, trackTotal, discNumber, discTotal).

No new service class needed — the logic is simple enough to live in the executor function. The existing `TrackTagService` handles the actual tag writes when the batch is approved.

## Files to Modify

| File | Change |
|------|--------|
| `frontend/electron/handlers/assistant.ts` | Add `fix_track_numbers` tool definition in `buildMutatingTools()`; add helper functions for the fix logic |
| `frontend/test/services/assistant-composite-macros.test.ts` | Add tests for the new track-number fix logic |

## Reuse

- **`currentAppState.tracks`** — the loaded tracks provide all metadata needed to detect issues.
- **`resolveTargetPaths()`** — existing helper to resolve target_scope to file paths (reuse directly).
- **`TrackTagService.planTagUpdates()`** — after computing desired values, call this to produce the diff/preview (reuse existing preview pipeline).
- **`TrackTagService.buildUndoSnapshots()`** / **`TrackTagService.applyTagUpdates()`** — used when batch is approved via existing `applyMetadataUpdateBatch` path.
- **`currentRuntime.createActionBatch()`** — standard preview batch creation pattern.
- **`PER_TRACK_UNIQUE_FIELDS`** — already includes `trackNumber`, `trackTotal`, `discNumber`, `discTotal` so the existing guard in `edit_metadata` won't interfere (the new tool explicitly handles per-track values).

## Steps

- [ ] **Step 1: Define the fix plan logic** — Write a helper function (in `assistant.ts`) that takes a list of track paths and returns per-track desired values:
  - Strip leading zeros from existing track numbers (e.g. "01" → 1).
  - Group tracks by `discNumber` when multiple discs are present; treat single disc / null disc as one group.
  - Within each group, sort by current `trackNumber` (if present and valid) or by filename sort order (if all null).
  - Assign sequential `trackNumber` (1, 2, 3, ...) per group.
  - Set `trackTotal` to the total number of tracks in the group (per disc).
  - Set `discTotal` to the total number of discs (if multi-disc).
  - Return a list of `TagUpdateInstruction` objects.
  
- [ ] **Step 2: Register the tool** — Add the `auto_numbering_tracks` tool definition in `buildMutatingTools()`:
  - **name**: `auto_numbering_tracks`
  - **description**: "Composite macro: automatically fix track numbering for target tracks. Assigns sequential track numbers, strips leading zeros, renumbers per-disc when multi-disc, sets trackTotal to match count, and reorders by filename when all numbers are null. No API calls."
  - **inputSchema**: targets (same `target_scope` + `paths` pattern). The tool automatically detects multi-disc albums and handles per-disc numbering without needing a toggle.
  - **isReadOnly**: false
  - **riskLevel**: "low"
  
- [ ] **Step 3: Handle edge cases**:
  - Empty target → return "no tracks" message.
  - Tracks already correctly numbered (sequential, no gaps, trackTotal matches) → return "no changes needed".
  - Missing/null trackNumber → assign from filename sort order.
  - Gaps in numbering (1, 3, 5) → compact to sequential (1, 2, 3).
  - Leading zeros ("01", "02") → strip to plain integers (1, 2).
  - Mixed discs (e.g. tracks from disc 1 and disc 2) → group by disc, number per-disc starting from 1.
  
- [ ] **Step 4: Wire the preview batch** — Use `TrackTagService.planTagUpdates()` and `currentRuntime.createActionBatch()` following the same pattern as `infer_tags_from_filenames`.

- [ ] **Step 4b: Update system prompt** — Add `auto_numbering_tracks` to the composite macro tool list in the `SYSTEM_PROMPT` in `AssistantRuntime.ts` so the LLM knows about it.
  
- [ ] **Step 5: Write tests** in `assistant-composite-macros.test.ts`:
  - Test with tracks that have null track numbers → numbered by filename sort order.
  - Test with gaps (e.g., tracks numbered 1, 3, 5 → should become 1, 2, 3).
  - Test with leading zeros ("01", "02" → 1, 2).
  - Test with mixed discs (disc 1 tracks 1-3, disc 2 tracks 1-3).
  - Test already-correct numbering returns "no changes needed".
  - Test empty target returns appropriate message.

## Verification

1. `cd frontend && npm test -- --run test/services/assistant-composite-macros.test.ts` — verify new tests pass.
2. Manual verification in the Electron app: select an album with messy track numbers, run the assistant with "fix track numbers for the active album", verify the preview shows correct sequential numbering.
3. No regressions: run full test suite with `npm test`.
