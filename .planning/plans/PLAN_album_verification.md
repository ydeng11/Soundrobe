# Plan: Deterministic Album Name Verification

**Goal:** Regardless of whether the album name hint comes from folder names or
existing file tags, verify it against authoritative databases (local dataset →
MusicBrainz → Discogs), capture MBIDs, and produce a confidence-scored
verdict on correctness.

---

## What Must Be TRUE

1. **Existing tags are used as hints** — not just folder names. If a file has
   `album="2001"` in its ID3/Vorbis tags, that takes priority over the folder
   name as a lookup hint.
2. **Local dataset checked first** — fastest, offline, already has MBIDs from
   MusicBrainz rows.
3. **Beets/MusicBrainz checked second** — canonical MBIDs, but rate-limited
   and requires network.
4. **Discogs checked third** — broader catalog (19M+ releases vs. MusicBrainz's
   ~3M), free API with no auth required for search + release endpoints.
5. **MBIDs captured from any source** — dataset (from
   musicbrainz-sourced CSV rows), beets (from `tag_album` results), Discogs
   (via master/release cross-reference if available).
6. **Album name verified against actual database results** — not just "did
   the search return something", but "does the best match agree with our
   candidate name."
7. **Graceful degradation** — if dataset index doesn't exist, skip it; if
   beets isn't installed, skip it; if Discogs is rate-limited, skip it.

---

## Architecture Changes

### 1. Enrich `LookupRequest` with existing tag data

**File:** `src/soundrobe/integrations/fallback.py`

Currently `parse_album_path()` only reads folder names. Add a new function
that also reads existing tags from the first audio file and uses them as
hints when folder names are absent or ambiguous:

```python
def parse_album_with_tags(path: Path) -> LookupRequest:
    """Build request using both folder name AND existing file tags."""
    # Existing folder-based request
    request = parse_album_path(path)

    # Read first audio file's tags as additional hints
    tag_album = _read_album_from_tags(path)

    # Prefer tags over folder names (tags are more intentional)
    artist_hint = tag_album.get("artist") or request.artist_hint
    album_hint = tag_album.get("album") or request.album_hint

    tracks = _track_hints_from_path(path)

    return LookupRequest(
        path=path,
        artist_hint=artist_hint,
        album_hint=album_hint,
        tracks=tracks,
    )
```

**Priority rule:** existing tag values > cleaned folder names. A human (or
previous tool) put those tags there intentionally. The folder name is just
file-system organization.

### 2. Add `LookupSource.DISCOGS` to the enum

**File:** `src/soundrobe/integrations/candidates.py`

```python
class LookupSource(Enum):
    BEETS = "beets"
    DATASET = "dataset"
    DISCOGS = "discogs"    # NEW
    FOLDER = "folder"
```

### 3. Add `DiscogsClient`

**New file:** `src/soundrobe/integrations/discogs_client.py`

Thin wrapper around the free Discogs API:

- `search(artist, album)` → search endpoint, returns release IDs + titles
- `get_release(release_id)` → full release with tracklist, artists, year, genres, images
- `get_master(master_id)` → master release with consolidated tracklist

**Key design decisions:**
- Use `httpx` (already a dependency) — no new library needed
- No authentication required for search + release endpoints (confirmed)
- Rate limit: 25 req/min unauthenticated. Implement `RateLimiter` with 2.5s interval
- Transform results into `AlbumCandidate` + `TrackCandidate` (same models as beets)
- Source: `LookupSource.DISCOGS`
- Discogs does NOT have MBIDs natively. Leave `musicbrainz_albumid=None` on
  Discogs candidates. MBIDs come from dataset and beets sources.

**Rate limiter reuse:** The existing `RateLimiter` in `beets_client.py` can
be extracted to `src/soundrobe/integrations/rate_limiter.py` and shared.
Or, `DiscogsClient` can instantiate its own with a different interval.

### 4. Extend `LookupService` cascade

**File:** `src/soundrobe/integrations/lookup.py`

New cascade order:

```
cache → dataset → beets → discogs → fallback
```

```python
def lookup_album(self, path: Path) -> list[AlbumCandidate]:
    request = self.request_from_path(path)  # now uses tags + folders
    cached = self.cache.get(request)
    if cached is not None:
        return cached

    # 1. Dataset (local, fast, has MBIDs)
    candidates = self._lookup_dataset(request)

    # 2. Beets/MusicBrainz (authoritative MBIDs, rate-limited)
    if not candidates and self.settings.remote_lookup_enabled:
        candidates = self._lookup_beets(request)

    # 3. Discogs (broader catalog, free)
    if not candidates and self.settings.discogs_enabled:
        candidates = self._lookup_discogs(request)

    # 4. Folder fallback (last resort)
    if not candidates:
        candidates = [candidate_from_folder(request)]

    self.cache.set(request, candidates)
    return candidates
```

### 5. New config settings

**File:** `src/soundrobe/config/settings.py`

```python
# New settings
discogs_enabled: bool = True           # Enable/disable Discogs lookup
discogs_rate_limit: float = 2.5        # Seconds between Discogs API calls
discogs_max_candidates: int = 3        # Max results to return
discogs_token: str | None = None       # Optional: personal access token for 60 req/min
```

### 6. Add `AlbumCandidate.verification_status` field

**File:** `src/soundrobe/integrations/candidates.py`

To support the "verify if the album name is correct" goal, add a field that
indicates whether the candidate agrees with the hint:

```python
@dataclass(frozen=True)
class AlbumCandidate:
    # ... existing fields ...
    verification: str | None = None  # "match", "close", "mismatch", or None (not checked)
```

A helper function compares the hint album name to the candidate album name:

```python
def verify_album_name(hint: str | None, candidate: AlbumCandidate) -> str:
    """Compare hint to candidate album name."""
    if not hint or not candidate.album:
        return "match"  # can't verify, assume match
    hint_norm = normalize_lookup_text(hint)
    cand_norm = normalize_lookup_text(candidate.album)
    if hint_norm == cand_norm:
        return "match"
    if hint_norm in cand_norm or cand_norm in hint_norm:
        return "close"
    return "mismatch"
```

---

## Implementation Phases

### Phase 1: Tag-aware LookupRequest (smallest change, highest impact)

1. Add `parse_album_with_tags()` to `fallback.py`
2. Add `_read_album_from_tags()` helper that reads the first audio file
3. Update `LookupService.request_from_path()` to use the new function
4. Write tests: tag hints take priority over folder hints

**Validation:** Running `auto-tag tag /path/to/album --dry-run` shows
existing tags as lookup hints in the candidate table.

### Phase 2: Discogs Client

1. Create `src/soundrobe/integrations/discogs_client.py`
2. Implement `DiscogsClient.search(artist, album) → list[dict]`
3. Implement `DiscogsClient.get_release(release_id) → AlbumCandidate`
4. Transform to `AlbumCandidate` with `LookupSource.DISCOGS`
5. Add config settings for Discogs
6. Write unit tests with `respx` or `pytest-httpx` for mocked API calls
7. Write integration test with real API (marked `network`)

**Validation:** `DiscogsClient.search("Dr. Dre", "2001")` returns the correct
release. `DiscogsClient.get_release(3201905)` returns full tracklist.

### Phase 3: Extended Lookup Cascade

1. Add `_lookup_discogs()` to `LookupService`
2. Wire into cascade after beets, before folder fallback
3. Add `discogs_enabled` setting check
4. Update cache key to include Discogs results
5. Update CLI preview to show Discogs as a source

**Validation:** On a library without MBIDs, `--dry-run` shows Discogs
candidates when beets returns nothing.

### Phase 4: Verification Status

1. Add `verification` field to `AlbumCandidate`
2. Add `verify_album_name()` helper in `candidates.py`
3. Compute verification status for all candidates in `LookupService`
4. Display verification status in CLI preview table
5. In YOLO mode: if verification is "mismatch", downgrade confidence

**Validation:** A folder named "2001" containing Dr. Dre tracks shows
`verification=match` from Discogs. A folder named "Wrong Name" shows
`verification=mismatch` with the suggested correct name.

---

## What This Does NOT Do

- **Does not write corrected tags in YOLO mode** — that's a separate
  workflow change (currently YOLO only re-writes existing tags). This plan
  focuses on getting the right answer, not applying it.
- **Does not add AcoustID fingerprinting** — too heavy for this tool's scope.
- **Does not replace the LLM selection step** — LLM still picks between
  multiple candidates when there's ambiguity.
- **Does not authenticate with Discogs** — the free unauthenticated tier
  (25 req/min) is sufficient. Token support is config-only, not in scope.

---

## Estimated Impact

| Scenario | Before | After |
|----------|--------|-------|
| Folder `2001` with tags `album=2001` | Lookup uses folder name only | Lookup uses existing tags → verified via Discogs |
| Obscure Chinese album not in MusicBrainz | Falls to folder fallback | Discogs may have it (19M releases) |
| Album with conflicting folder vs. tags | Folder wins silently | Tags win, mismatch flagged |
| Digit-only album name `2001` | Text search works but no verification | Verified against Discogs release |
| Fully offline | Dataset only (if built) | Same, no change |

---

## Files Changed

| File | Change |
|------|--------|
| `src/soundrobe/integrations/fallback.py` | Add `parse_album_with_tags()`, `_read_album_from_tags()` |
| `src/soundrobe/integrations/candidates.py` | Add `LookupSource.DISCOGS`, `verification` field, `verify_album_name()` |
| `src/soundrobe/integrations/discogs_client.py` | **NEW** — DiscogsClient with search + release lookup |
| `src/soundrobe/integrations/lookup.py` | Add `_lookup_discogs()`, update cascade, use tag-aware request |
| `src/soundrobe/config/settings.py` | Add `discogs_enabled`, `discogs_rate_limit`, etc. |
| `tests/test_discogs_client.py` | **NEW** — Discogs client tests (mocked + live) |
| `tests/test_lookup.py` | Add Discogs cascade tests |
| `tests/test_fallback.py` | Add tag-aware parsing tests |
