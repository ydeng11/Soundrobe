# Plan: Per-track artist credits from MusicBrainz

## Problem

The auto-tagger writes `ARTIST` from the **release-level** artist credit, ignoring per-track credits. For tracks like 林俊杰 - 加油! (feat. MC HotDog), the file gets `ARTIST=林俊傑` but MusicBrainz has `林俊傑 feat. MC HotDog` on both the release-track and recording levels.

### Root cause

`parseTracksFromMedia()` receives only the release-level `artistName` and applies it to every track:

```typescript
// musicbrainz.ts, parseTracksFromMedia()
artist: artistName,        // ← always release-level
artists: artistName ? [artistName] : [],  // ← always release-level
```

Additionally, `loadTracks()` uses `inc=recordings` (no `artist-credits`), so even the API response doesn't include per-track credits in that code path. Only `fetchReleaseById()` uses `inc=recordings+artist-credits`.

### Evidence

MusicBrainz release `b7b08c20-ec63-4b2f-99e9-ff367655b63e` ("100天") with `inc=artist-credits+recordings`:

| Level | Track 3 (加油!) artist credit |
|---|---|
| Release-level | 林俊傑 |
| Release-track | 林俊傑 feat. MC HotDog |
| Recording | 林俊傑 feat. MC HotDog |

The API already returns per-track credits — the code just doesn't use them.

## Scope

### In scope
1. `parseTracksFromMedia()` — extract per-track `artist-credit` when present, fall back to recording-level, then release-level
2. `loadTracks()` — change `inc=recordings` → `inc=recordings+artist-credits` so the API returns per-track credits
3. `fetchReleaseById()` — already uses `inc=recordings+artist-credits`, no change needed
4. `TrackCandidate` type — no change needed (already has `artist` + `artists` fields)
5. Tests — add test cases for tracks with featured artists
6. Cache staleness — bump/namespace the MusicBrainz release-detail cache key to force re-fetch

### Out of scope
- Album-level artist credit (`ALBUMARTIST`) — stays as release-level, no change
- Writer/tag service — already correctly writes whatever `artist`/`artists` the candidate has
- Discogs artist credits — separate concern
- LLM fallback — separate concern
- Changing the `MUSICBRAINZ_ARTISTID` tag — stays as primary (first) artist ID

## Design

### Artist credit extraction helpers

Add two helper functions near `parseTracksFromMedia()`:

**`formatArtistCredit(credit: Array<Record<string, unknown>>): string`**
- Joins all `name` fields with their `joinphrase` (e.g., `["林俊傑", " feat. ", "MC HotDog"]` → `"林俊傑 feat. MC HotDog"`)
- Returns the full display string

**`artistNamesFromCredit(credit: Array<Record<string, unknown>>): string[]`**
- Returns array of individual `name` values for `TrackCandidate.artists`

### Changes to `parseTracksFromMedia()`

```
for each track in medium.tracks:
  1. Check if track has "artist-credit" array → use it
  2. Else check if recording has "artist-credit" array → use it
  3. Else fall back to release-level artistName (current behavior)
  4. Set artist = formatArtistCredit(resolvedCredit)
  5. Set artists[] = artistNamesFromCredit(resolvedCredit)
```

**Precedence: track credit → recording credit → release-level. Do not skip the recording-level fallback.**

### Changes to `loadTracks()`

Change the API URL from:
```
/release/{id}?fmt=json&inc=recordings
```
to:
```
/release/{id}?fmt=json&inc=recordings+artist-credits
```

### Cache staleness

Existing cached `AlbumCandidate` objects may preserve old release-level track artists. Bump the MusicBrainz release-detail cache key/version to force re-fetch after this change.

### Tag field mapping

| Tag | Source | Value (track 3) |
|---|---|---|
| `ARTIST` | `TrackCandidate.artist` | `林俊傑 feat. MC HotDog` |
| `ARTISTS` | `TrackCandidate.artists` | `["林俊傑", "MC HotDog"]` |
| `ALBUMARTIST` | `AlbumCandidate.albumArtist` | `林俊傑` (unchanged) |
| `MUSICBRAINZ_ARTISTID` | `AlbumCandidate.musicbrainzArtistId` | `e5d8c705-...` (primary artist, unchanged) |

## Risk assessment

### Low risk
- `loadTracks()` URL change — adding `+artist-credits` to an existing `inc` param is safe, MusicBrainz supports it
- `parseTracksFromMedia()` fallback — existing behavior preserved when no per-track credit exists

### Medium risk
- **Player compatibility**: Some players may split `ARTIST = "林俊傑 feat. MC HotDog"` into two artists, causing duplicate library entries if `ALBUMARTIST` is missing. Mitigated by keeping `ALBUMARTIST` as release-level.
- **Cache staleness**: Existing cached releases don't have per-track credits. After this change, re-fetching is needed. Mitigated by bumping cache key/version.

### No risk
- `TrackCandidate` type already supports `artist` + `artists` — no schema change
- Writer already writes whatever it receives — no change needed

### Follow-up (out of scope)
- `TrackCandidate` currently lacks per-track artist MBIDs. If `ARTIST` becomes per-track but `MUSICBRAINZ_ARTISTID` remains album-primary only, that is still imperfect MusicBrainz fidelity. Note as a follow-up.

## Test plan

### Unit tests (musicbrainz.test.ts)
1. **Test: release-track credit** — mock release where track has `artist-credit: [{name: "A", joinphrase: " feat. "}, {name: "B"}]`, verify `TrackCandidate.artist = "A feat. B"` and `TrackCandidate.artists = ["A", "B"]`
2. **Test: recording-level fallback** — mock release where track has no `artist-credit` but recording has one, verify recording credit is used
3. **Test: release-level fallback** — mock release where neither track nor recording has `artist-credit`, verify release-level artist is used
4. **Test: loadTracks requests artist-credits** — verify the `inc` param includes `artist-credits`

### Integration test
- Copy the 林俊杰 FLAC to a temp path (do not mutate the real file)
- Tag the copy, verify `ARTIST` = `林俊傑 feat. MC HotDog`
- Verify `ALBUMARTIST` = `林俊傑` (unchanged)

## Execution order

1. Add `formatArtistCredit()` and `artistNamesFromCredit()` helpers
2. Update `parseTracksFromMedia()` to use per-track credits with fallback chain
3. Update `loadTracks()` API URL
4. Bump cache key/version for release-detail cache
5. Add unit tests
6. Run `just fe-check` (typecheck + tests)
7. Integration test with copied FLAC file
