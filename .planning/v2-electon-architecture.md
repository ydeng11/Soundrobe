# Auto Tagger v2 — Pure TypeScript Electron Architecture

**Status:** Draft  
**Created:** 2026-05-23

## Goal

Build an MP3tag-like desktop tag editor using **Electron + React + TypeScript**. Single language, single process, no Python dependency.

All music-library logic (tag read/write, MusicBrainz lookups, Discogs lookups, LLM integration, cover art) runs in the Electron main process. The renderer is a React app that communicates via Electron IPC.

## Architecture

```
┌───────────────────────────────────────────┐
│  Electron App (.app / .exe / .AppImage)  │
│                                           │
│  ┌───────────────────────────────────┐   │
│  │  Renderer (React + Vite +         │   │
│  │  Tailwind)                        │   │
│  │                                   │   │
│  │  App.tsx                          │   │
│  │  ├── AlbumBrowser (DataTable)     │   │
│  │  ├── TrackTable (sort/filter/     │   │
│  │  │   multi-select, batch edit)    │   │
│  │  ├── TagPanel (form fields +      │   │
│  │  │   validation, <keep> support)  │   │
│  │  ├── CoverArt (img preview)       │   │
│  │  ├── Toolbar (buttons)            │   │
│  │  ├── StatusBar (filter + stats)   │   │
│  │  └── SettingsModal                │   │
│  │                                   │   │
│  │  state/                           │   │
│  │  └── AppState, UndoManager        │   │
│  └───────────┬───────────────────────┘   │
│              │ contextBridge (ipcRenderer │
│              │ invoke/handle)             │
│  ┌───────────▼───────────────────────┐   │
│  │  Main Process (Node.js)           │   │
│  │                                   │   │
│  │  Library scanning                 │   │
│  │  taglib-ts   → read+write tags    │   │
│  │  fetch()     → MusicBrainz API    │   │
│  │  fetch()     → Discogs API        │   │
│  │  fetch()     → OpenRouter API     │   │
│  │  better-sqlite3  → cache + local dataset │   │
│  │  ~/.auto-tagger/ → first-priority SQLite │   │
│  │  sharp        → cover art resize         │   │
│  │  opencc-js    → Chinese text             │   │
│  │  child_process → ffprobe, rgain3         │   │
│  │  yaml         → config files             │   │
│  └───────────────────────────────────┘   │
└───────────────────────────────────────────┘
```

**Single process.** No HTTP servers, no ports, no health checks. The main process does everything. The renderer is a thin UI layer.

## Key Decisions

### 1. Language: TypeScript (100%)
- Electron main process handles all music-library operations
- `taglib-ts` for tag read/write across all supported formats (MP3 ID3v2, FLAC Vorbis, MP4 atoms, WAV)
- `musicbrainz-api` or raw `fetch()` for MusicBrainz lookups
- Discogs, OpenRouter, LLM — all `fetch()`
- `sharp` for cover art image processing
- `better-sqlite3` for cache and dataset index
- `opencc-js` for Chinese text conversion
- `child_process` for ffprobe and ReplayGain

### 2. UI Framework: Electron
- Agent-debuggable via `electron.launch` → CDP → `snapshot -i`, `click`, `fill`, `screenshot`, `qa.attached`, `eval --stdin`
- Cross-platform (macOS ARM, Windows x64, Linux x64)
- One binary, one package manager, one test framework

### 3. Frontend: React + Vite + Tailwind CSS
- `frontend/` directory at project root
- Dev mode: `npm run dev` starts Vite HMR + Electron main process
- Production: Vite builds static files into `dist/`, Electron loads from disk

### 4. Tag I/O: music-metadata (read) + node-id3 (write) / custom writers
- **Read:** `music-metadata.parseFile(path)` → returns all tag fields, cover art, format info
- **Write:** `node-id3` for MP3 ID3v2 tags; custom Vorbis comment writer for FLAC; WAV writes are a no-op
- Coverage: MP3 (ID3v2.3/2.4), FLAC, Ogg Vorbis, Opus, MP4/M4A, WAV
- Pure JavaScript (music-metadata) + thin native addon (node-id3 for MP3). No `@electron/rebuild` complexity required.
- Simpler than the originally planned `taglib-ts` — avoids native rebuild issues at the cost of format-specific write code.

### 5. Communication: Electron IPC (contextBridge)
- No HTTP, no REST, no ports
- Renderer calls `window.api.scanLibrary(path)` → main process runs `taglib-ts` on the filesystem → returns results as plain objects
- All free-form objects pass through Electron's structured clone algorithm (no serialization issues)

```typescript
// preload.ts
contextBridge.exposeInMainWorld("api", {
  // Library
  scanLibrary: (path: string) => ipcRenderer.invoke("library:scan", path),
  refreshAlbum: (path: string) => ipcRenderer.invoke("album:refresh", path),

  // Tracks
  readAlbum: (path: string) => ipcRenderer.invoke("album:read", path),
  writeTrack: (path: string, fields: Record<string, unknown>) => ipcRenderer.invoke("track:write", path, fields),
  writeTracks: (updates: Array<{path: string; fields: Record<string, unknown>}>) => ipcRenderer.invoke("tracks:batch-write", updates),

  // Auto-tag
  autoTagAlbum: (path: string) => ipcRenderer.invoke("album:auto-tag", path),
  getTaskProgress: (taskId: string) => ipcRenderer.invoke("task:progress", taskId),
  cancelTask: (taskId: string) => ipcRenderer.invoke("task:cancel", taskId),
  getDatasetStatus: () => ipcRenderer.invoke("dataset:status"),

  // Cover art
  getCoverUrl: (albumPath: string) => ipcRenderer.invoke("cover:path", albumPath),

  // Config
  getConfig: () => ipcRenderer.invoke("config:get"),
  setConfig: (key: string, value: unknown) => ipcRenderer.invoke("config:set", key, value),

  // Events (renderer → main)
  onFocus: () => ipcRenderer.invoke("window:focused"),
})
```

### 6. State Management: Thick Client / Optimistic
- React `useReducer` holds library state, selected album, selected tracks
- Field edits: React updates state immediately, calls `api.writeTrack()` async, rolls back on error
- Undo: client-side stack of `TrackSnapshot` objects — same pattern as current `undo.py`, ported to TypeScript
- Sort/filter: client-side, no IPC
- Same data model as current `AppState`, `AlbumData`, `TrackData`, `TrackAuditResult`

### 7. Library Scanning
- `scanLibrary(path)` → Electron main process uses `fs.readdir` to find audio files, groups by album directory
- Returns full list (same as current behavior)
- Sub-second for 100K files (Node.js `fs.readdir` is fast)
- Refresh button re-scans
- Window focus triggers a lightweight re-scan

### 8. Cover Art
- Main process discovers: external `cover.jpg`/`cover.png` → embedded cover (via `taglib-ts`) → returns file path
- Renderer loads via `file://` protocol or converts to `data:` URL via `sharp`
- No HTTP endpoint needed — it's local IPC

### 9. Auto-Tag (MusicBrainz + LLM)
- Main process calls MusicBrainz API directly via `fetch()`
- Rate limiting: simple `setTimeout` between requests (1 req/sec)
- Candidate ranking: port `candidates.py` logic to TypeScript (pure string comparison)
- LLM selection: `fetch()` to OpenRouter API with the same prompt templates (ported to TypeScript strings)
- Long-running: returns a `taskId`, renderer polls `getTaskProgress(taskId)`

### 10. File Watching
- `document.visibilitychange` → `api.onFocus()` → main process re-scans current album directory
- Refresh button for explicit reload
- No `fs.watch` for MVP

### 11. Undo: Client-Side
- TypeScript `UndoManager` class (port of `undo.py`)
- Session-only — lost on app restart (same as MP3tag)
- Each operation pushes `TrackSnapshot[]` into stack
- Undo = `api.writeTrack(path, oldFields)` to restore

### 12. CLI / Headless Mode
- The Electron app is the primary product
- Python v1 is **kept as-is** for users who want CLI batch processing (`auto-tag batch`). No changes to the Python package.
- No CLI mode in v2 for MVP. Optional: add a CLI entry point later via `ts-node` or a compiled Node.js binary.

### 13. Packaging
- **Dev:** `npm run dev` — Vite HMR + Electron
- **Production:** `npm run build` → Electron Builder → `.dmg` (macOS), `.exe` (Windows), `.AppImage` (Linux)
- No Python, no PyInstaller, no second build system
- `taglib-ts` rebuild for Electron ABI: `@electron/rebuild` runs in `postinstall` script, or prebuilt binaries in CI

### 14. Configuration
- Config file: `~/.config/auto-tagger/config.yaml` read via `yaml` npm package on startup
- LLM API key, model, dataset paths — all in the YAML file
- UI preferences: Electron `electron-store` package (JSON file in userData)
- Settings modal in the UI calls `api.getConfig()` / `api.setConfig()`

### 15. Testing Strategy

| Layer | Tool | What it covers |
|---|---|---|
| **Library (tag reading, MusicBrainz client, etc.)** | Vitest | No Electron, pure Node.js tests. Mock `fetch()`. Test against real audio fixture files with music-metadata and node-id3. |
| **React components** | Vitest + jsdom | Component rendering, form validation, state management |
| **IPC handlers** | Vitest + Electron mocking | Verify handler logic, mock `music-metadata`, mock `fetch()` |
| **Integration (smoke)** | Bash script | Starts Electron app, verifies it loads and connects to the filesystem |
| **End-to-end exploratory** | Agent tooling | `electron.launch` → `snapshot` → `qa.attached` → `screenshot`. Manual debugging, not CI. |

No E2E framework. The Electron main process is ~150 lines of IPC wiring. The complex logic (tag reading, MusicBrainz matching, LLM selection) is pure TypeScript tested with Vitest.

## Project Structure

```
auto_tagger/
├── frontend/                          # Electron + React app
│   ├── package.json
│   ├── vite.config.ts
│   ├── tsconfig.json
│   ├── tailwind.config.ts
│   ├── electron/
│   │   ├── main.ts                    # Main process: IPC handlers, window
│   │   ├── preload.ts                 # contextBridge
│   │   └── handlers/
│   │       ├── library.ts             # scanLibrary, refreshAlbum
│   │       ├── tracks.ts              # readAlbum, writeTrack, batchWrite
│   │       ├── auto-tag.ts            # Dataset → MusicBrainz → LLM auto-tag
│   │       ├── dataset.ts             # Local SQLite lookups (~/.auto-tagger/)
│   │       ├── cover.ts               # cover art discovery
│   │       ├── lyrics.ts              # LRC discovery + encoding fix
│   │       ├── discogs.ts             # artist artwork from Discogs
│   │       └── config.ts              # config file read/write
│   ├── src/                           # React renderer
│   │   ├── App.tsx
│   │   ├── hooks/
│   │   │   ├── useLibrary.ts
│   │   │   ├── useUndo.ts
│   │   │   └── useTaskPolling.ts
│   │   ├── components/
│   │   │   ├── Toolbar.tsx
│   │   │   ├── AlbumBrowser.tsx
│   │   │   ├── TrackTable.tsx
│   │   │   ├── TagPanel.tsx
│   │   │   ├── CoverArt.tsx
│   │   │   ├── StatusBar.tsx
│   │   │   └── SettingsModal.tsx
│   │   └── state/
│   │       ├── AppState.ts           # Types + reducers
│   │       ├── UndoManager.ts        # Port of undo.py
│   │       └── actions.ts
│   └── test/
│       ├── handlers/                 # Main process handler tests
│       └── components/               # React component tests
├── src/auto_tagger/                   # Unchanged Python v1 (CLI)
│   ├── cli.py
│   ├── commands/
│   ├── core/
│   ├── integrations/
│   ├── features/
│   ├── quality/
│   ├── config/
│   └── ui/                           # TUI — unchanged (not touched by v2)
├── tests/                             # Unchanged Python v1 tests
├── pyproject.toml                     # Unchanged
└── Justfile                           # v1 commands unchanged
```

## Build Phases

### Phase 1: Electron Scaffold + tag I/O (DONE)
- Initialized `frontend/` with Vite + React + Tailwind + Electron
- Used `music-metadata` (parse) + `node-id3` (write) instead of the originally-planned `taglib-ts`
- Implemented `electron/main.ts`, `electron/preload.ts`, IPC channels
- Built `handlers/library.ts` — scan library directory for audio files
- Built `handlers/tracks.ts` — `readAlbum()` using `music-metadata`
- Built `handlers/cover.ts` — cover art discovery via `sharp`
- Built `handlers/directory.ts` — directory listing for tree browser
- Built `handlers/writer.ts` — tag writing via `node-id3` for MP3, custom Vorbis/FLAC writer
- Tests for all handlers with Vitest (library, cover, writer)
- **Result:** Launch app, open library, browse individual tracks, edit and save tags

### Phase 2: React UI — FileGrid + MetadataEditor + TitleBar (DONE)
**Actual implementation (simpler than original plan):**
- `FileGrid.tsx` — sortable data table of all tracks with multi-column sort, regex filtering, shift-click multi-select, alternating row colors, footer stats
- `MetadataEditor.tsx` — right-pane form with fields for Title, Artist, Album, Year, Track, Genre, Composer, Comment; cover art preview with Change/Remove buttons; format details (codec, sample rate, bitrate, size); detailed tags section (MusicBrainz IDs, lyrics preview, disc info)
- `TitleBar.tsx` — top toolbar with Open Library button, library path + count display, search/filter input, Save/Revert/Convert/Num/Rename action buttons, error display, file count
- `state/AppState.ts` — `useReducer`-based state with `AppState + AppAction` discriminated union; `SET_LIBRARY`, `SET_ALBUMS`, `SET_TRACKS`, `SET_ACTIVE_ALBUM`, `SELECT_TRACK`, `UPDATE_TRACK`, `PUSH_UNDO`, `POP_UNDO`, `SET_SAVING`, `SET_ERROR`, `CLEAR_ALL`, and more
- `state/UndoManager.ts` — session-only undo stack of `TrackSnapshot[]` with configurable max depth (50)
- `state/actions.ts` — `TrackSnapshot` type export
- Optimistic field edits with rollback on write failure (wired in `App.tsx`)

**Deferred from original Phase 2 scope (not implemented):**
- Filename ↔ Tag conversion dialog
- Lyrics / LRC discovery and encoding fix
- Discogs artist artwork fetching

### Phase 3: Auto-Tag (Local Dataset first, then MusicBrainz + LLM) — DONE

**Implemented files:**
- `handlers/auto-tag.ts` — Orchestrator: task queue, full lookup chain, cancellation, config loading
- `handlers/candidates.ts` — AlbumCandidate, TrackCandidate, LookupRequest types + queryHash + normalizeLookupText
- `handlers/cache.ts` — MatchCache: SQLite lookup cache, album state ledger, LLM extraction
- `handlers/aliases.ts` — saveAlias, getAliases, isChineseName
- `handlers/fallback.ts` — folder name parsing, year extraction, folder fallback candidate
- `handlers/dataset.ts` — DatasetReader: queries ~/.auto-tagger/ SQLite index
- `handlers/musicbrainz.ts` — MusicBrainzClient: raw fetch() to JSON API, 1 req/sec rate limit
- `handlers/discogs.ts` — DiscogsClient: raw fetch() to Discogs API, sliding-window rate limiter
- `handlers/openrouter.ts` — OpenRouterClient: chat completions with structured JSON, retries, cost estimation
- `handlers/prompts.ts` — Prompt builders for selection, fallback, folder extraction
- `handlers/schemas.ts` — TypeScript interfaces for structured LLM responses
- `electron/main.ts` — Real IPC handlers wired (album:auto-tag, task:progress, task:cancel, dataset:status, config:get/config:set)

**Tests:** 205 across 17 test files, all passing

### Phase 4: Polish + Packaging — IN PROGRESS

**Done:**
- SettingsModal.tsx — modal with LLM API key, model, Discogs token, remote lookup toggle, Discogs toggle; loads/saves via IPC
- AppState TOGGLE_SETTINGS action, TitleBar ⚙️ button wired
- Window state persistence: saves position/size/maximized to `~/.auto-tagger/window-state.json`, restores on startup, validates against available displays
- File watching: `visibilitychange` listener calls `api.onFocus()` on focus

**Tests:** 218 across 19 test files, all passing

All Phase 4 items are complete.

**Build targets (via `npm run dist:*`):**
- `dist:mac` → macOS `.dmg` (arm64 + x64) + `.zip`
- `dist:win` → Windows `.exe` (NSIS installer, x64)
- `dist:linux` → Linux `.AppImage` + `.deb` (x64)
- `rebuild-native` → rebuilds `better-sqlite3` + `sharp` for Electron ABI (run before first `dist:*` after install)

## Build Phases Summary

| Phase | Scope | Maturation | Status |
|---|---|---|---|
| **1** | Electron scaffold + tag I/O (`music-metadata` + `node-id3`) | Browse library, view metadata, cover art | ✅ Done |
| **2** | React UI (FileGrid + MetadataEditor + TitleBar) + optimistic writes + undo | Full tag editor | ✅ Done |
| **3** | Auto-tag (MusicBrainz + LLM) | Right-click auto-tag | ✅ Done |
| **4** | Polish + packaging | Shippable .app | ✅ Done |

## Feature Scope

| Feature | In v1 Python? | In v2 Electron? | Phase |
|---|---|---|---|
| Browse library by track | ✅ | ✅ (FileGrid) | 1 |
| View track metadata in table | ✅ | ✅ (FileGrid) | 1 |
| Edit individual fields | ✅ | ✅ (MetadataEditor) | 2 |
| Multi-track batch edit | ✅ | ✅ (Shift-click multi-select, writes per-track) | 2 |
| Cover art preview | ✅ | ✅ (MetadataEditor) | 2 |
| Undo (session-only) | ✅ | ✅ (UndoManager) | 2 |
| Sort by column | ✅ | ✅ (FileGrid — all columns) | 2 |
| Filter by regex/text | ✅ | ✅ (TitleBar filter input) | 2 |
| Dark theme | ❌ | ✅ (Tailwind dark theme) | 2 |
| Auto-tag via MusicBrainz + LLM | ✅ | ✅ (auto-tag.ts + musicbrainz.ts + discogs.ts + openrouter.ts + dataset.ts) | 3 ✅ |
| Filename → Tag conversion | ❌ | ❌ Deferred | 2 |
| Tag → Rename files | ❌ | ❌ Deferred | 2 |
| Auto-numbering wizard | ❌ | ❌ Deferred | — |
| ReplayGain calculation | ✅ | ❌ Deferred | — |
| Lyrics / LRC | ✅ | ❌ Deferred | 2 |
| LLM audit | ✅ | ❌ Deferred | — |
| Health reports | ✅ | ❌ Deferred | — |
| Clean junk tags | ✅ | ❌ Deferred | — |
| Discogs artist artwork | ✅ | ❌ Deferred | 2 |
| Python CLI (`auto-tag batch`, etc.) | ✅ | ❌ (use v1) | Separate |

## Risks & Mitigations

| Risk | Mitigation |
|---|---|
| `music-metadata` + `node-id3` write support differs per format | Test against real fixture files for MP3, FLAC, M4A. Compare output with mutagen (v1). WAV writes are a no-op (tagged via sidecar/CLI). |
| MusicBrainz rate limiting (1 req/sec) with 50-track album | Sequential lookup with per-track timeout. Show progress in the task poller. Dataset hit skips network entirely. |
| LLM API key management (no env vars, no CLI) | Config file + settings form. Same as v1's `config.yaml`. |
| No headless/CI mode in v2 | v1 Python CLI is still available for scripting. v2 Electron app is for interactive desktop use. Dataset setup (`auto-tag dataset setup`) still requires Python CLI (run once, then v2 reads the same `.db`). |
| Native rebuild per platform/architecture/Electron-version matrix | Use `electron-builder` which handles this for common targets. Pin Electron version. Prebuild only for macOS ARM + Windows x64 + Linux x64. |

## Reference: Existing Python Code → TypeScript

| Python file | TypeScript equivalent | Strategy |
|---|---|---|
| `state.py` | `frontend/src/state/AppState.ts` | Direct port of types and reducer logic |
| `undo.py` | `frontend/src/state/UndoManager.ts` | Direct port of class structure |
| `core/audio.py` + `core/formats.py` + `core/metadata.py` | `music-metadata` (read) + `node-id3` (write) | Replaced entirely |
| `core/writer.py` | `node-id3` + custom Vorbis writer | Replaced entirely |
| `integrations/beets_client.py` | `handlers/auto-tag.ts` | Rewrite as raw `fetch()` to MusicBrainz API |
| `integrations/candidates.py` | `handlers/auto-tag.ts` | Algorithm port (string matching, ranking) |
| `integrations/discogs_client.py` | `handlers/discogs.ts` | Rewrite as raw `fetch()` for artist image lookup |
| `integrations/lookup.py` | `handlers/auto-tag.ts` + `handlers/dataset.ts` | Orchestration: dataset → MusicBrainz → LLM |
| `integrations/cache.py` | `better-sqlite3` | SQLite schema port |
| `integrations/dataset.py` + `dataset_raw.py` | `handlers/dataset.ts` + `better-sqlite3` | Read existing SQLite index at `~/.auto-tagger/`. Query before any network call. |
| `llm/client.py` | `handlers/auto-tag.ts` | fetch() to OpenRouter |
| `llm/prompts.py` | `handlers/auto-tag.ts` | Template strings |
| `llm/selection.py` | `handlers/auto-tag.ts` | Logic port |
| `llm/schemas.py` | `handlers/auto-tag.ts` | TypeScript interfaces |
| `llm/cost.py` | `handlers/auto-tag.ts` | Math port |
| `features/cover_art.py` | `handlers/cover.ts` | Logic port |
| `features/compilations.py` | `handlers/library.ts` | Logic port |
| `config/loader.py` + `settings.py` | `handlers/config.ts` | `yaml` npm + `zod` for validation |
| `quality/audio_validation.py` | `child_process` → `ffprobe` | Subprocess wrapper port |
| `quality/replaygain.py` | Deferred | Subprocess wrapper port |
| `quality/lrc.py` | `handlers/lyrics.ts` | Encoding detection + UTF-8 conversion port |
| `quality/metadata_validation.py` | `state/AppState.ts` | Validation logic port |
| `ui/render_cover.py` | `CoverArt.tsx` + `sharp` | Port rendering logic |
