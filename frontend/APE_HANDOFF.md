# APE Tag Support — Handoff Document

## Status ✅ **All bugs fixed and verified**

**954 tests pass** (1001 total, 47 pre-existing skipped).  
**Smoke test PASS** on real APE file (24MB, 3 original APEv2 blocks, MAC 3.99).

---

## Root Causes Found & Fixed (3 bugs)

### Bug 1: Stale APEv2 header not stripped
**Problem**: `stripApeTag()` only removed `tagSize` bytes from EOF (items + footer), leaving the 32-byte `APETAGEX` header if the original tag had header+footer layout.

**Fix**: `getApeTagStart()` scans all `APETAGEX` occurrences via `Buffer.indexOf`, resolves each to its true byte span (header or items start), and picks the earliest offset.

### Bug 2: `itemsStart` calculation wrong with trailing data
**Problem**: `itemsStart = data.length - tagSize` assumed the footer is at the very end. With ID3v1 or other trailing data after the APEv2 footer, this overestimates `itemsStart` by the trailing bytes.

**Fix**: Use `itemsStart = footerOffset + 32 - tagSize` (relative to the footer, not to file end).

### Bug 3: Gap between MAC audio data and first APEv2 tag
**Problem**: The real APE file had a **15-byte gap** between where `computeAudioEnd()` placed the parser (24195360) and the first `APETAGEX` header (24195375). After stripping, this gap remained in `body`. The parser's fallback path (reading remaining buffer from `audioEnd` to EOF) started at the gap, not at the items. `parseTags()` read garbage → no tags.

**Fix**: `getApeTagStart()` now calls `computeAudioEnd()` which parses the MAC descriptor/header to compute `forwardBytes` (`seekTableBytes + headerDataBytes + apeFrameDataBytes + terminatingDataBytes`). If the earliest tag byte is after `audioEnd`, it returns `audioEnd` instead, so `stripApeTag` removes the gap along with the tags.

---

## Architecture

### APEv2 writer (`frontend/electron/handlers/writer.ts`)

| Function | Purpose |
|----------|---------|
| `export parseApeTagItems(data)` | Parse APEv2 items from a buffer (finds last footer, reads items) |
| `export findApeFooterOffset(data)` | Backward scan for last `APETAGEX` footer (safety net, 2048 byte range) |
| `buildApeTagItems(entries)` | Build binary APEv2 items from key-value pairs |
| `buildApeFooter(tagSize, count)` | Build 32-byte APEv2 footer |
| `stripApeTag(data)` | Remove all tag data via `getApeTagStart()` |
| `writeApe(filePath, fields)` | Full write pipeline: read → parse existing → merge → strip → write new |

### Merge semantics (matching Vorbis writer)
- `undefined` = keep existing field unchanged
- `null` or `""` = delete field from tag
- `string` = set field to that value
- `artist` + `artists` merge into single `ARTIST` list
- `albumArtist` + `albumArtists` merge into single `ALBUM ARTIST` list

### Readback fallback (`frontend/electron/handlers/tracks.ts`)

**`readTrackMetadata`**: After `parseFile()`, if the file is `.ape` and `common.title` is blank, calls `parseApeTagItems()` on the raw file and populates TrackData fields from APEv2 items.

**`readExtraTags`**: If native tag iteration returns no rows for `.ape` files, falls back to `parseApeTagItems()` and filters out standard keys via `METADATA_EDITOR_KEYS`.

---

## Smoke Test Results

**File**: `/private/tmp/auto-tagger-ape-smoke/10 - 我们最后的话.ape` (24,195,732 bytes, MAC 3.99, 2 ch, 44100 Hz, 16 bit)

**Before**: 3 APETAGEX blocks (1 header+footer pair + 1 orphan footer), `tagTypes: []`, no tags visible.

**After `writeTags({ title: "SmokeTest", track: "10" })`**:
- 1 APETAGEX block (clean footer-only)
- `tagTypes: ['APEv2']`
- `common.title === "SmokeTest"` ✓
- `common.track.no === 10` ✓

---

## Files Changed

| File | Changes |
|------|---------|
| `frontend/electron/handlers/writer.ts` | `getApeTagStart`, `computeAudioEnd`, `findApeFooterOffset`, fixed `stripApeTag`, fixed `parseApeTagItems`. Exported `parseApeTagItems`, `findApeFooterOffset`. |
| `frontend/electron/handlers/tracks.ts` | APEv2 readback fallback in `readTrackMetadata` and `readExtraTags`. Import `parseApeTagItems`, `readFile`. |
| `frontend/test/services/ApeWriter.test.ts` | 25 tests including ID3v1 and header+footer regression + gap handling |
| `frontend/APE_HANDOFF.md` | This file |

---

## To Verify in the App

1. **Sidebar editor**: Edit a field (e.g. title) on an `.ape` file — should appear immediately
2. **Converter**: Run filename-to-tags on an `.ape` file — tags should show up
3. **Undo**: After write, Cmd+Z should revert tags
4. **Multiple writes**: Write tags several times — no stale bytes accumulate
