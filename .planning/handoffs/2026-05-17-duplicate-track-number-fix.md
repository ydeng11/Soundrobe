# Handoff: Duplicate Track Number Auto-Fix

**Date:** 2026-05-17  
**Branch:** (current)  
**State:** All existing tests pass, 5 new tests added

---

## What Changed

### Problem

Three 陈慧娴 albums were blocked from tagging by `metadata.duplicate_track_number` (ERROR) and `metadata.track_sequence_gap` (WARNING):

| Album | Tracks | Issue |
|-------|--------|-------|
| 反叛 (1986) | 11 | Duplicate track 1 on disc 1 |
| Get Up And Dance (1991) | 8 | Duplicate track 1 on disc 1 |
| 永远是你的陈慧娴 (1991) | 34 | Duplicate track 1 on disc 1 |

These errors caused `health_report.can_tag = False`, which blocked the YOLO write path. The existing `_fix_metadata()` cascade (DB lookup → LLM fallback) could potentially fix these, but for albums that don't appear in MusicBrainz/Discogs — common with niche 80s/90s Cantopop — no candidate is found and the files remain untagged.

### Root Cause

Audio files had correct filenames (e.g. `01 反叛 (序曲).flac`, `02 反叛.flac`) but their internal metadata tags both had `track_number=1`. This creates duplicate track numbers at the album level, which the health report flags as an ERROR-level blocking issue.

### Fix: Filename-Based Renumbering

**`_fix_duplicate_track_numbers()`** — a new method on `AlbumWorkflow` that runs before the external lookup cascade. It:

1. Scans the health report for `metadata.duplicate_track_number` issues
2. For each group of paths sharing the same duplicate track number, extracts the leading digit from each filename (e.g. `01 Song.flac` → 1)
3. Only applies the fix when **all** of these conditions hold:
   - Every file in the group has a leading numeric prefix
   - The extracted numbers are **unique** within the group
   - No extracted number **conflicts** with a non-duplicate track on the same disc
4. Writes the corrected track number to each file's metadata
5. Rebuilds the health report

If any condition fails (no prefix, non-unique prefixes, conflict with existing tracks), the fix is skipped and the normal lookup/LLM cascade proceeds.

### Where it runs in the workflow

```
AlbumWorkflow.run()
  ├── read_metadata()
  ├── build initial health report
  ├── exclude stray tracks (existing logic)
  │
  ├── NEW: _fix_duplicate_track_numbers()  ← runs BEFORE can_write check
  │     └── fixes track_number from filename prefixes
  │     └── rebuilds health report
  │
  ├── can_write = health_report.can_tag && yolo   ← now fresh after fix
  ├── _fix_metadata() (lookup cascade)           ← runs only if still needed
  └── write enriched metadata + cover art
```

### Key design decisions

| Decision | Rationale |
|----------|-----------|
| **Runs before lookup cascade** | Fixes the blocking error early, making `can_tag=True` without expensive external calls |
| **Only modifies duplicate files** | Leaves non-duplicate tracks untouched — conservative, minimal change |
| **Try/except on write failures** | Matches existing pattern in `_write_candidate_metadata()`; handles corrupt/empty files gracefully |
| **No-op when filenames lack prefixes** | Can't derive correct numbers → falls through to lookup/LLM path |
| **No-op when filenames are also duplicates** | Both files named `01 Intro.flac` and `01 Song.flac` → ambiguous → falls through |

### Extracted module-level helpers

Two functions were extracted from the nested scope inside `_match_tracks_to_files()` to module level so both methods can use them:

- **`_stem_track_number(stem)`** — extracts leading digits from filename stem (`"01 Song"` → `1`)
- **`_clean_stem(stem)`** — strips leading digits + separator from filename stem (`"01 Song"` → `"Song"`)

The nested versions inside `_match_tracks_to_files()` were replaced with a comment pointing to the module-level versions.

### Test coverage

5 new tests in `tests/test_album_workflow.py`:

| Test | What it verifies |
|------|------------------|
| `test_fix_duplicate_track_numbers_renumbers_from_filenames` | Core fix: 3 files, 2 with track=1 → renumbered to 1, 2, 3; `can_tag=True` |
| `test_fix_duplicate_track_numbers_skipped_when_no_duplicates` | No-op when health report is clean |
| `test_fix_duplicate_track_numbers_skipped_when_filenames_lack_number_prefix` | No-op when filenames like `SongA.flac` have no leading digits |
| `test_fix_duplicate_track_numbers_skipped_when_stem_numbers_not_unique` | No-op when both duplicates are `01 Intro.flac` + `01 Song.flac` (same prefix) |
| `test_fix_duplicate_track_numbers_end_to_end_via_workflow_yolo` | Full workflow: mocks health report with duplicates, verifies fix runs before `_fix_metadata` |

### Files changed

| File | Change |
|------|--------|
| `src/soundrobe/workflows/album.py` | + `_stem_track_number()`, `_clean_stem()` (module-level), + `_fix_duplicate_track_numbers()` method, + integration in `run()`, - nested duplicates of `_clean_stem`/`_stem_track_number` |
| `tests/test_album_workflow.py` | + 5 new test functions |

---

## For the Next Developer

### To test the fix on real albums

```bash
# Generate a health report first
auto-tag batch "/Volumes/downloads/陈慧娴" --dry-run --health-report health.json

# Tag with the fix (requires --yolo)
auto-tag tag "/Volumes/downloads/陈慧娴/1986 - 反叛" --yolo --verbose

# Or batch tag all albums
auto-tag batch "/Volumes/downloads/陈慧娴" --yolo
```

### If the fix doesn't work for a specific album

Check which condition fails:
1. **No filename prefix**: Files named `反叛.flac` instead of `01 反叛.flac`
2. **Non-unique prefixes**: Both files are `01 Intro.flac` and `01 Song.flac`
3. **Conflict with existing tracks**: The stem number matches a non-duplicate track

If filenames lack prefixes, the fix is not applicable — the files need the lookup cascade or manual intervention.

### Edge cases to be aware of

- **Multi-disc albums with disc-level duplicates**: The fix operates per duplicate group (same disc + same track). If files on different discs have the same track number (e.g., disc 1 track 1, disc 2 track 1), they're different groups and not affected.
- **Large compilations**: 永远是你的陈慧娴 has 34 tracks. If the filenames have proper prefixes (`01`–`34`), the fix handles all of them.
- **Zero-padding**: `_stem_track_number` matches 1-2 digits only (`\d{1,2}`). Files with 3-digit prefixes (`001`) won't have their track number extracted — this is by design to avoid false positives from numeric album titles.

### Failure modes

If `_fix_duplicate_track_numbers` throws an unhandled exception, `run()` will raise and the batch process will catch it as a failed album:

```python
for album in albums:
    processed += 1
    try:
        result = self.album_workflow_factory(self.settings).run(
            album, dry_run=dry_run, ...
        )
    except Exception as exc:
        failed += 1
        errors.append(f"{album}: {exc}")
        continue
```

Currently, the write failures are caught inside the method (try/except around `write_metadata`), so the only way to get an unhandled exception is if `_stem_track_number` or the health report rebuild fails unexpectedly.

### Related code

- **`metadata_validation.py`**: `_check_duplicate_tracks()` — generates the `metadata.duplicate_track_number` issue with `{"paths": [...]}` in details
- **`health.py`**: `AlbumHealthReport.can_tag` property — returns `False` when any ERROR-severity issue exists
- **Existing exclusion logic in `run()`**: Pattern 3 handles the case where duplicate track number is caused by missing disc_number (one file has `disc=None`, another has `disc=1`) — this is a different scenario from same-disc duplicates
