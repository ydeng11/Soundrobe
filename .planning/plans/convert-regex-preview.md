# Plan: Implement Convert Function — Regex-Based Bidirectional Convert with Preview

## Context

The **Convert** feature is a toolbar action that transforms between **filenames** and **tag values** (title). Currently, the implementation in `App.tsx` (`handleConvertAction`, lines ~750–830) is incomplete:

1. **`filename-to-title`** — Uses a hardcoded `"strip-number"` string (not a regex) as the default. Falls back to a user-supplied regex but only extracts **capture group 1** to become the title. It cannot map multiple capture groups to different metadata fields (title, artist, track, year, etc.).
2. **`title-to-filename`** — Shows an error: *"Title→filename rename not yet supported by the backend"* — completely unimplemented.
3. **`custom-regex`** — Applies a regex to filename but only writes to the title tag.
4. **No preview** — The user has to apply blindly and undo if wrong.
5. **No default options** — Only one hardcoded default, no selectable common patterns.

The user wants: a full regex-based bidirectional convert between filename ↔ tag, with regex capture groups that can map to any metadata field (title, artist, track, year, etc.), default preset patterns, and an inline preview before applying.

## Approach

The convert logic currently runs purely on the frontend. The approach is to **keep it on the frontend** (no backend IPC needed beyond the existing `track:write`) but fundamentally redesign the UX and logic:

1. **Redesign `ConvertDialog.tsx`** to offer a set of **default regex presets** with labels, plus a **custom regex** input. Show a **preview** of what the result will look like (both "from" and "to" rendered side by side) before the user clicks Convert.
2. **Implement `title-to-filename`** — generate a new filename from the title tag using regex/replacement patterns (e.g., `{track} - {title}{ext}`).
3. **Implement `custom-regex` for both directions** — with named groups or positional capture groups, allowing full bidirectional transformation.
4. **Add a preview panel** that shows the current value and the converted result in real-time as the user types/changes selections.

### Direction modes

### Multi-field capture group mapping

Each capture group in the regex maps to a specific metadata field. The mapping is determined by:
- **Named capture groups** (preferred): `(?<title>.+)`, `(?<artist>.+)`, `(?<track>\d+)`, `(?<year>\d{4})` — names map directly to field names
- **Positional groups** (fallback): group 1 → title, group 2 → artist, group 3 → track, group 4 → year, group 5 → album (configurable)

| Mode | Input Source | Output Target | Semantics |
|------|-------------|---------------|-----------|
| **Filename → Tag(s)** | `track.path` basename (minus extension) | Write to one or more tag fields | Regex with capture groups mapping to title, artist, track, year, album, etc. |
| **Title → Filename** | Current tags (title, artist, track, etc.) + `{ext}` from file | Rename file on disk | Template string: `{title}`, `{artist}`, `{track}`, `{year}`, `{ext}` and other field placeholders are interpolated from the track's current tags |
| **Custom / Bidirectional** | User-chosen source (filename or tags) | User-chosen target (tags or filename) | Full regex with named or positional capture groups for tag→file; template with field placeholders for file→tag |

### Default presets

Suggested presets for quick selection:

| Label | Direction | Pattern / Template | Capture mapping |
|-------|-----------|-------------------|-----------------|
| "Track - Title" | Filename→Tags | `^(?<track>\d+)\s*[-.]\s*(?<title>.+)$` | track → track number, title → title. `01 - Song Title.mp3` → track=01, title=Song Title |
| "Artist - Title" | Filename→Tags | `^(?<artist>.+?)\s*-\s*(?<title>.+)$` | artist → artist, title → title. `Artist - Song Title.mp3` → artist=Artist, title=Song Title |
| "Track. Artist - Title" | Filename→Tags | `^(?<track>\d+)\.\s*(?<artist>.+?)\s*-\s*(?<title>.+)$` | track, artist, title. `01. Artist - Song Title.mp3` → multi-field extraction |
| "Full metadata" | Filename→Tags | `^(?<track>\d+)\s*(?<artist>.+?)\s*-\s*(?<title>.+?)\s*\((?<year>\d{4})\)$` | track, artist, title, year. `01 Artist - Song Title (2023).mp3` → full extraction |
| "Title → Filename" | Tags→Filename | `{track} - {title}{ext}` | Interpolate {track}, {title}, {ext} from current tags |
| "Custom" | Both | Free-form regex/template | User-defined mapping |

### Preview behavior

As the user configures the conversion (selects direction, picks a preset or enters custom regex), the dialog should show:

- **Current value** (what will be read)
- **Converted result** (what will be written)
- Updated in real-time as the user adjusts settings

If the selected track has known fields (title, artist, trackNumber), use those in the preview. If the result is identical to the current value, show a warning/disable the Convert button.

## Files to modify

| File | Change |
|------|--------|
| `frontend/src/components/ConvertDialog.tsx` | Redesign: add preset selector, preview panel, bidirectional regex support, inline preview rendering |
| `frontend/src/App.tsx` (lines ~737–850) | Rewrite `handleConvertAction`: implement title→filename (file rename), handle bidirectional regex, support multi-field extraction (title + track number from filename) |
| `frontend/electron/preload.ts` | Optionally: add `renameFile(oldPath, newPath)` IPC export if not already available |
| `frontend/electron/main.ts` | Optionally: add IPC handler for file rename if title→filename requires it |
| `frontend/electron/handlers/tracks.ts` | Optionally: add `renameTrack` handler for renaming audio files on disk |

## Reuse / Existing code

- **Writer fields** (`frontend/electron/handlers/writer.ts`): The `WriteFields` interface already supports `title`, `trackNumber`, etc. — no changes needed for writing tags.
- **Track data** (`frontend/electron/preload.ts`, `frontend/electron/handlers/tracks.ts`): The `TrackData` interface provides all fields needed for previews and conversion.
- **Undo stack** (`App.tsx`): The existing `PUSH_UNDO` / `UPDATE_TRACK` pattern works for the file-rename case if we store old path + old title.
- **`handleConvertAction`** in `App.tsx`: Core logic to rewrite. Keep the undo/write/re-read pattern.
- **`ConvertDialog` props**: Keep `onConvert(direction, pattern)` signature but extend with a `patternType` or restructure to pass direction + source/target info.
- **`readAlbum` / `album:read`** (`tracks.ts`): Already re-reads track metadata after writes — the conversion write path re-uses this.

## Detailed Steps

### Step 1: Audit current convert usage fully

Read the full flow to confirm all call sites, state interactions, and edge cases. Already done above — no further action needed.

### Step 2: Redesign ConvertDialog (UI)

- Replace the current 3-radio layout with:
  - **Direction row**: two buttons "Filename → Title" / "Title → Filename" (toggle)
  - **Preset selector**: dropdown or chip list of common regex patterns
  - **Regex input**: text field, pre-filled when a preset is selected, editable for customization
  - **Preview panel**: shows two rows — "Current" and "Result" — rendered as formatted text
  - For `title→filename`: preview should show the new filename, with `{ext}` resolved from the current file extension
  - Disable the Convert button when preview shows "no change"
- State: track data needed for preview must be passed in (or the dialog can receive the current track)

### Step 3: Implement filename→tags regex extraction (multi-field via named/positional groups)

- Enhance the current logic to extract **any number of metadata fields** from the filename:
  1. Check for **named capture groups** first: `(?<title>.+)`, `(?<artist>.+)`, `(?<track>\d+)`, `(?<year>\d{4})`, `(?<album>.+)` — each named group maps to its corresponding field name in the write payload
  2. Fall back to **positional groups**: capture group 1 → title, group 2 → artist, group 3 → track, group 4 → year, group 5 → album
  3. If a positional mapping doesn't make sense (e.g., only 1 group), just map #1 → title
- Apply regex to basename minus extension
- Build a write payload with all extracted fields and call `writeTrack` once

### Step 4: Implement title→filename file rename

- Generate new filename from template string (e.g. `{track} - {title}{ext}`)
- Replace placeholders: `{title}` → track.title, `{artist}` → track.artist, `{track}` → track.trackNumber padded to 2 digits, `{ext}` → original file extension
- Use `fs.renameSync` on the backend via a new IPC channel `track:rename` (or inline via `fs.promises.rename`)
- Update undo to include old path + old title so undo can revert both
- Re-read the album after rename to refresh the track list

### Step 5: Implement custom regex bidirectional

- Let user provide a regex with named or positional capture groups
- Auto-detect direction: if the regex is applied to the **filename** (contains `ext`) → it's filename→tags; if applied to **tags** (contains `{title}`, `{artist}`, etc.) → it's tags→filename
- For tags→filename: use a template string with `{title}`, `{artist}`, `{track}`, `{year}`, `{album}`, `{ext}` placeholders that interpolate from current track tags
- Same preview and apply flow
- If the user provides both a source regex (to parse tags from filename) and a destination template (to build filename from tags), support chaining: parse → modify → build

### Step 6: Add preview rendering

- Compute the conversion result **in the dialog** before applying
- Show side-by-side: current value → converted result
- If result is identical to current value, show muted text and disable Convert button
- Update preview reactively on every input change (direction, preset, regex pattern)

### Step 7: Add rename IPC handler (if not already present)

Check if `writeTrack` already supports file paths (it doesn't — it only modifies tags). Add a new handler:

- IPC channel: `track:rename` → `(oldPath: string, newPath: string) => TrackData`
- Handler in `tracks.ts`: `fs.promises.rename(oldPath, newPath)` → `readTrackMetadata(newPath)`
- Export in `preload.ts` as `renameTrack(oldPath, newPath)`
- Register in `main.ts`

### Step 8: Wire up in App.tsx

- Rewrite `handleConvertAction` to handle all three modes with the new dialog data
- For title→filename: call rename IPC, then re-read album to update grid
- For filename→title: existing writeTrack flow, but support multi-field writes
- For undo: track both path changes and field changes
- Update `state.selectedTrack` to point to the new path after rename (or re-select by new path)

## Verification

1. **Manual test: Filename→Title (strip track number)**
   - Select a file named `01 - Song Title.mp3`
   - Open Convert, choose "Filename→Title", preset "Strip track number"
   - Preview shows: "01 - Song Title" → "Song Title"
   - Click Convert, verify title tag changes to "Song Title" in the grid
   - Undo restores original title

2. **Manual test: Title→Filename**
   - Select a file with title "Song Title" (current filename e.g. `unknown.mp3`)
   - Open Convert, choose "Title→Filename", preset `{track} - {title}{ext}`
   - Preview shows: `unknown.mp3` → `01 - Song Title.mp3`
   - Click Convert, verify filename changes in the grid
   - Undo restores original filename

3. **Manual test: Custom regex**
   - File `Artist - Song Title.mp3` with regex `^(.+?)\s*-\s*(.+)$`
   - Preview shows extraction of artist and title
   - Click Convert, verify both tags written

4. **Manual test: No-change case**
   - Preview shows "no change" text
   - Convert button is disabled

5. **Manual test: Error handling**
   - Invalid regex → error message, no write
   - Regex with no captures → error message
   - Rename to existing file → error handled gracefully

6. **Run existing tests**: `npx vitest run` (ensure no regressions)
