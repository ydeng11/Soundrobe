# Plan: Redesign LLM inference for tag parsing + fallback generation

## Problem

Currently the LLM hint enhancement (Step 2) only runs when the folder name looks "ambiguous" (e.g. has brackets). It only sees the raw folder name, not existing file tags, and only extracts artist/album/year — which can be worse than the basic parser. The LLM output is used solely to overwrite search hints, never as a fallback candidate with genre.

## New flow

Move LLM inference **before API lookups**, give it **file path + existing tags**, ask it to produce **a complete tag map**. Use its output for both:
1. **Cleaned search params** → fed to MusicBrainz/Discogs
2. **Fallback candidate with genre** → used when APIs return nothing

### Priority order (preserved from current behavior)

Fields are never overwritten once set. The lookup chain priority is:

```
API candidates (MusicBrainz, Discogs)  >  LLM fallback  >  folder fallback (no genre)
```

The LLM's role is split:
1. **Search params** → corrected artist/album/year fed into API query URLs (no change to existing search flow)
2. **Fallback tag map** → only used when all API lookups return 0 candidates, providing genre and other fields the folder fallback lacks

### Step-by-step

```
Step 1:  Parse folder hints (as-is)
Step 2:  Read existing file metadata (already partially done in request.tracks)
Step 3:  LLM inference → full tag map + corrected search hints
Step 4:  Use corrected hints to search Dataset / Cache / MusicBrainz / Discogs
Step 5:  If all lookups return 0, use LLM's tag map as the fallback candidate (with genre!)
Step 6:  Apply tags (existing metadata preserved — LLM/null fields never overwrite what APIs set)
```

**Field merging detail:** The LLM fallback candidate only fills fields that are still null after API lookups. Existing file tags are also preserved — when writing, tags that are null in the candidate are skipped (thanks to the `candidate.genre` truthy fix already applied), so pre-existing genre on a file won't be deleted by the LLM fallback.

### What to change

**1. New LLM prompt** (`prompts.ts`)
- Build a new `buildTagCorrectionMessages` function that includes:
  - Folder name (raw)
  - Parent name (artist folder)
  - Existing per-track metadata (title, artist, album, track_number, genre)
  - The basic parser's hints
- LLM returns a structured JSON with:
  - `artist` — corrected artist name
  - `albumArtist` — corrected album artist (may differ from track artist for compilations)
  - `album` — corrected album name  
  - `year` — corrected year
  - `genre` — genre if confidently known
  - `tracks` — per-track corrections (title, artist)
  - `confidence` — how confident

**2. New LLM inference step** (`auto-tag.ts`)
- Replace the current `enhanceHints` method with a new `resolveTagsViaLLM` method
- Always runs when `llmApiKey` is configured (no ambiguity gate)
- Reads existing track metadata from `request.tracks`
- Calls LLM with the new prompt
- Returns `{ correctedRequest, fallbackCandidate }` where:
  - `correctedRequest` = updated LookupRequest with cleaned artist/album/year hints
  - `fallbackCandidate` = AlbumCandidate with all fields populated (including genre)

**3. Update `processAlbum` flow** (`auto-tag.ts`)
- Remove old Step 2 (enhanceHints)
- Insert new Step 2: resolveTagsViaLLM (after parsing, before lookups)
- Step 3-6: use `correctedRequest` for search queries  
- Step 7: if all candidates are still just the folder fallback, use `fallbackCandidate` instead (which has genre)

**4. Update `folderCandidate` logic** (`auto-tag.ts`)
- The folder fallback currently has `genre: null`. After LLM resolution, the LLM-provided fallback candidate already has genre.
- If LLM is not available (no API key), keep the old fallback behavior.

## Files to modify

| File | Change |
|---|---|
| `frontend/electron/handlers/prompts.ts` | Add `buildTagCorrectionMessages()` |
| `frontend/electron/handlers/auto-tag.ts` | Replace `enhanceHints` with `resolveTagsViaLLM`; update `processAlbum` flow |
| `frontend/test/handlers/auto-tag.test.ts` | Update tests for new behavior |
| `frontend/test/handlers/prompts.test.ts` | Add tests for new prompt builder |

## Verification

### Unit tests (existing + new)

- `hintsAreAmbiguous` tests — already passing (24 tests)
- New `buildTagCorrectionMessages` tests — verify prompt structure and payload shape
- New `resolveTagsViaLLM` integration test — mock LLM, verify corrected request + fallback candidate

### Scenario tests (auto-tag.test.ts)

| Scenario | Input | Expected |
|---|---|---|
| **蛋堡 case** (normal) | path=`/蛋堡/2009-Winter Sweet[flac]`, `llmApiKey=mocked` | LLM extracts artist=`蛋堡`, album=`Winter Sweet`, year=`2009`, genre is populated in fallback candidate, API search uses corrected hints |
| **CD subfolder edge case** | path=`/Artist/Album/CD1/tracks`, parent structure detected | CD subfolder detection still works, LLM gets parent folder name as album hint |
| **No LLM key** | `llmApiKey=undefined` | Falls back to folder fallback (current behavior, no regression) |
| **LLM fails** | LLM throws | Non-fatal, falls back to existing hints + folder fallback |

Run all: `npx vitest run test/handlers/`
