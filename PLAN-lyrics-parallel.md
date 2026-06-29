# Plan: Parallelize Lyrics Downloads in Auto-Tag ✅ DONE

## Context

Step 9 ("Applying tags") of `processAlbum` takes ~110s for a 12-track album with no local lyrics files. The bottleneck is sequential lyrics API downloads: 12 tracks × ~10s/request = ~120s. The lrclib.net API has inherently high latency (5-10s per request) with no explicit rate-limit headers.

The local LRC check (`readLocalLyrics`) already works correctly — albums with local `.lrc` files skip the API entirely. This fix targets albums that have **no local lyrics files**.

## Current Flow (sequential)

```
for each audioFile:
  1. readLocalLyrics(filePath)          // fast, sync
  2. if no local && download enabled:
     await fetchTrackLyrics(...)        // BLOCKS ~10s per track
  3. push to writeJobs
```

Tracks are processed one at a time. 12 tracks = 12 sequential 10s waits.

## Proposed Changes

### 1. Parallelize lyrics downloads in `applyCandidateTags()` (auto-tag.ts, ~line 1355)

Split the existing loop into two phases, using existing `mapConcurrent` from `electron/services/concurrency.ts`:

**Phase A:** Build writeJobs and identify tracks needing lyrics (fast, sync). Store direct reference to `mergedFields` so lyrics can be written back without index mapping.

**Phase B:** Download missing lyrics concurrently (2 at a time). Each downloaded result writes directly to the referenced `mergedFields` object.

```typescript
// Phase A: Build write jobs, check local lyrics
const lyricsNeeded: Array<{
  fields: WriteFields;
  trackName: string;
  artistName: string;
  album: string | null | undefined;
}> = [];
const writeJobs: Array<{ filePath: string; fields: WriteFields }> = [];

for (let i = 0; i < audioFiles.length; i++) {
  const filePath = audioFiles[i];
  const trackFields = i < trackFieldsList.length ? trackFieldsList[i] : {};
  const mergedFields: WriteFields = { ...albumFields, ...trackFields };

  const lyrics = readLocalLyrics(filePath);
  if (lyrics) {
    mergedFields.lyrics = lyrics;
  } else if (this.config.lyricsDownloadEnabled) {
    const trackName = mergedFields.title;
    const artistName = mergedFields.artist ?? albumFields.albumArtist ?? folderName;
    if (trackName && artistName) {
      lyricsNeeded.push({ fields: mergedFields, trackName, artistName, album: mergedFields.album });
    }
  }

  if (Object.keys(mergedFields).length === 0) continue;
  writeJobs.push({ filePath, fields: mergedFields });
}

// Phase B: Download missing lyrics concurrently (2 at a time)
const LYRICS_DOWNLOAD_CONCURRENCY = 2;
if (lyricsNeeded.length > 0) {
  await mapConcurrent(lyricsNeeded, LYRICS_DOWNLOAD_CONCURRENCY, async (entry) => {
    const downloaded = await this.fetchTrackLyrics(taskId, entry.trackName, entry.artistName, entry.album);
    if (downloaded) {
      entry.fields.lyrics = downloaded;
    }
  });
}
```

No index-based merge needed — each `lyricsNeeded` entry holds a direct reference to the `WriteFields` object that's already in `writeJobs`.

**Expected impact:** 120s → ~60s worst-case (2 concurrent × 10s × 6 batches).

### 2. Add per-request cache for duplicate tracks (auto-tag.ts)

Some albums have duplicate files (e.g. `爱来了.flac` + `爱来了.1.flac`) with the same artist/title. Cache lyrics results in a local `Map` keyed by normalized `artist:title` to avoid redundant API calls.

```typescript
const lyricsCache = new Map<string, string | null>();

// In Phase B mapConcurrent callback:
const cacheKey = `${entry.artistName}:${entry.trackName}`.toLowerCase();
if (lyricsCache.has(cacheKey)) {
  const cached = lyricsCache.get(cacheKey);
  if (cached) entry.fields.lyrics = cached;
  return;
}
const downloaded = await this.fetchTrackLyrics(...);
lyricsCache.set(cacheKey, downloaded);
if (downloaded) entry.fields.lyrics = downloaded;
```

Cache is local to `applyCandidateTags` — not persisted to disk.

### 3. No timeout change

Keep `AbortSignal.timeout(10_000)` as-is. The lrclib.net API legitimately takes 5-10s for some requests. Changing timeout is a separate behavior change that should be user-approved.

## Files to Modify

| File | Change |
|------|--------|
| `electron/handlers/auto-tag.ts` | Split loop into Phase A+B in `applyCandidateTags()`, add `mapConcurrent` import, add lyrics cache |
| `test/handlers/auto-tag.test.ts` | Add tests: local LRC skips API, missing LRC downloads concurrently |
| `test/handlers/lyrics.test.ts` | No changes needed (existing tests cover `readLocalLyrics` and `LyricsClient`) |

## Reuse

- **`mapConcurrent`** from `electron/services/concurrency.ts` — already imported in auto-tag.ts
- **`fetchTrackLyrics`** — unchanged, just called in parallel
- **`readLocalLyrics`** — unchanged, still called first in Phase A

## What This Does NOT Change

- Local LRC file checking (already correct)
- Tag writing concurrency (`TagWriteQueue` maxConcurrency=1)
- Lyrics API timeout (stays 10s)
- Discogs/MusicBrainz lookup timing
- Album-level task queue behavior

## Verification

1. `npx vitest run test/handlers/auto-tag.test.ts` — existing + new tests pass
2. `npx vitest run test/handlers/lyrics.test.ts` — existing tests pass
3. `npx vitest run` — full suite passes
4. Manual: Run auto-tag on `/Volumes/downloads/李圣杰/1999-冷咖啡[flac]/` (12 tracks, no local LRC) — step 9 should drop from ~110s to ~60s
5. Manual: Run auto-tag on `/Volumes/downloads/李翊君/7情6欲 绝对精彩13首 Best I/` (13 tracks, all local LRC) — should remain fast, zero API calls
