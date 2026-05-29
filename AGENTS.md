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
├── .planning/                # Project planning & roadmap (outdated — refer to plans/)
├── AGENTS.md                 # This file — agent orientation guide
├── Justfile                  # Development task runner (just)
├── PLAN.md                   # Active plan
├── README.md
├── plans/                    # Active plans and designs
│   └── llm-tag-parsing-redesign.md
├── docs/                     # Documentation
│   ├── HANDOFF.md
│   ├── dataset-handoff.md
│   └── plans/
├── config.example.yaml       # Example YAML config (shared between Python & Electron)
└── frontend/                 # ★ PRIMARY: Electron desktop app
    ├── package.json          # Dependencies & scripts
    ├── tsconfig.json         # TypeScript config
    ├── vite.config.ts        # Vite bundler config
    ├── tailwind.config.ts    # Tailwind CSS config
    ├── playwright.config.ts  # E2E test config
    ├── electron/             # Main process (Node.js backend)
    │   ├── main.ts           # Electron main process entry
    │   ├── preload.ts        # Preload script (context bridge)
    │   └── handlers/         # Tagging pipeline handlers
    │       ├── aliases.ts       # Artist name aliases / variants
    │       ├── audit.ts         # Audit report generation
    │       ├── auto-tag.ts      # ★ Central orchestrator (processAlbum)
    │       ├── cache.ts         # SQLite-based request cache
    │       ├── candidates.ts    # Candidate types, merging, ranking
    │       ├── cover.ts         # Cover art embedding
    │       ├── dataset.ts       # Dataset index (SQLite) queries
    │       ├── debug.ts         # Debug logging for pipeline
    │       ├── directory.ts     # Directory scanning & structuring
    │       ├── discogs.ts       # Discogs API client
    │       ├── fallback.ts      # Path-based & track-hint fallback
    │       ├── library.ts       # Library scanning
    │       ├── musicbrainz.ts   # MusicBrainz / Beets client
    │       ├── native-check.ts  # Native module & binary checks
    │       ├── openrouter.ts    # LLM API client (OpenRouter)
    │       ├── prompts.ts       # LLM prompt templates
    │       ├── schemas.ts       # LLM structured output schemas
    │       ├── tracks.ts        # Track reading & writing
    │       └── writer.ts        # Tag writing
    ├── src/                  # Renderer process (React UI)
    │   ├── main.tsx          # React entry
    │   ├── App.tsx           # Root component
    │   ├── index.css         # Tailwind + custom styles
    │   ├── global.d.ts       # Global type declarations
    │   ├── vite-env.d.ts     # Vite type shims
    │   ├── components/       # UI components
    │   │   ├── AuditBanner.tsx
    │   │   ├── AuditPanel.tsx
    │   │   ├── BatchEditor.tsx
    │   │   ├── BatchExtraTagsEditor.tsx
    │   │   ├── ConvertDialog.tsx
    │   │   ├── ExtraTagsEditor.tsx
    │   │   ├── FileGrid.tsx
    │   │   ├── FolderTree.tsx
    │   │   ├── MetadataEditor.tsx
    │   │   ├── ScanProgressBar.tsx
    │   │   ├── SettingsModal.tsx
    │   │   ├── Sidebar.tsx
    │   │   └── TitleBar.tsx
    │   └── state/            # State management
    │       ├── AppState.ts       # Central app state & reducer
    │       └── UndoManager.ts    # Undo/redo for tag edits
    ├── test/                 # Test suite (Vitest + Testing Library)
    │   ├── components/       # Component tests
    │   │   ├── BatchEditor.test.tsx
    │   │   ├── BatchExtraTagsEditor.test.tsx
    │   │   ├── ExtraTagsEditor.test.tsx
    │   │   ├── FileGrid.test.tsx
    │   │   ├── MetadataEditor.test.tsx
    │   │   ├── SettingsModal.test.tsx
    │   │   └── TitleBar.test.tsx
    │   ├── handlers/         # Handler unit tests
    │   │   ├── aliases.test.ts
    │   │   ├── audit.test.ts
    │   │   ├── auto-tag.test.ts
    │   │   ├── cache.test.ts
    │   │   ├── candidates.test.ts
    │   │   ├── config.test.ts
    │   │   ├── cover.test.ts
    │   │   ├── dataset.test.ts
    │   │   ├── debug.test.ts
    │   │   ├── directory.test.ts
    │   │   ├── discogs.test.ts
    │   │   ├── fallback.test.ts
    │   │   ├── library.test.ts
    │   │   ├── musicbrainz.test.ts
    │   │   ├── native-check.test.ts
    │   │   ├── openrouter.test.ts
    │   │   ├── prompts.test.ts
    │   │   ├── tracks.test.ts
    │   │   └── writer.test.ts
    │   └── state/            # State tests
    │       ├── app-reducer.test.ts
    │       └── undo-manager.test.ts
    └── e2e/                  # E2E tests (Playwright)
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

## Configuration

Settings are managed through the **Electron UI** (Settings modal) and persisted to `~/.auto-tagger/config.yaml`.

Key config fields the agent may reference:

| Field              | Default                           | Description                        |
|--------------------|-----------------------------------|------------------------------------|
| `llm_api_key`      | —                                 | OpenRouter API key                 |
| `llm_model`        | `openrouter/owl-alpha`            | LLM model for tag generation       |
| `discogs_token`    | —                                 | Discogs personal access token      |
| `discogs_enabled`  | `true`                            | Enable Discogs genre enrichment    |
| `remote_lookup_enabled` | `true`                       | Enable MusicBrainz + Discogs APIs  |

Config is loaded from (in priority order): CLI flags → environment variables → `~/.auto-tagger/config.yaml` (also checks `~/.config/auto-tagger/config.yaml` and `./auto-tagger.yaml`).

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

## Key Design Decisions

- **Tagging pipeline**: folder hints → MusicBrainz lookup → Discogs lookup → LLM fallback (each step sets fields; never overwrites once set)
- **Config path priority**: Electron's `getConfigPaths()` checks `~/.auto-tagger/config.yaml` before `~/.config/auto-tagger/config.yaml` (opposite of Python CLI — see `config.ts`)
- **LLM cost target**: Under $0.01 per album (uses cost-efficient models via OpenRouter)
- **Read files up to 50KB / 2000 lines**: output truncation limit — use offset/limit for larger files
- **Never edit Python files** (`src/auto_tagger/`, `tests/`, `pyproject.toml`) — those are the legacy CLI; only the TypeScript Electron app (`frontend/`) is maintained

---

## Common Workflows

### Quick start

```bash
cd frontend && npm install
just fe-check       # Run tests + typecheck
just fe-dev         # Start Electron dev server
```

### Tag an album (via app)

1. Open the app (`just fe-dev`)
2. Select an album folder in the sidebar
3. Click "Auto Tag" — the pipeline runs MusicBrainz → Discogs → LLM
4. Review candidate and apply tags
