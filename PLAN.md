# Plan: Trust Remote Track Artist on Match

## Context

The artist replacement logic in `RemoteTrackMatcher.ts` (lines 867–925) has grown complex with three overlapping conditions (`remoteEnrichesLocal`, `localArtistIsUnusual`, `remoteMatchesHint`). This led to bugs (collaborative tracks like `品冠 vs 光良` not matching the hint `"品冠"`). The root cause: when a track title matches a remote release, MB/Discogs per-track artist-credits are authoritative — no need to second-guess them.

The `processAlbum` method is 262 lines with 9 steps, including complex candidate merging. The goal is to simplify to a clear flow where matched release/track data is ground truth.

## Proposed Simplified Flow

```
1. artistFinder    → resolve MB/Discogs artist IDs (already exists)
2. fetchReleases   → fetch all releases for artist from MB + Discogs, store in memory
3. processAlbum    → for each album:
   a. find matching release (by title)
   b. for each track:
      - title matches  → use remote artist/title as ground truth
      - no match       → use LLM fallback
```

### How this differs from current flow

| Current (9 steps, 262 lines) | Proposed (5 steps, <200 lines) |
|---|---|
| Parse hints → resolve IDs → cache → direct ID lookup → search MB → search Discogs → LLM → filter → protect → merge → apply | Resolve IDs → fetch all releases → for each album: match release → match tracks (remote or LLM) → apply |
| Multiple candidates merged with priority logic | Single release selected per album |
| Complex artist replacement (3 conditions) | Simple: match = trust remote |
| `protectCandidateTrackFieldsForAutoApply` with `RemoteTrackMatcher` | Simplified matching: title match → use remote |
| `fillTrackGapsByPosition` to merge candidates | No merging needed |
| `trackArtistCanBeEnriched` + `enrichArtistWithRemote` | Removed — remote is ground truth for matched tracks |

### LLM fallback preserved

- LLM is called per-album as it is now (step 7 in current flow)
- For tracks with no title match against MB/Discogs, LLM per-track data is used
- For albums not found in MB/Discogs at all, LLM provides album-level + per-track data

## Files to modify

1. **`frontend/electron/services/RemoteTrackMatcher.ts`** — simplify artist logic to just trust remote on match
2. **`frontend/electron/handlers/auto-tag.ts`** — simplify `processAlbum` flow
3. **`frontend/test/services/RemoteTrackMatcher.test.ts`** — update/add tests

## Steps

### Step 1: Simplify artist logic in `RemoteTrackMatcher.ts`

Replace lines 867–925 (the `localArtistBlank / remoteEnrichesLocal / localArtistIsUnusual` block) with:

```ts
// ── Remote artist/artists ─────────────────────────────────
// When a track title matched, remote per-track artist-credits are
// authoritative (MusicBrainz/Discogs are the ground truth).
const remoteArtist = remoteTrack.artist;
if (remoteArtist) {
  result.artist = remoteArtist;
  result.artists = remoteTrack.artists.length > 0
    ? [...remoteTrack.artists]
    : [remoteArtist];
}
```

This removes: `localArtistIsUnusual`, `remoteMatchesHint`, `localLooksCorrupted`, `remoteEnrichesLocal`, `remotePrimaryMatch`, `localArtistBlank`.

### Step 2: Remove `enrichArtistWithRemote` and `trackArtistCanBeEnriched`

In `auto-tag.ts`:
- Delete `enrichArtistWithRemote` (lines 301–332)
- Delete `trackArtistCanBeEnriched` (lines 282–293)
- Simplify `fillTrackGapsByPosition`: for unmatched tracks, if remote has artist data and local is blank, fill it. Otherwise leave local as-is.

```ts
function fillTrackGapsByPosition(
  target: TrackCandidate[],
  source: TrackCandidate[],
  sourceName: AlbumCandidate["source"],
): void {
  for (let i = 0; i < Math.min(target.length, source.length); i++) {
    const targetTrack = target[i];
    const sourceTrack = source[i];
    if (API_TITLE_CLEANUP_SOURCES.has(sourceName) && targetTrack.title && sourceTrack.title) {
      const titleReplacement = replacementTitleForPollutedTitle(
        targetTrack.title, sourceTrack.title);
      if (titleReplacement) {
        targetTrack.title = titleReplacement;
      } else if (isPlaceholderTitle(targetTrack.title)) {
        targetTrack.title = sourceTrack.title;
      }
    }
    // Only fill blank local artists — matched tracks already have correct artist
    if (sourceTrack.artist && (!targetTrack.artist || targetTrack.artist.trim() === "")) {
      targetTrack.artist = sourceTrack.artist;
      targetTrack.artists = sourceTrack.artists.length > 0 ? sourceTrack.artists : [sourceTrack.artist];
    }
    targetTrack.musicbrainzTrackId ??= sourceTrack.musicbrainzTrackId;
    targetTrack.length ??= sourceTrack.length;
    targetTrack.genre ??= sourceTrack.genre;
  }
}
```

### Step 3: Remove `protectCandidateTrackFieldsForAutoApply`

Since RemoteTrackMatcher no longer has complex artist logic, this function can be simplified to just the title matching + writing matched remote data. The trust is implicit — all matched tracks use remote data.

### Step 4: Update tests

- **Remove** tests for `localArtistIsUnusual` / `remoteMatchesHint` / `remoteEnrichesLocal` behavior
- **Add** tests:
  - Matched MB track replaces `[momishi.com]` with remote artist
  - Matched MB track replaces non-blank local artist with remote duet artist
  - Unmatched track preserves local artist
  - SC/TC title match triggers remote artist

### Step 5: Build and verify

1. `npx vitest run` — all tests pass
2. `npm run build`
3. Clear `lookup_cache` for affected albums
4. Re-run auto-tag on `品冠/2005-后来的我[flac]/` — verify:
   - `品冠_光良-身边.flac` → `ARTIST=品冠 vs 光良` (was `[momishi.com]`)
   - `品冠-又一年又三年.flac` → `ARTIST=品冠` (unchanged)
   - `品冠_梁静茹-明明很爱你.flac` → `ARTIST=品冠 vs 梁靜茹` (was `[momishi.com]`)
