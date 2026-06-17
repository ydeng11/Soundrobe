# Plan: Artist-Scoped Release Browsing with Cached Normalized Index

## Context

The auto-tagger currently searches MusicBrainz and Discogs by **name-based queries** (`q=artist+album`), which fail for CJK/ non-Latin albums because:
1. MusicBrainz returns Traditional Chinese + ellipsis titles that `verifyAlbumName` can't match
2. Discogs generic search returns 0 relevant results for non-Latin queries
3. The file already has `DISCOGS_ARTIST_ID` (and sometimes `MUSICBRAINZ_ARTIST_ID`) but these IDs are only used for **direct release-ID lookups** — never to browse all releases and find the matching album by title.

The solution: resolve artist identity early, browse releases by artist ID (with pagination), normalize both sides (OpenCC S/T Chinese + punctuation stripping), score-match with CJK length guards, and cache results to avoid redundant API calls.

## Approach

### Priority flow in `performDirectIdLookups()`

```
1. musicbrainzAlbumId → lookupReleaseById()        ← authoritative, skip rest
2. discogsReleaseId  → lookupReleaseById()         ← authoritative, skip rest
3. MB artist ID → browse releases, score-match      ← new
4. Discogs artist ID → browse releases, score-match  ← upgrade existing
5. Name-based search (existing)                      ← unchanged fallback
```

### Core design decisions

**Cache**: Prevents redundant API calls for the same artist. Extends the existing `cache.db` SQLite database with two new structured tables:
  - `artist_release_cache(provider, artist_id, page, releases_json, fetched_at)` — stores lightweight release lists per artist
  - `release_detail_cache(provider, release_id, detail_json, fetched_at)` — stores full release details
  Both tables use structured columns in the same `better-sqlite3` database that `MatchCache` uses. An independent module `ReleaseCache` (in `handlers/cache.ts`) provides `getArtistReleaseList()`, `setArtistReleaseList()`, `getReleaseDetail()`, `setReleaseDetail()` methods. The MB and Discogs services call `ReleaseCache` directly — it's their cache layer, not a separate service.

**Normalization helper**: Single shared function in `candidates.ts` — NFKC → lowercase → strip punctuation → OpenCC S/T Chinese. Replaces the regexes duplicated in `candidates.ts`, `DiscogsService.ts`, and `DiscogsService`'s static method.

**Matching**: Score-based, not binary. Exact normalized title=100, remote-contains-local=85, local-contains-remote=70. Bonus +10 for: year match, artist credit match, track count within 1. Reject if normalized CJK length <4 or Latin token <3. Threshold ≥75.

**Pagination**: 3-5 pages per artist, 100 items per page. Stop early on exact match.

**Tracklist detail**: Only fetched for top candidate(s), not all releases. MB uses `inc=recordings` for the winner only.

## Files to modify

| File | Change |
|---|---|
| `frontend/electron/handlers/candidates.ts` | Extract shared `normalizeForMatch()` used by both services |
| `frontend/electron/handlers/musicbrainz.ts` | Add `lookupArtistReleaseByAlbum(mbArtistId, albumHint)` using `/release?artist={id}` |
| `frontend/electron/handlers/discogs.ts` | Upgrade `lookupArtistReleaseByAlbum()` with pagination, OpenCC normalization, scoring matcher, CJK guard |
| `frontend/electron/handlers/cache.ts` | Add methods: `getArtistReleaseList()`, `setArtistReleaseList()`, `getReleaseDetail()`, `setReleaseDetail()` — keyed by `{provider, artistId, page}` and `{releaseId}` |
| `frontend/electron/handlers/auto-tag.ts` | Update `performDirectIdLookups()`: resolve artist identity early, add MB and Discogs artist browse steps. Also update `searchVariants()` in steps 5/6 to use artist-scoped browse when artist ID is available. |
| `frontend/electron/services/ArtistIdentityResolver.ts` | No change needed — already resolves artist identity with caching |
| `frontend/electron/services/DiscogsService.ts` | Align `getArtistReleaseByTitle()` to use shared `normalizeForMatch()` from `candidates.ts` |

## Reuse

- `findArtistIdentity()` in `ArtistIdentityResolver.ts` — already cached, use directly
- `DiscogsService.normalizeForMatch()` — replace with shared version in `candidates.ts`
- `CacheMatch` class in `cache.ts` — extend with artist release list / release detail cache tables
- `DiscogsClient.lookupArtistReleaseByAlbum()` in `discogs.ts` — upgrade in place
- `MusicBrainzClient.searchAlbum()` → reuse `loadTracks()` and `parseTracksFromMedia()` for the new method

## Steps

### Step 1: Extract shared `normalizeForMatch()` in `candidates.ts`

- Move the existing `normalizeLookupText()` + `UNICODE_PUNCT_RE` + add `ASCII_PUNCTUATION_RE` usage into a new exported `normalizeForMatch(text: string): Promise<string>`
- Add OpenCC Simplified Chinese conversion inside (async)
- Keep `normalizeLookupText()` calling through to `normalizeForMatch()` for backward compat
- Export `normalizeForMatch` so `discogs.ts`, `musicbrainz.ts`, `DiscogsService.ts` can import it

### Step 2: Add `ReleaseCache` to `cache.ts`

Extends the existing `cache.db` SQLite database. `ReleaseCache` is an independent class that works alongside `MatchCache` but is called directly by MB and Discogs services.

**Schema** (two new tables in `cache.db`):
```sql
CREATE TABLE IF NOT EXISTS artist_release_cache (
  provider TEXT NOT NULL,   -- 'musicbrainz' | 'discogs'
  artist_id TEXT NOT NULL,
  page INTEGER NOT NULL DEFAULT 1,
  releases_json TEXT NOT NULL,        -- JSON array of ReleaseMeta[]
  fetched_at TEXT NOT NULL,           -- ISO 8601
  PRIMARY KEY (provider, artist_id, page)
);

CREATE TABLE IF NOT EXISTS release_detail_cache (
  provider TEXT NOT NULL,
  release_id TEXT NOT NULL,
  detail_json TEXT NOT NULL,          -- JSON of cached release detail
  fetched_at TEXT NOT NULL,
  PRIMARY KEY (provider, release_id)
);
```

**`ReleaseMeta` type** (structured per row):
```ts
interface ReleaseMeta {
  id: string;           // Discogs release/master ID or MusicBrainz release ID
  title: string;
  year: number | null;
  type: string | null;  // 'master' | 'release' | null
  artistName: string | null;
}
```

**`ReleaseCache` class**:
```ts
class ReleaseCache {
  constructor(cachePath: string)     // same cache.db as MatchCache
  close(): void

  // Artist release list
  getArtistReleaseList(provider: string, artistId: string, page: number): ReleaseMeta[] | null
  setArtistReleaseList(provider: string, artistId: string, page: number, releases: ReleaseMeta[]): void

  // Full release detail
  getReleaseDetail(provider: string, releaseId: string): AlbumCandidate | null
  setReleaseDetail(provider: string, releaseId: string, candidate: AlbumCandidate): void

  // Prune entries older than N hours (called on init, or explicitly)
  prune(maxAgeHours: number): void
}
```

**Usage pattern in MB/Discogs services**: Before making any API call, check `ReleaseCache.getArtistReleaseList()`. If cache hit, use cached release list directly. If miss, fetch from API, transform response to `ReleaseMeta[]`, store via `ReleaseCache.setArtistReleaseList()`, then use the returned data. Same pattern for release details (already hit `lookupReleaseById()` which could also benefit from cached release details).

**Note**: The existing `MatchCache` (`lookup_cache` table) caches **album search results** (query → AlbumCandidate[]). The `ReleaseCache` caches **raw API data** (artist releases and release details at a lower level). They are complementary — `ReleaseCache` prevents the API call itself, `MatchCache` prevents re-running the full lookup pipeline.

### Step 3: Add `MusicBrainzClient.lookupArtistReleaseByAlbum()` in `musicbrainz.ts`

- Signature: `async lookupArtistReleaseByAlbum(artistId: string, albumHint: string, options?: { cache }): Promise<AlbumCandidate | null>`
- Endpoint: `GET /ws/2/release?artist={artistId}&limit=100&offset={page*100}&fmt=json&inc=artist-credits`
- Paginate: up to 3 pages, stop early on `release-count` exhaustion
- Normalize: call `normalizeForMatch()` on both `albumHint` and each release title
- Score: exact=100, remote-contains-local=85, local-contains-remote=70
- Bonus: year match +10, artist credit match +10, track count match +10
- Min length guard: reject CJK containment <4 chars
- Threshold: ≥75
- Cache: check `ReleaseCache.getArtistReleaseList('musicbrainz', artistId, page)` before each API call. On miss: fetch from API, transform MB release list to `ReleaseMeta[]`, store via `setArtistReleaseList()`.
- On winner: call `lookupReleaseById()`, cache result via `ReleaseCache.setReleaseDetail()`.

### Step 4: Upgrade `DiscogsClient.lookupArtistReleaseByAlbum()` in `discogs.ts`

- Same signature as MB version
- Endpoint: `GET /artists/{artistId}/releases?per_page=100&page={N}&sort=year&sort_order=desc`
- Paginate: up to 3 pages
- Normalize: use `normalizeForMatch()` from `candidates.ts`
- Score: same scoring as MB version
- Min length guard: same as MB version
- Cache: check `ReleaseCache.getArtistReleaseList('discogs', artistId, page)` before each API call. On miss: fetch from API, transform Discogs releases to `ReleaseMeta[]`, store via `setArtistReleaseList()`.
- On winner: call `lookupReleaseById()`, cache result via `ReleaseCache.setReleaseDetail()`.

### Step 5: Update `performDirectIdLookups()` in `auto-tag.ts`

Current flow:
```
if musicbrainzAlbumId → lookupReleaseById()
if musicbrainzArtistId only → skip (comment says "falling back to name search")
if discogsReleaseId → lookupReleaseById()
if discogsArtistId + albumHint → lookupArtistReleaseByAlbum()
```

New flow in the same method:
```
→ Resolve artist identity early (call findArtistIdentity if IDs missing)
→ if musicbrainzAlbumId → lookupReleaseById()           ← authoritative
→ if discogsReleaseId → lookupReleaseById()             ← authoritative
→ if musicbrainzArtistId + albumHint → MB artist browse  ← new
→ if discogsArtistId + albumHint → Discogs artist browse  ← upgraded
→ return results[]
```

Also update steps 5/6 (`searchVariants`) to pass artist IDs when available:
- If `musicbrainzArtistId` is known after identity resolution, skip name search and use artist browse directly
- Same for Discogs

### Step 6: Update `searchVariants()` to use artist-scoped browse when feasible

- Currently: `client.searchAlbum(artist, album)` — generic name-based
- After step 5: when artist IDs are available, call `client.lookupArtistReleaseByAlbum(artistId, album)` instead of name search
- Needs: pass the resolved IDs through the pipeline

### Step 7: Align `DiscogsService.getArtistReleaseByTitle()` with shared normalizer

- Import `normalizeForMatch` from `candidates.ts`
- Remove static `normalizeForMatch()` and `UNICODE_PUNCT_RE` from `DiscogsService.ts`
- Keep the pagination logic but use shared normalizer

### Step 8: Tests

- `candidates.test.ts`: test `normalizeForMatch()` with CJK, fullwidth, ellipsis cases
- `musicbrainz.test.ts`: test `lookupArtistReleaseByAlbum()` with mock
- `discogs.test.ts`: test upgraded `lookupArtistReleaseByAlbum()` with mock
- `cache.test.ts`: test artist release list cache round-trip
- `auto-tag.test.ts`: test that `performDirectIdLookups()` calls artist browse when IDs present

## Verification

1. Run full test suite: `npx vitest run` — all 1043+ tests must pass
2. Test with CJK album that has `DISCOGS_ARTIST_ID` but no `DISCOGS_RELEASE_ID` (e.g. 郭富城/到底有谁能够告诉我):
   - Verify `performDirectIdLookups()` calls Discogs artist browse
   - Verify normalization matches Simplified Chinese hint to Traditional Chinese title
   - Verify candidate returned with `discogsReleaseId` populated
3. Test with English album that has `MUSICBRAINZ_ARTIST_ID` but no `MUSICBRAINZ_ALBUM_ID`
4. Test cache: second lookup for same artist should skip API call (verify via `ReleaseCache.getArtistReleaseList()` returning cached data before any fetch)
5. Test cache TTL: verify `prune()` removes entries older than threshold
6. Test structure: verify `artist_release_cache` and `release_detail_cache` tables round-trip ReleaseMeta[] correctly
