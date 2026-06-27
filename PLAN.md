# Plan: Add missing auto-tag fields to ExtraTags preset

## Context

The `ExtraTagsEditor` component has a `SUGGESTED_TAG_KEYS` preset that provides autocomplete suggestions when users type in the tag key field. The auto-tag feature writes several extra tags (tags not in the standard metadata editor) that are missing from this preset, so users can't get autocomplete suggestions for them.

## Analysis

Auto-tag writes these fields via `WriteFields` in `writer.ts`. Some become standard metadata tags (shown in the main editor), while others become "extra tags" (shown in the ExtraTags editor because they're not in `METADATA_EDITOR_KEYS`).

**Missing from `SUGGESTED_TAG_KEYS`:**

| Tag key | Written by | Format |
|---------|-----------|--------|
| `COMPILATION` | `writeVorbis`, `mergeMp3UserDefinedText` | Vorbis + MP3 TXXX |
| `ARTISTS` | `setVorbisList`, `mergeMp3UserDefinedText` | Vorbis + MP3 TXXX |
| `ALBUMARTISTS` | `setVorbisList`, `mergeMp3UserDefinedText` | Vorbis + MP3 TXXX |
| `MusicBrainz Track Id` | `mergeMp3UserDefinedText` | MP3/WAV TXXX only |
| `MusicBrainz Album Id` | `mergeMp3UserDefinedText` | MP3/WAV TXXX only |
| `MusicBrainz Artist Id` | `mergeMp3UserDefinedText` | MP3/WAV TXXX only |
| `Discogs Artist Id` | `mergeMp3UserDefinedText` | MP3/WAV TXXX only |
| `Discogs Release Id` | `mergeMp3UserDefinedText` | MP3/WAV TXXX only |

**Already in preset (no change needed):** `MUSICBRAINZ_TRACKID`, `MUSICBRAINZ_ALBUMID`, `MUSICBRAINZ_ARTISTID`, `DISCOGS_ARTIST_ID`, `DISCOGS_RELEASE_ID`, `DESCRIPTION`

The Vorbis uppercase forms (e.g. `MUSICBRAINZ_TRACKID`) match when the user types "MUSICBRAINZ", but the MP3 space-separated descriptions (e.g. `MusicBrainz Track Id`) don't match because `toUpperCase()` produces "MUSICBRAINZ TRACK ID" which doesn't match `MUSICBRAINZ_TRACKID` (underscore vs space).

## Approach

### File-type-specific suggestions

The suggestion list must be file-type specific — tags not used by a file's format should not appear. Replace the static `SUGGESTED_TAG_KEYS` constant with a function `getSuggestedTagKeys(filePath: string): string[]` that derives the correct list based on the file extension.

**Vorbis files** (`.flac`, `.ogg`, `.opus`) — uppercase keys:
- Current Vorbis/general section (COMMENT, DESCRIPTION, …, TOTALTRACKS)
- Add: `ARTISTS`, `ALBUMARTISTS`, `COMPILATION`
- MusicBrainz section (MUSICBRAINZ_ALBUMID, …, MUSICBRAINZ_WORKID)
- Discogs section (DISCOGS_ARTIST_ID, …, DISCOGS_VOTES)
- Do NOT include space-separated TXXX descriptions like `MusicBrainz Track Id`

**ID3v2 files** (`.mp3`, `.wav`) — TXXX descriptions with spaces:
- Current ID3v2 native frame IDs (TCOM, TIT3, …, TSOT)
- Add TXXX descriptions: `MusicBrainz Track Id`, `MusicBrainz Album Id`, `MusicBrainz Artist Id`, `Discogs Artist Id`, `Discogs Release Id`, `ARTISTS`, `ALBUMARTISTS`, `COMPILATION`, `DESCRIPTION`, `COMMENT`
- Do NOT include Vorbis-style `MUSICBRAINZ_*` or `DISCOGS_*_ID` keys

**APE files** (`.ape`) — uppercase keys (same as Vorbis):
- Same list as Vorbis files

### Implementation

1. Replace the static `SUGGESTED_TAG_KEYS` constant with a function `getSuggestedTagKeys(filePath: string): string[]`
2. Extract the file extension, classify into `vorbis`, `id3v2`, or `ape`
3. Return the appropriate subset of tag keys
4. Update `keySuggestions` memo in `ExtraTagsEditor` to call `getSuggestedTagKeys(track.path)` instead of referencing the static array

## Files to modify

- `frontend/src/components/ExtraTagsEditor.tsx` — replace static `SUGGESTED_TAG_KEYS` with `getSuggestedTagKeys(filePath)` function; update `keySuggestions` memo

## Verification

- Open the app, open ExtraTags editor on a FLAC track:
  - Type "comp" → should suggest `COMPILATION`
  - Type "artist" → should suggest `ARTISTS`
  - Should NOT show `MusicBrainz Track Id` (space-separated MP3 form)
- Open ExtraTags on an MP3 track:
  - Type "Discogs Art" → should suggest `Discogs Artist Id`
  - Type "MusicBrainz" → should suggest `MusicBrainz Track Id`, `MusicBrainz Album Id`, etc.
  - Should NOT show `MUSICBRAINZ_TRACKID` (Vorbis uppercase form)
- Open ExtraTags on an APE track:
  - Same behavior as FLAC (uppercase Vorbis-style keys)
