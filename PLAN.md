# Plan: Add APE (Monkey's Audio) File Support

## Context

The app (Electron frontend, TypeScript/JavaScript only — no Python) currently does not recognize
or load `.ape` (Monkey's Audio) files. A user has an APE file at:
`/Volume/downloads/music/刺猬乐队/刺猬乐队 - 幻象波谱星 APE` that they cannot open.

`music-metadata` (npm, already a dependency) ships with a complete APEv2 parser — reading APE
metadata works out of the box once the extension is allowed. The missing piece is that `.ape` is
excluded from the extension allowlists, and the tag writer (`writer.ts`) lacks an APEv2 code path.

Goal: make APE files fully visible, readable, and writable within the app — they should appear in
the library browser, have their metadata displayed in the editor, and accept tag writes (both
standard and extra tags).

## Approach

### 1. Add `.ape` to extension allowlists (read support)

The `SUPPORTED_EXTENSIONS` sets in `tracks.ts` and `library.ts` need `.ape` added. This makes APE
files discoverable during scanning and parseable by `music-metadata` (which already registers an
`apeParserLoader` for `.ape` files in `ParserFactory.ts`).

### 2. Add APEv2 tag writing to the writer

`node-id3` does not support APEv2, so a purpose-built APEv2 tag writer is needed.
The APEv2 format is a simple key-value binary format: tag items followed by a 32-byte footer at
the end of the file. This follows the same pattern as the existing Vorbis/writer code.

## Files to Modify

| # | File | Change |
|---|------|--------|
| 1 | `frontend/electron/handlers/tracks.ts` | Add `.ape` to `SUPPORTED_EXTENSIONS` and `EXTRA_TAG_EXTENSIONS` |
| 2 | `frontend/electron/handlers/library.ts` | Add `.ape` to `SUPPORTED_EXTENSIONS` |
| 3 | `frontend/electron/handlers/writer.ts` | Add `.ape` case to `writeTags`, `writeExtraTags`, `writeTagsWithOutcome`, `writeExtraTagsWithOutcome`; implement `writeApe` and `writeApeExtraTags` |

## Reuse

- **`music-metadata`** (npm, already a dependency) — `apeParserLoader` maps `.ape` → `APEv2Parser`;
  reading metadata works automatically once the extension is in the allowlist.
- Existing `writeVorbis` / `writeMp3` patterns in `writer.ts` — read existing tags from the binary
  file, merge with new fields, write back. The APEv2 writer follows the identical pattern.
- Tag field mapping conventions already established for Vorbis/Comments apply directly to APEv2
  (case-insensitive key-value pairs).

## Steps

### Step 1: Frontend — Add `.ape` to extension allowlists in `tracks.ts`

- In `frontend/electron/handlers/tracks.ts`:
  - Add `".ape"` to `SUPPORTED_EXTENSIONS` set (line ~30)
  - Add `".ape"` to `EXTRA_TAG_EXTENSIONS` set (line ~13)

### Step 2: Frontend — Add `.ape` to extension allowlist in `library.ts`

- In `frontend/electron/handlers/library.ts`:
  - Add `".ape"` to `SUPPORTED_EXTENSIONS` set (line ~18)

### Step 3: Frontend — Implement APEv2 tag writer in `writer.ts`

APEv2 tag layout (at the **end** of the file):

```
[APE audio data]
[Tag items...]
[Footer (32 bytes)]
```

**Footer** (32 bytes):
| Offset | Size | Field | Value |
|--------|------|-------|-------|
| 0 | 8 | Preamble | `APETAGEX` (ASCII) |
| 8 | 4 | Version | 2000 (LE uint32 = `0x000007D0`) |
| 12 | 4 | Tag size | Total tag bytes including footer (LE uint32) |
| 16 | 4 | Item count | Number of tag items (LE uint32) |
| 20 | 4 | Flags | `0x80000000` (bit 31 = footer present, LE) |
| 24 | 8 | Reserved | All zeros |

**Each tag item** (immediately before footer):
| Offset | Size | Field | Value |
|--------|------|-------|-------|
| 0 | 4 | Value size | Length of value in bytes (LE uint32) |
| 4 | 4 | Item flags | `0x20000000` (bit 29 = UTF-8 text, LE) |
| 8 | varies | Key | Null-terminated ASCII/UTF-8, uppercase |
| after \0 | varies | Value | Raw bytes (UTF-8 text) |

Tag field mapping (same uppercase keys as Vorbis comments):
- `TITLE`, `ARTIST`, `ALBUM`, `ALBUM ARTIST`, `DATE` (year), `GENRE`, `COMPOSER`,
  `COMMENT`, `LYRICS`, `DESCRIPTION`
- `TRACK` (format `"1"` or `"1/10"`), `DISC` (format `"1"` or `"1/2"`)
- `COMPILATION` (`"1"` / `""`)
- `MUSICBRAINZ_TRACKID`, `MUSICBRAINZ_ALBUMID`, `MUSICBRAINZ_ARTISTID`
- Cover art: APEv2 supports `COVER ART (FRONT)` but for MVP we skip cover writing
  (cover art in APE is uncommon; existing `hasCover` detection via embedded pictures
  won't find any and the external cover fallback works fine)

Implementation:

- Add `writeApe(filePath, fields)`:
  1. Read file into buffer
  2. Look for existing APEv2 footer at end of file (scan for `APETAGEX` at `buffer.length - 32`)
  3. If found, strip existing tag (truncate buffer to `buffer.length - tagSize`)
  4. Build tag items from `WriteFields` (skip null/undefined fields)
  5. Serialize items + footer
  6. Append to buffer and write back with `writeFile`
- Add `writeApeExtraTags(filePath, extraTags)` — same pattern, but processes `ExtraTagUpdate[]`
  and preserves standard tags, stripping only non-standard ones (matching Vorbis extra-tag logic)
- Add `.ape` to the switch statements in `writeTags()`, `writeExtraTags()`,
  `writeTagsWithOutcome()`, `writeExtraTagsWithOutcome()`

## Verification

1. **Scan test** — Place a real `.ape` file under a test library directory, run a scan, verify
   the file appears and metadata (title, artist, album, duration) is correctly read.
2. **Write test** — Generate a minimal APE file via ffmpeg
   (`ffmpeg -f lavfi -i anullsrc=r=44100:cl=mono -t 1 -acodec ape test.ape`),
   write tags to it, re-read and verify values.
3. **Write round-trip test** — Clone an existing test pattern (e.g. `test_flac_write` from specs)
   adapted for APE — write a known set of tags, read back, assert equality.
4. **Existing test suite** — Run `npm test` to verify no regressions.
5. **Manual check** — Open the app, add a folder containing `.ape` files, verify they appear
   in the library browser, metadata shows correctly in the editor, and tag edits are saved.
