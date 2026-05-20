# Handoff: Sequential Track Renumbering Fallback

**Date:** 2026-05-18  
**Branch:** (current)  
**State:** All 308 tests pass

---

## What Changed

### Problem

After the 2026-05-17 handoff that added filename-prefix-based renumbering, 5 more 陈慧娴 albums remained blocked by `metadata.duplicate_track_number`. These albums have filenames like `陈慧娴-歌名.flac` or `陈慧娴 - .歌名.flac` — no leading numeric prefix, so the prefix-based fix was a no-op.

The lookup cascade (`_fix_metadata()`) CAN fix some of these (e.g., 痴心傻女 matched to 傻女 via beets), but others may fail if no candidate is found in MusicBrainz/Discogs/LLM, leaving files completely untagged even in YOLO mode.

### Root Cause

Three distinct filename patterns that lack numeric prefixes:

| Album | Files | Filename pattern |
|-------|-------|-----------------|
| 痴心傻女 (2001) | 15 | `陈慧娴-歌名.flac` / `陈慧娴 - .歌名.flac` |
| 归来吧 (1992) | 11 | `陈慧娴-歌名.flac` |
| 娴情时间 (2001) | 16 | `陈慧娴-歌名.flac` |
| 情意结 (2003) | 10 | Mixed `陈慧娴-...` and `陈慧娴 - .` |
| BTB-千年恋人 (2007) | 10 | Multi-artist, no numeric prefix |

Additionally, albums where ALL files share track_number=1 (like 痴心傻女) also trigger Pattern 3 of the exclusion logic, which deletes files with `disc_number=None` that belong to a duplicate group. This was a secondary ordering bug: the duplicate fix ran AFTER exclusion, so files were deleted before the fix could save them.

### Fix: Two Changes

#### 1. Reorder: Duplicate fix runs before exclusion

Moved `_fix_duplicate_track_numbers()` to run BEFORE Pattern 3 exclusion logic. Previously the order was:

```
read_metadata → build health → EXCLUDE strays → fix duplicates → lookup cascade
```

Now:

```
read_metadata → build health → FIX DUPLICATES → exclude strays → lookup cascade
```

This prevents Pattern 3 from falsely deleting files from all-track-1 albums. Once duplicates are resolved, each file has a unique track number and no duplicate group exists to trigger the stray heuristic.

#### 2. Strategy 2: Sequential renumbering fallback in `_fix_duplicate_track_numbers()`

Added a second strategy inside the existing method:

- **Strategy 1 (prefix-based, existing):** Extract numbers from leading digits in filenames (e.g., `01 Song.flac` → 1). Unchanged.
- **Strategy 2 (sequential, new):** When Strategy 1 doesn't resolve all duplicates, group files by effective disc (treating `disc_number=None` as disc 1). If ALL files on an effective disc share the same track number (e.g., all track=1), renumber them sequentially 1..N based on filename sort order.

Strategy 2 conditions:
- All files on the effective disc share the same non-None track number
- At least one of those files is still flagged as a duplicate in the health report
- Normalises `disc_number` from None to 1 when writing, preventing false overlap in the rebuilt health report

**Deliberately does NOT apply to partial duplicates** (e.g., 归来吧 where tracks 5 and 9 each duplicated once alongside other tracks). Those still need the lookup cascade.

### Where it runs in the workflow

```
AlbumWorkflow.run()
  ├── read_metadata()
  ├── build initial health report
  │
  ├── NEW: _fix_duplicate_track_numbers()  ← moved BEFORE exclusion
  │     ├── Strategy 1: prefix-based renumbering (existing)
  │     └── NEW Strategy 2: sequential renumbering (all-track-1 fallback)
  │     └── rebuilds health report
  │
  ├── exclude stray tracks (Patterns 1-4)  ← now sees clean track numbers
  ├── rebuild health report if tracks excluded
  ├── _fix_metadata() (lookup cascade)
  └── write enriched metadata + cover art
```

### New method: `_fix_duplicate_tracks_sequential()`

Extracted into a separate method called from `_fix_duplicate_track_numbers()` when Strategy 1 leaves unresolved duplicates.

Key design:
- Normalises `disc_number=None` → disc 1 for grouping AND writing
- Verifies files are still flagged as duplicates (avoid re-fixing already-clean discs)
- Sorts by filename for deterministic track order
- Writes metadata inline, wrapping exceptions per-file

### Files changed

| File | Change |
|------|--------|
| `src/auto_tagger/workflows/album.py` | Moved `_fix_duplicate_track_numbers()` call before exclusion in `run()`; added Strategy 2 + `_fix_duplicate_tracks_sequential()`; moved `fix_messages` init earlier |
| `tests/test_album_workflow.py` | Updated 2 existing tests to expect `True` (was `False`), added assertion for sequential renumbering values |

### Updated test coverage

| Test | What it verifies |
|------|------------------|
| `test_fix_duplicate_track_numbers_renumbers_from_filenames` | Prefix-based: 3 files, 2 with track=1 → renumbered to 1, 2, 3 (unchanged) |
| `test_fix_duplicate_track_numbers_skipped_when_no_duplicates` | No-op when clean (unchanged) |
| `test_fix_duplicate_track_numbers_renumbers_sequentially_when_filenames_lack_prefix` | **Renamed from skipped**: SongA/SongB both track=1, no prefix → sequential 1, 2 |
| `test_fix_duplicate_track_numbers_skipped_when_stem_numbers_not_unique` | **Updated**: 01 Intro/01 Song share prefix but both track=1 → sequential 1, 2 via Strategy 2 |
| `test_fix_duplicate_track_numbers_end_to_end_via_workflow_yolo` | Full YOLO workflow with mock (unchanged, message updated) |

---

## For the Next Developer

### Albums this fixes directly

| Album | Issue | Fixed by |
|-------|-------|----------|
| 痴心傻女 (2001) | All 15 files track=1 | Strategy 2 sequential renumbering |

### Albums that still need the lookup cascade

| Album | Issue | Expected path |
|-------|-------|--------------|
| 归来吧 (1992) | Partial dupes on tracks 5, 9 | Lookup cascade (beets/Discogs/LLM) |
| 娴情时间 (2001) | Partial dupe on track 1 | Lookup cascade |
| 情意结 (2003) | Partial dupes on tracks 6, 8 | Lookup cascade |
| BTB-千年恋人 (2007) | Partial dupe on track 1 | Lookup cascade |
| 永远是你的陈慧娴 (1991) | Mixed prefixed/non-prefixed | Prefix fix for the 01-14 files, rest via cascade |

### Running the batch

```bash
# The batch is slow due to many API calls (Discogs per-variant + retry). Use --parallel 1:
just batch "/Volumes/downloads/陈慧娴" --yolo --parallel 1

# For faster iteration on specific albums:
just tag "/Volumes/downloads/陈慧娴/2001 - 痴心傻女" --yolo
```

### Edge cases in Strategy 2

- **Mixed disc=None and disc=1 files**: Grouped together (both treated as disc 1), assigned sequential tracks across the combined set. disc_number normalised to 1 for files that had None.
- **Already-clean discs**: If no files from a disc appear in any duplicate-track-number issue, Strategy 2 skips that disc entirely.
- **Multi-disc albums**: Each effective disc is processed independently. E.g., disc 1 group with all-track-1 gets tracks 1..N; disc 2 group with all-track-1 also gets tracks 1..N.
- **Files with real audio but `write_metadata` fails**: Caught by existing try/except; that file is skipped but other files in the group still get renumbered.

### Failure modes

- If Strategy 1 and Strategy 2 both fail, `_fix_duplicate_track_numbers` returns False and the normal `needs_fix → _fix_metadata()` path runs.
- If `_fix_metadata()` also fails (no candidate found), the album stays blocked. This is the existing behavior.

### Related code

- **`metadata_validation.py`**: `_check_duplicate_tracks()` — `disc = metadata.disc_number or 1` grouping is why disc=None and disc=1 files collide
- **Pattern 3 in `run()`**: Now runs after duplicate fix, so it sees unique track numbers and doesn't false-positive on all-track-1 albums
- **`_fix_duplicate_tracks_sequential()`**: Self-contained on `AlbumWorkflow`, takes same params as `_fix_duplicate_track_numbers()`
