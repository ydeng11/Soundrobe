# Improve DatasetReader.queryAlbum() Hit Rate

## Context

The local SQLite dataset at `~/.auto-tagger/dataset-index.sqlite` (157 GB) has 7 albums for 蛋堡 from MusicBrainz alone. But the current `DatasetReader.queryAlbum()` only finds matches when **both** the normalized artist hint **and** the normalized album hint exactly match the database columns. This misses many valid matches due to:

- Extra tokens in folder names (year prefixes, format suffixes, parenthetical notes)
- Artist name variations (parenthetical aliases like "蛋堡 (Soft Lipa)")
- Album subtitle / extra-info patterns not handled by the word-dropping prefix fallback

The year-prefix case is the most common miss we want to fix. Artist parenthetical stripping prevents exact-match failures on common naming patterns. Artist-only fallback recovers cases where the album hint is completely wrong (e.g., user typed a subtitle), bounded by `maxCandidates` to avoid flooding the pipeline with noise.

## Current Flow

1. `normalizeLookupText(artistHint)` + `normalizeLookupText(albumHint)`
2. `buildLookupVariantPairs()` — SC/TC variants + original
3. `queryLookupTable()` — exact `WHERE normalized_artist = ? AND normalized_album = ?`
4. If no results: `progressivePrefixFallback()` — drop words from end of album, try prefix LIKE `prefix%`
5. If still no results: return empty

## Root Causes of Misses

| Scenario | Example hint | Normalized | DB has | Current result |
|---|---|---|---|---|
| Year prefix | "2009 Winter Sweet" | `2009 winter sweet` | `winter sweet` | ❌ prefix fallback: `2009 winter` > `2009` |
| Artist extra parens | "蛋堡 (Soft Lipa)" | `蛋堡 (soft lipa)` | `蛋堡` | ❌ artist mismatch |
| Format suffix residual | "Winter Sweet[flac]" | `winter sweet flac` | `winter sweet` | ✅ (fix applied, but if 3+ words, prefix drops 1 at a time) |
| Extra words in middle | "Winter Sweet Deluxe Edition" | `winter sweet deluxe edition` | `winter sweet` | ❌ prefix drops from end: `winter sweet deluxe` > `winter sweet` ✅ actually works |

## Approach

All changes in `frontend/electron/handlers/dataset.ts`. Each new fallback is tried only if previous steps returned nothing (preserving the existing priority).

### 1. Artist hint cleanup (pre-normalization)
Strip parenthetical suffixes from artist hint before normalization:
- `"蛋堡 (Soft Lipa)"` → `"蛋堡"`
- `"Various Artists (VA)"` → `"Various Artists"`

This helps the exact match succeed more often.

### 2. Year-prefix album fallback
If the album hint starts with a 4-digit year (e.g., "2009 Winter Sweet"), extract it and try matching the remaining text against the DB. Also try using the year as a filter if provided.

### 3. Artist-only fallback (bounded)
If all artist+album queries return empty, try finding albums by artist alone:
- Query `WHERE normalized_artist = ?` and return the top N candidates (capped by `maxCandidates`)
- This catches cases where the album hint is completely wrong but the artist is correct
- **Multiplicity handled**: `maxCandidates` (default 5) bounds the result list, giving the pipeline a handful of candidates to rank

### 4. Cross-service deduplication
Currently the same album can appear once per service (MB, Spotify, Tidal, Deezer) with different `album_id`s. The existing `seenIds` set only catches same-service duplicates. Add a deduplication pass that groups candidates by normalized `(artist, album)` pair and keeps only the best source per group:
- Priority: MusicBrainz → Spotify → Tidal → Deezer (same order as the SQL `CASE`)
- If year is available in the hint, also prefer candidates whose year matches the hint year

**No new indexes needed** — there is no separate step (no new queries).

## Deduplication impact on hit-rate improvements
| New step | Could produce duplicates? | Dedup handles it? |
|---|---|---|
| Artist parenthetical strip | Same artist matches same album from multiple services → duplicates | ✅ Dedup groups by (artist, album) name, keeps best source |
| Year-prefix fallback | Matched album could exist in MB + Spotify + Tidal + Deezer → up to 4 copies | ✅ Dedup collapses to 1 |
| Artist-only fallback | Artist has albums from multiple services → many cross-service copies | ✅ Dedup keeps 1 per unique (artist, album) pair |

## Files to modify

- `frontend/electron/handlers/dataset.ts` — all matching logic changes
- `frontend/test/handlers/dataset.test.ts` — new test scenarios

## Reuse

- `normalizeLookupText()` from `candidates.ts` — already used
- Artist cleanup can use inline regex (no existing utility for stripping parens)

## Steps

- [ ] Step 1: Add artist hint cleanup (strip parenthetical suffixes pre-normalization)
- [ ] Step 2: Add year-prefix album fallback (extract leading year, retry without it)
- [ ] Step 3: Add cross-service deduplication (group by normalized (artist, album), keep best source)
- [ ] Step 4: Add artist-only fallback (return albums by artist when album hint fails)
- [ ] Step 5: Add test cases for each new fallback
- [ ] Step 6: Run full test suite and verify no regressions

## Verification

1. `npm test` — all existing tests pass
2. New tests in `dataset.test.ts` cover:
   - Year prefix album ("2009 Winter Sweet" → "winter sweet")
   - Artist parenthetical ("蛋堡 (Soft Lipa)" → "蛋堡")
   - Cross-service dedup: same album from 2 services → 1 result
   - Artist-only fallback (wrong album hint → artist match returns up to 5 albums, deduped)
   - No regression on existing exact matches, Chinese SC/TC variants, prefix fallback
3. Manual verification with the real 157 GB dataset for 蛋堡
