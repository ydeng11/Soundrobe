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

### 4. Tag I/O: taglib-ts (both read and write)
- **Read:** `TagLib.read(path)` → returns all tag fields, cover art, format info
- **Write:** `TagLib.write(path, fields)` → writes tags to file, same API for all formats
- Coverage: MP3 (ID3v2.3/2.4), FLAC, Ogg Vorbis, Opus, MP4/M4A, WAV, AIFF
- Native npm addon (napi-rs). Requires `@electron/rebuild` for production .app, or prebuilt binaries per platform via CI.
- One library, one API, matches mutagen's unified interface.

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
| **Library (tag reading, MusicBrainz client, etc.)** | Vitest | No Electron, pure Node.js tests. Mock `fetch()`. Test against fixture files for taglib-ts. |
| **React components** | Vitest + jsdom | Component rendering, form validation, state management |
| **IPC handlers** | Vitest + Electron mocking | Verify handler logic, mock `taglib-ts`, mock `fetch()` |
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

### Phase 1: Electron Scaffold + taglib-ts Read
- Initialize `frontend/` with Vite + React + Tailwind + Electron
- Install `taglib-ts`, wire up `@electron/rebuild` in `postinstall`
- Implement `main.ts`, `preload.ts`, IPC channels
- Build `handlers/library.ts` — scan library directory for audio files
- Build `handlers/tracks.ts` — `readAlbum()` using `taglib-ts`
- Build `handlers/cover.ts` — cover art discovery
- `handlers/library.ts` test with Vitest
- **Result:** Launch app, open library, see albums and tracks in a table

### Phase 2: React UI (album browser + track view, conversions, lyrics, Discogs)
- Build `AlbumBrowser.tsx` — sortable data table of albums
- Build `TrackTable.tsx` — per-album track list with sort, filter, multi-select
- Build `TagPanel.tsx` — metadata form with validation, `<keep>` support
- Build `CoverArt.tsx` — embedded/external cover display
- State management: `AppState.ts`, `UndoManager.ts`
- `api.writeTrack()` → `taglib-ts` writes tags to file
- Optimistic field edits with rollback

**Filename ↔ Tag conversion:**
- Add pattern editor UI (like MP3tag's "Convert" dialog): user enters a pattern like `$artist - $title` or `%track% %artist% - %title%`
- `File → Tag`: extract fields from filename using regex/pattern → populate tag fields → bulk write via `api.writeTracks()`
- `Tag → File`: rename files using tag fields → pattern parser → rename via `fs.rename()`
- Undoable: each conversion pushes a snapshot before applying
- Patterns support: `$artist`, `$title`, `$album`, `$track`, `$year`, `$genre`, `$composer`, custom separators, padding

**Lyrics / LRC:**
- Build `handlers/lyrics.ts` — discover `.lrc` files alongside audio files, detect encoding (UTF-8 vs legacy), convert to UTF-8, sync with track timing
- Build `handlers/lyrics.ts` — fetch lyrics from remote sources (if available)
- Add `lyrics` field to `TrackTable` and `TagPanel` — view/edit embedded lyrics
- Auto-detect and fix encoding on library scan (flag non-UTF-8 LRC files)

**Discogs artist artwork:**
- Build `handlers/discogs.ts` — fetch artist images from Discogs API using artist name
- Album browser shows artist `artist.jpg` when available (like Navidrome's artist page)
- Right-click artist column → "Fetch Artist Artwork" → downloads `artist.jpg` into artist directory
- Scan existing artist directories for `artist.jpg` on library open

- **Result:** MP3tag-like editor — browse, edit, undo, filename↔tag conversion, lyrics, artist artwork

### Phase 3: Auto-Tag (Local Dataset first, then MusicBrainz + LLM)
- Build `handlers/dataset.ts` — read the existing MusicMoveArr SQLite index at `~/.auto-tagger/` via `better-sqlite3`. Same schema as v1's dataset.
- **Lookup priority:** Local dataset → MusicBrainz API → LLM fallback generation.
  - **1st:** Query local SQLite dataset for album/track metadata (fast, offline, zero cost)
  - **2nd:** MusicBrainz API `fetch()` if dataset misses (rate limited: 1 req/sec)
  - **3rd:** LLM-generated tags from file context if both miss
- Port candidate ranking (from `candidates.py`) and prompt templates (from `llm/prompts.py`) to TypeScript
- OpenRouter LLM client — `fetch()` to OpenRouter API
- Task queue with polling: `autoTagAlbum()` → `getTaskProgress()` → `cancelTask()`
- **Result:** Right-click album → Auto-Tag → dataset hit (fast) or network lookup → results

### Phase 4: Polish + Packaging
- Settings modal — config editing (LLM key, model, etc.)
- File watching — poll on focus
- Window lifecycle: save/restore size, recent workspaces
- Electron Builder config for `.dmg`, `.exe`, `.AppImage`
- CI pipeline: `npm run test` + `npm run build` + upload artifacts
- `taglib-ts` prebuilt binary for Electron ABI in CI
- **Result:** Shippable .app

## Build Phases Summary

| Phase | Scope | Maturation |
|---|---|---|
| **1** | Electron scaffold + taglib-ts read | See music library, browse albums |
| **2** | React UI + taglib-ts write | Full MP3tag editor |
| **3** | Auto-tag (MusicBrainz + LLM) | Right-click auto-tag |
| **4** | Polish + packaging | Shippable .app |

## Feature Scope

| Feature | In v1 Python? | In v2 Electron? | Phase |
|---|---|---|---|
| Browse library by album | ✅ | ✅ | 1 |
| View track metadata in table | ✅ | ✅ | 1 |
| Edit individual fields | ✅ | ✅ | 2 |
| Multi-track batch edit (`<keep>`) | ✅ | ✅ | 2 |
| Cover art preview | ✅ | ✅ | 2 |
| Undo (session-only) | ✅ | ✅ | 2 |
| Sort by column | ✅ | ✅ | 2 |
| Filter by regex | ✅ | ✅ | 2 |
| Dark theme | ❌ | ✅ | 2 |
| Auto-tag via MusicBrainz + LLM | ✅ | ✅ | 3 |
| Filename → Tag conversion | ❌ | ✅ | 2 |
| Tag → Rename files | ❌ | ✅ | 2 |
| Auto-numbering wizard | ❌ | ❌ | Deferred |
| ReplayGain calculation | ✅ | ❌ | Deferred |
| Lyrics / LRC | ✅ | ✅ | 2 |
| LLM audit | ✅ | ❌ | Deferred |
| Health reports | ✅ | ❌ | Deferred |
| Clean junk tags | ✅ | ❌ | Deferred |
| Discogs artist artwork | ✅ | ✅ | 2 |
| Python CLI (`auto-tag batch`, etc.) | ✅ | ❌ (use v1) | Separate |

## Risks & Mitigations

| Risk | Mitigation |
|---|---|
| `taglib-ts` native addon breaks on Electron ABI mismatch | `@electron/rebuild` in postinstall. CI builds prebuilt binaries per Electron version. Tests read/write fixture files. |
| `taglib-ts` write support is buggy in some formats | Test against real fixture files for MP3, FLAC, M4A. Compare output with mutagen (v1). Start with read-only (Phase 1), add writes after read is verified (Phase 2). |
| MusicBrainz rate limiting (1 req/sec) with 50-track album | Sequential lookup with per-track timeout. Show progress in the task poller. Dataset hit skips network entirely. |
| LLM API key management (no env vars, no CLI) | Config file + settings form. Same as v1's `config.yaml`. |
| No headless/CI mode in v2 | v1 Python CLI is still available for scripting. v2 Electron app is for interactive desktop use. Dataset setup (`auto-tag dataset setup`) still requires Python CLI (run once, then v2 reads the same `.db`). |
| Native rebuild per platform/architecture/Electron-version matrix | Use `electron-builder` which handles this for common targets. Pin Electron version. Prebuild only for macOS ARM + Windows x64 + Linux x64. |

## Reference: Existing Python Code → TypeScript

| Python file | TypeScript equivalent | Strategy |
|---|---|---|
| `state.py` | `frontend/src/state/AppState.ts` | Direct port of types and reducer logic |
| `undo.py` | `frontend/src/state/UndoManager.ts` | Direct port of class structure |
| `core/audio.py` + `core/formats.py` + `core/metadata.py` | `taglib-ts` | Replaced entirely |
| `core/writer.py` | `taglib-ts` | Replaced entirely |
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
