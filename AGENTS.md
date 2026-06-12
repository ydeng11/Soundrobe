# Auto Tagger — Agent Guide

## Project Overview

Auto Tagger is a desktop **Electron + React** app for intelligent audio file tagging. It automates metadata tagging for Navidrome-oriented music libraries using MusicBrainz, Discogs, LLM assistance (via OpenRouter), and local dataset lookups. The app provides a full GUI with editing, batch operations, and audit capabilities.

**Version:** 0.1.0  
**License:** MIT  
**Entry point:** `frontend/` — Electron app (TypeScript + React)

> ⚠️ A separate **Python CLI** (`src/auto_tagger/`) also exists as a legacy implementation. It is **not maintained** and should be ignored. All active development is on the Electron app.

---

## Project Structure

```
auto_tagger/
├── .env                      # Local env vars (loaded by just's dotenv-load)
├── .planning/                # Project planning & roadmap
├── AGENTS.md                 # This file — agent orientation guide
├── Justfile                  # Development task runner (just)
├── PLAN.md                   # Active plan
├── README.md
├── plans/
├── docs/
├── config.example.yaml
└── frontend/                 # ★ PRIMARY: Electron desktop app
    ├── package.json
    ├── tsconfig.json
    ├── vite.config.ts
    ├── tailwind.config.ts
    ├── playwright.config.ts
    ├── electron/             # Main process (Node.js)
    │   ├── main.ts           # Entry, IPC registration
    │   ├── preload.ts        # Context bridge (api.*)
    │   ├── better-sqlite3.d.ts
    │   ├── handlers/         # IPC handlers + business logic
    │   │   ├── aliases.ts
    │   │   ├── assistant.ts
    │   │   ├── audit.ts
    │   │   ├── auto-tag.ts      # ★ Central orchestrator
    │   │   ├── cache.ts
    │   │   ├── candidates.ts
    │   │   ├── conversation-logger.ts
    │   │   ├── cover.ts
    │   │   ├── dataset.ts
    │   │   ├── debug.ts
    │   │   ├── directory.ts
    │   │   ├── discogs.ts
    │   │   ├── fallback.ts
    │   │   ├── library.ts
    │   │   ├── lyrics.ts
    │   │   ├── musicbrainz.ts
    │   │   ├── native-check.ts
    │   │   ├── openrouter.ts
    │   │   ├── organizer.ts
    │   │   ├── prompts.ts
    │   │   ├── schemas.ts
    │   │   ├── tracks.ts
    │   │   └── writer.ts
    │   └── services/         # Pure logic (no Electron APIs)
    │       ├── ArtworkResolverService.ts
    │       ├── AssistantRuntime.ts
    │       ├── AssistantToolRegistry.ts
    │       ├── ConvertService.ts
    │       ├── ExtraTagService.ts
    │       ├── FilenameTagInferenceService.ts
    │       ├── FolderOrganizerService.ts
    │       ├── LibraryService.ts
    │       ├── LlmTaskRunner.ts
    │       ├── PlanExecutor.ts
    │       ├── SafeApiRequestService.ts
    │       ├── SafeQueryService.ts
    │       └── TrackTagService.ts
    ├── src/                  # Renderer process (React UI)
    │   ├── main.tsx
    │   ├── App.tsx
    │   ├── index.css
    │   ├── global.d.ts
    │   ├── vite-env.d.ts
    │   ├── utils/
    │   │   └── path.ts
    │   ├── components/
    │   │   ├── AssistantPanel.tsx
    │   │   ├── AuditBanner.tsx
    │   │   ├── AuditPanel.tsx
    │   │   ├── BatchEditor.tsx
    │   │   ├── BatchExtraTagsEditor.tsx
    │   │   ├── ConvertDialog.tsx
    │   │   ├── ErrorBoundary.tsx
    │   │   ├── ExtraTagsEditor.tsx
    │   │   ├── FileGrid.tsx
    │   │   ├── FolderTree.tsx
    │   │   ├── MetadataEditor.tsx
    │   │   ├── ScanProgressBar.tsx
    │   │   ├── SettingsModal.tsx
    │   │   ├── Sidebar.tsx
    │   │   └── TitleBar.tsx
    │   └── state/
    │       ├── AppState.ts
    │       └── UndoManager.ts
    ├── test/
    │   ├── components/
    │   ├── handlers/
    │   ├── services/
    │   ├── state/
    │   ├── integration/
    │   ├── helpers/
    │   └── utils/
    └── e2e/
        ├── convert.electron.spec.ts
        └── extra-tags.electron.spec.ts
```

---

## Tech Stack

| Category        | Technology                                                  |
|-----------------|-------------------------------------------------------------|
| Platform        | **Electron** (desktop app — macOS, Windows, Linux)          |
| Language        | **TypeScript** (strict)                                     |
| UI Framework    | **React 19** + **Tailwind CSS**                             |
| Build           | **Vite** + **electron-builder**                             |
| Test Runner     | **Vitest** (unit/integration), **Playwright** (E2E)         |
| Tag Reading     | **music-metadata** (parse tags from audio files)            |
| Tag Writing     | **node-id3** (ID3v2), **music-metadata** / native writers   |
| Image Processing| **sharp** (cover art resize/convert)                        |
| Storage         | **better-sqlite3** (cache, dataset index)                   |
| Chinese Tools   | **opencc-js** (Simplified/Traditional Chinese conversion)   |
| External APIs   | **MusicBrainz** (MBID lookup), **Discogs** (genre/cover)    |
| LLM Provider    | **OpenRouter** (any model — configurable via settings)      |
| Code Quality    | **TypeScript** strict mode, **Vitest** for tests            |

---

## Available Recipes (just)

Prerequisite: Install [just](https://github.com/casey/just) (`brew install just`) and run `npm install` in `frontend/`.

> **`.env` loading**: All `just fe-*` recipes automatically load `.env` from the project root
> via `set dotenv-load` in the Justfile. `LLM_API_KEY`, `LLM_MODEL`, and other env vars
> are exported into the app's environment without manual `export`.

### Development

| Recipe                      | Description                                       |
|-----------------------------|---------------------------------------------------|
| `just fe-dev`               | Start dev server (Vite HMR + Electron hot-reload) |
| `just fe-install`           | Install frontend dependencies (`npm install`)      |
| `just fe-build`             | Build for production (`tsc && vite build`)         |
| `just fe-test`              | Run all tests (`vitest run`)                      |
| `just fe-typecheck`         | TypeScript type checker (`tsc --noEmit`)          |
| `just fe-check`             | Typecheck + test (full quality gate)              |

### Distribution

| Recipe                      | Description                                       |
|-----------------------------|---------------------------------------------------|
| `just fe-dist mac`          | Build macOS `.dmg` distributable                  |
| `just fe-dist win`          | Build Windows `.exe` distributable                |
| `just fe-dist linux`        | Build Linux AppImage distributable                |
| `just fe-rebuild-native`    | Rebuild native modules for Electron's ABI         |

### Dataset (one-time setup — requires Python venv)

| Recipe                      | Description                                       |
|-----------------------------|---------------------------------------------------|
| `just dataset-status`       | Check local dataset index status                  |
| `just dataset-plan`         | Preview dataset setup plan without downloading    |
| `just dataset-setup`        | Download dataset and build local SQLite index     |

---

## Logging & Debugging

### Debug Logs (Electron App)

The debug logger writes timestamped JSON entries to :

```
~/.auto-tagger/auto-tag-debug-YYYY-MM-DD.log
```

Each log entry contains `timestamp`, `tag`, `level` (info/warn/error/debug), `message`, and optional `data`. Tags include `auto-tag`, `config`, `cache`, `dataset`, `musicbrainz`, `discogs`, `cover`, `timer`, and `debug`.

The file is truncated at each app session start. The logger lives in `frontend/electron/handlers/debug.ts`.

### Auto-Tagger General Log

```
~/.auto-tagger/auto-tagger.log
```

### Enabling Debug Mode

- **Via Settings UI**: Toggle "Debug mode" in the app settings
- **Via env var** (before launch): `AUTO_TAG_DEBUG=true`
- **Via config file**: Add `debug: true` to `~/.auto-tagger/config.yaml`

When debug mode is enabled, live log entries are forwarded to the renderer's DevTools console via IPC (`debug:log` channel).

### Agent Session

When troubleshooting an active app session:

1. **Check running processes**: `ps aux | grep -i electron | grep -v Helper | grep -v grep` — look for the main `Electron .` process
2. **Check logs**: Tail the debug log for the current day: `tail -f ~/.auto-tagger/auto-tag-debug-$(date +%F).log`
3. **Check app config**: `~/.auto-tagger/config.yaml` — library path, API keys, feature toggles
4. **Check window state**: `~/.auto-tagger/window-state.json` — last window position/size
5. **Check cache DB**: `~/.auto-tagger/cache.db` — SQLite database with three tables:
   - `lookup_cache` — MusicBrainz/Discogs lookup results by query hash
   - `album_state` — Per-album processing status (pending, llm_parsed, tagged_ok, error) + LLM extraction results
   - `conversation_log` — AI assistant conversation history (user messages, assistant responses, API calls, tool calls)

   Query recent assistant sessions:
   ```bash
   sqlite3 ~/.auto-tagger/cache.db "SELECT session_number, entry_count, firstMessage, lastActivity, totalCost FROM (
     SELECT session_number, COUNT(*) as entry_count,
       (SELECT content FROM conversation_log cl2 WHERE cl2.session_uuid = cl.session_uuid AND cl2.entry_type = 'user_message' ORDER BY cl2.id ASC LIMIT 1) as firstMessage,
       MAX(timestamp) as lastActivity,
       COALESCE(SUM(cost), 0) as totalCost
     FROM conversation_log cl
     GROUP BY session_uuid
     ORDER BY MAX(id) DESC LIMIT 10
   );"
   ```

   Query album processing state:
   ```bash
   sqlite3 ~/.auto-tagger/cache.db "SELECT status, disc_count, error, processed_at FROM album_state ORDER BY processed_at DESC LIMIT 20;"
   ```

6. **Check user data dir**: `/Users/ihelio/Library/Application Support/auto-tagger/` — Electron's standard user data (session storage, local storage, preferences)
7. **Vite HMR status**: Verify the Vite dev server responds: `curl -s -o /dev/null -w "%{http_code}" http://localhost:5173/`

For log analysis, use `tail` or `grep` on the debug log to filter by level or tag:
```bash
grep '"level":"error"' ~/.auto-tagger/auto-tag-debug-$(date +%F).log
grep '"tag":"auto-tag"' ~/.auto-tagger/auto-tag-debug-$(date +%F).log | tail -30
```

---

## Testing

The primary test runner is **Vitest**, run via `just fe-test` or `cd frontend && npm test`.

```
just fe-test           # Run all 100+ tests
just fe-typecheck      # TypeScript type check only
just fe-check          # Both: typecheck + test
```

To run a specific test file:

```bash
cd frontend && npx vitest run test/handlers/auto-tag.test.ts
```

E2E tests use Playwright:

```bash
cd frontend && npx playwright test
```

---

## Rules

1. **Never edit Python files** (`src/auto_tagger/`, `tests/`, `pyproject.toml`). The CLI in `src/` is a legacy Python implementation — all active development is the Electron app in `frontend/`.

2. **Tagging pipeline immutability**: folder hints → MusicBrainz → Discogs → LLM. Each step sets fields only if not already set. Never overwrites.

3. **LLM cost target**: under $0.01/album. Prefer free-tier OpenRouter models.

5. **IPC boundary**: `handlers/` receive IPC calls and wire services. `services/` contain pure business logic (no Electron APIs, testable in plain Node).

6. **`.env` is local-only**: never loaded by app code. The Justfile's `dotenv-load` exports it before running `npm`. Tests manage env vars explicitly.

## Key Design Decisions

- **Tagging pipeline**: folder hints → MusicBrainz lookup → Discogs lookup → LLM fallback (each step sets fields; never overwrites once set)
- **Config path priority**: Electron's `getConfigPaths()` checks `~/.auto-tagger/config.yaml` before `~/.config/auto-tagger/config.yaml`
- **LLM cost target**: Under $0.01 per album (uses cost-efficient models via OpenRouter)
- **Services vs handlers**: `electron/services/` contains pure business logic (no Electron APIs, testable in Node). `electron/handlers/` wires services to IPC channels.
- **Never edit Python files** (`src/auto_tagger/`, `tests/`, `pyproject.toml`) — those are the legacy CLI; only the TypeScript Electron app (`frontend/`) is maintained

---

## Metadata Handling

The app reads audio file metadata using the **[music-metadata](https://github.com/Borewit/music-metadata)** library (`parseFile` from `music-metadata`).

### Tag normalization

`music-metadata` normalizes format-specific tag keys into a `common` object with consistent property names regardless of the source format:

| Vorbis Comment (FLAC/OGG) | ID3 (MP3) | `common` property |
|---|---|---|
| `ARTIST` | `TPE1` | `common.artist` |
| `ALBUM` | `TALB` | `common.album` |
| `TITLE` | `TIT2` | `common.title` |
| `DATE` / `YEAR` | `TDRC` / `TYER` | `common.year` |
| `GENRE` | `TCON` | `common.genre` |
| `ALBUMARTIST` / `album_artist` | `TPE2` | `common.albumartist` |
| `TRACKNUMBER` / `track` | `TRCK` | `common.track.no` |
| `DISCNUMBER` / `disc` | `TPOS` | `common.disc.no` |
| `COMPOSER` | `TCOM` | `common.composer` |
| `LYRICS` | `USLT` | `common.lyrics` |
| `ARTISTS` | `TSOP` | `common.artists` (array) |
| `MUSICBRAINZ_ALBUMID` | `TXXX:MusicBrainz Album Id` | `common.musicbrainz_albumid` |
| `MUSICBRAINZ_ARTISTID` | `TXXX:MusicBrainz Artist Id` | `common.musicbrainz_artistid` |
| `MUSICBRAINZ_TRACKID` | `UFID:http://musicbrainz.org` | `common.musicbrainz_trackid` |

**Key consequence for agent troubleshooting**: When searching for metadata with `ffprobe`, Vorbis comment keys are **uppercase** (`ARTIST`, `ALBUM`, `TITLE`). The `music-metadata` library normalizes them to lowercase `common` properties. The grep pattern `artist|album|title|genre` matches ID3 keys but **not** uppercase Vorbis keys. Use a **case-insensitive grep** or search for the uppercase keys directly when investigating FLAC/OGG files.

Example — FLAC metadata displayed by `ffprobe`:
```
TAG:ARTIST=F.I.R.飞儿乐团
TAG:ALBUM=无限
TAG:TITLE=千年之恋
```
These are read by `parseFile()` as `common.artist = "F.I.R.飞儿乐团"`, `common.album = "无限"`, `common.title = "千年之恋"`.

### Reading conventions in the codebase

| File | Function | How metadata is read |
|---|---|---|
| `handlers/cover.ts` | `readFirstTrackMetadata()` | `parseFile(filePath)` → `common.artist`, `common.album`, `common.musicbrainz_albumid` |
| `handlers/auto-tag.ts` | `parseAlbumWithTags()` | Uses `music-metadata` to extract all per-track fields including artist, title, track number, genre from each audio file |
| `handlers/tracks.ts` | `readTrackTags()` | Full tag reading for the metadata editor; accesses both `common` and `native` format-specific tag representations |
| `services/FilenameTagInferenceService.ts` | — | Infers tags from filenames (not from file metadata) |

### Cover / artwork resolution chain

The `ArtworkResolverService` (`services/ArtworkResolverService.ts`) resolves album covers and artist images by trying providers in fixed order:

**Album covers**: `local → cover-art-archive → discogs → theaudiodb → google`

1. **local**: Checks for `cover.jpg`, `Cover.jpg`, `front.jpg`, `folder.jpg`, etc. in the album directory. For artist images, checks `artist.jpg` in the parent folder.
2. **cover-art-archive**: Requires `musicbrainzAlbumId` from file metadata. Fetches from https://coverartarchive.org/release/{mbid}.
3. **discogs**: Requires both `artist` and `album` metadata. Searches Discogs database with `per_page=10` and scans candidates, validating that the returned release's artist and album actually match the requested ones (with normalization for Chinese variants, punctuation, containment, etc.). Only downloads the cover from the first valid match. **Warning**: Discogs search is unreliable for non-Latin scripts — Chinese queries often return incorrect results or the same unrelated release for multiple different albums. The validation rejects these mismatches. This is the primary source used when Discogs token is configured.
4. **theaudiodb**: Requires `theAudioDbApiKey` config. Skipped when key is missing.
5. **google**: Requires `googleApiKey` + `googleImageSearchEngineId` config. Skipped when missing.

**Artist images**: `local → wikimedia → google`

The `cover` tag is used in debug log filtering. Enable debug mode (`AUTO_TAG_DEBUG=true`) to trace each provider attempt, search query, and result via `grep '\"tag\":\"cover\"' ~/.auto-tagger/auto-tag-debug-*.log`.

