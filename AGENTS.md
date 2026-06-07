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

Each log entry contains `timestamp`, `tag`, `level` (info/warn/error/debug), `message`, and optional `data`. Tags include `auto-tag`, `config`, `cache`, `dataset`, `musicbrainz`, `discogs`, `timer`, and `debug`.

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

