# Plan: LRC Downloading & Encoding Fixer

## Context

The Electron app currently reads **local** `.lrc`/`.txt` lyrics files that already exist on disk alongside audio files (`findLocalLyrics()` in `auto-tag.ts:918`). It does **not** download lyrics from any API, and its only encoding handling is BOM detection for UTF-16LE/BE.

The user wants:
1. **Encoding checker/fixer** that runs on **both** local LRC/TXT files and any downloaded content — detect if the encoding is wrong (e.g., shift-jis, gbk, latin-1 stored as UTF-8, or UTF-16 without BOM) and convert to proper UTF-8 before writing into audio tags
2. **LRC downloading** from a lyrics API (the described `/get` endpoint with `track_name`, `artist_name`, `album_name`, `duration`) — only when no local `.lrc`/`.txt` file exists

The writer layer already handles writing lyrics to all formats (USLT for MP3, LYRICS for FLAC/Vorbis, ©lyr for M4A), so the main work is: read local file (or fetch) → detect encoding → fix if wrong → inject into tag pipeline.

## Approach

### Architecture — Overall Flow

For each track, the lyrics resolution order is:
1. **Check for local `.lrc`/`.txt` file** — `readLocalLyrics()` (refactored from `findLocalLyrics()`)
2. **Run encoding detection/fixer** on the file content — `normalizeLyricsEncoding()`
3. If no local file **and** `lyricsDownloadEnabled`, **download from API** → `LyricsClient.fetchLyrics()`
4. **Run encoding detection/fixer** on downloaded content too
5. Write the clean UTF-8 lyrics into audio tags

### Components

A. **Encoding fixer** — shared utility (in `lyrics.ts`):
   - Detect BOM (UTF-16LE `0xFF 0xFE`, UTF-16BE `0xFE 0xFF`) → convert (existing logic, extracted)
   - Detect non-UTF-8 encodings via `jschardet` (shift-jis, gbk, latin-1, etc.)
   - Convert to UTF-8 string
   - Applied to EVERY lyrics source (local file + download)

B. **`LyricsClient`** — new class in `lyrics.ts`:
   - `fetchLyrics(trackName, artistName, albumName?, duration?): Promise<string | null>`
   - GET to configured base URL with query params
   - Returns raw bytes → passed through `normalizeLyricsEncoding()`
   - Configurable `baseUrl`, custom `User-Agent`

C. **Integration into auto-tag flow** (`auto-tag.ts`):
   - Refactor `findLocalLyrics()` → `readLocalLyrics()`: reads file bytes, passes through `normalizeLyricsEncoding()`, returns string or null
   - If null AND `config.lyricsDownloadEnabled`, call `LyricsClient.fetchLyrics()` → `normalizeLyricsEncoding()`
   - Merge into `mergedFields.lyrics`
   - Config: `lyricsDownloadEnabled` (default false), `lyricsApiUrl`

4. **Config plumbing**:
   - Add `lyricsDownloadEnabled` and `lyricsApiUrl` to `AutoTagConfig` interface
   - Add config file key mappings (`lyrics_download_enabled`, `lyrics_api_url`)
   - Add config setters in `loadConfig()` (file + env vars: `AUTO_TAG_LYRICS_DOWNLOAD_ENABLED`, `AUTO_TAG_LYRICS_API_URL`)
   - Add `saveConfig` key → YAML key mapping

5. **UI — SettingsModal**:
   - Add toggle: "Auto-download Lyrics" 
   - Add field: "Lyrics API URL" (with sensible default)
   - Follow existing `ToggleRow` and `FieldRow` patterns

6. **IPC wiring** — `main.ts`:
   - Register a new `lyrics:fetch` handler (for potential standalone UI use)
   - Auto-tag flow calls it internally, no separate IPC needed for the main flow

7. **Optional per-track UI** — `MetadataEditor.tsx`:
   - A "Download Lyrics" button next to existing lyrics display
   - Calls `lyrics:fetch` IPC, shows preview, user can accept

### Dependencies

We need lightweight encoding detection + conversion:
- **jschardet** (pure JS, no native deps) — charset detection
- Node.js built-in `TextDecoder` / `TextEncoder` (available in Electron) — conversion from detected charset to UTF-8. This covers major encodings (shift-jis, gbk, big5, latin-1, etc.) without needing `iconv-lite`.

If `jschardet` + `TextDecoder` prove insufficient for edge cases, add `iconv-lite` (pure JS, no native deps) as a fallback.

## Files to Modify

| File | Change |
|------|--------|
| `frontend/electron/handlers/lyrics.ts` | **New file** — LyricsClient + encoding fixer utility |
| `frontend/electron/handlers/auto-tag.ts` | Add `lyricsDownloadEnabled`, `lyricsApiUrl` to `AutoTagConfig`; load/save in config; import and call `LyricsClient` in `applyCandidateTags()` / `findLocalLyrics()` flow |
| `frontend/electron/main.ts` | Register new IPC handlers for `lyrics:fetch` (optional standalone) |
| `frontend/electron/preload.ts` | Add `fetchLyrics` to `ElectronAPI` interface and bridge |
| `frontend/src/components/SettingsModal.tsx` | Add lyrics settings UI |
| `frontend/src/components/MetadataEditor.tsx` | (Optional) Add "Download Lyrics" button |
| `frontend/package.json` | Add `jschardet` dependency |

## Reuse

- **Existing writer** already handles `fields.lyrics` → `unsynchronisedLyrics`/`LYRICS`/`©lyr` in `writer.ts`. No changes needed there.
- **Existing `findLocalLyrics()`** BOM logic (`auto-tag.ts:921-929`) should be extracted into the shared encoding fixer in `lyrics.ts`.
- **API client pattern** follows `MusicBrainzClient`/`DiscogsClient` — one class with `fetch()` and User-Agent.
- **Config pattern** follows the existing `AutoTagConfig` interface + `CONFIG_KEY_MAP` + `loadConfig()` setters + env vars.
- **Settings UI pattern** follows `ToggleRow`/`FieldRow` in `SettingsModal.tsx`.

## Steps (in execution order)

- [ ] **1. Add dependencies**
  - `cd frontend && npm install jschardet`
- [ ] **2. Create `frontend/electron/handlers/lyrics.ts`** — encoding fixer first
  - `export function normalizeLyricsEncoding(buffer: Buffer): string`
    - Detect BOM (UTF-16LE `0xFF 0xFE`, UTF-16BE `0xFE 0xFF`) → existing logic, extracted from `findLocalLyrics`
    - For non-BOM buffers: use `jschardet` to detect encoding
    - Convert to UTF-8 via `TextDecoder` (built into Node.js/Electron)
    - Fallback to `buffer.toString("utf8")` if detection is uncertain
- [ ] **3. Create `LyricsClient`** in `lyrics.ts`
  - `export class LyricsClient` with configurable `baseUrl`, `userAgent`
  - `fetchLyrics(trackName, artistName, albumName?, duration?): Promise<string | null>`
  - GET to `<baseUrl>/get?track_name=...&artist_name=...` (optional `album_name`, `duration`)
  - Error handling: non-200 → return null, network error → return null with warn log
- [ ] **4. Refactor `findLocalLyrics()` in `auto-tag.ts`**
  - Rename to `readLocalLyrics()` to clarify it does I/O
  - Replace inline BOM detection with call to `normalizeLyricsEncoding()` from `lyrics.ts`
  - The method now: read file bytes → `normalizeLyricsEncoding()` → return string or null
  - This ensures encoding fix runs on ALL local LRC/TXT files
- [ ] **5. Add config keys in `auto-tag.ts`**
  - Add `lyricsDownloadEnabled?: boolean` and `lyricsApiUrl?: string` to `AutoTagConfig`
  - Add setters in `loadConfig()` for `lyrics_download_enabled` and `lyrics_api_url`
  - Add env var support: `AUTO_TAG_LYRICS_DOWNLOAD_ENABLED`, `AUTO_TAG_LYRICS_API_URL`
  - Add to `CONFIG_KEY_MAP`
  - Add to `saveConfig()`
- [ ] **6. Integrate download into auto-tag flow**
  - In `applyCandidateTags()`: call `readLocalLyrics()` first (which now includes encoding fix)
  - If it returns null AND `config.lyricsDownloadEnabled`:
    - Create `LyricsClient` instance
    - Call `fetchLyrics()` for the track, pass result through `normalizeLyricsEncoding()`
    - Emit event: `"Downloaded lyrics for {trackName}"`
    - Merge into `mergedFields.lyrics`
- [ ] **7. Register IPC in `main.ts` + `preload.ts`**
  - Add `lyrics:fetch` IPC handler in `main.ts`
  - Add `fetchLyrics(trackName, artistName, albumName?, duration?)` to `preload.ts`'s `ElectronAPI`
  - Expose via `contextBridge` as `window.api.fetchLyrics`
- [ ] **8. UI — SettingsModal.tsx**
  - Add toggle: "Auto-download Lyrics" → `lyricsDownloadEnabled`
  - Add field: "Lyrics API URL" → `lyricsApiUrl`
- [ ] **9. (Optional) UI — MetadataEditor.tsx**
  - Add "Download Lyrics" button in the detailed tags area
  - Shows loading state, then previews the downloaded lyrics
  - User can accept/reject before saving
- [ ] **10. Write tests**
  - `frontend/test/handlers/lyrics.test.ts`:
    - `normalizeLyricsEncoding()` with UTF-8, UTF-16LE/BE BOM, shift-jis bytes, gbk bytes
    - `LyricsClient.fetchLyrics()` with mocked fetch (success, error, non-200)
    - `readLocalLyrics()` integration (local file with wrong encoding gets fixed)

## Verification

1. **Encoding fix on local files**: Place a `.lrc` file encoded as shift-jis (or UTF-16LE) next to a track. Run auto-tag. Confirm the lyrics are written correctly as UTF-8 into the audio tag (no mojibake).
2. **Download fallback**: Remove local `.lrc` files. Enable auto-download in Settings. Run auto-tag — observe "Downloaded lyrics" events. Confirm lyrics appear in tags.
3. **Download encoding fix**: If the API returns non-UTF-8 content (e.g., shift-jis for Japanese songs), confirm the fixer converts it cleanly.
4. **Disable toggle**: Turn off auto-download. Confirm behavior reverts to local-only (no network calls).
5. **Settings UI**: Open Settings, see new toggle and URL field. Save, reopen, confirm values persist.
6. **Regression**: Existing tests pass (`cd frontend && npm test`).
7. **Unit tests**: `npx vitest run frontend/test/handlers/lyrics.test.ts` covers encoding detection, conversion, and API client.
