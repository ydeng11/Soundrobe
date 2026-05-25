# Auto Tagger — Agent Guide

## Project Overview

Auto Tagger is a Python CLI + TUI tool for intelligent audio file tagging. It automates metadata tagging for Navidrome-oriented music libraries using MusicBrainz, Beets, LLM assistance, and local dataset lookups. The tool handles everything from single-album tagging to full library batch processing, with a full terminal UI built on [Textual](https://textual.textualize.io/).

**Version:** 0.1.0  
**License:** MIT  
**Entry point:** `auto-tag` (installed) / `python -m auto_tagger` (development)

---

## Project Structure

```
auto_tagger/
├── .env.example              # Environment variable template
├── .gitignore
├── .planning/                # Project planning & roadmap
│   ├── PROJECT.md            # Vision, goals, milestones
│   ├── ROADMAP.md
│   ├── MILESTONES.md
│   ├── STATE.md              # Current state tracker
│   ├── phases/               # Phase breakdowns
│   ├── milestones/           # Milestone definitions
│   └── research/             # Technical research notes
├── AGENTS.md                 # This file — agent orientation guide
├── Justfile                  # Development task runner (just)
├── README.md
├── LICENSE
├── pyproject.toml            # Build config, deps, tool settings
├── config.example.yaml       # Example YAML configuration
├── docs/                     # Documentation
│   ├── HANDOFF.md
│   ├── dataset-handoff.md
│   ├── release-checklist.md
│   ├── PLAN_album_verification.md
│   └── plans/
├── packaging/                # Distribution packaging
│   └── homebrew/
│       └── auto-tagger.rb    # Homebrew formula template
├── dist/                     # Built distributions (wheel + sdist)
├── tests/                    # Test suite (pytest)
│   ├── conftest.py
│   ├── fixtures/
│   │   ├── __init__.py
│   │   └── factory.py        # Test fixture factory
│   ├── test_*.py             # ~45 test files
│   └── __init__.py
└── src/
    └── auto_tagger/
        ├── __init__.py       # Version info
        ├── __main__.py       # python -m auto_tagger entry
        ├── cli.py            # Click CLI definitions (tag, batch, config, dataset, version)
        ├── exceptions.py     # Custom exception hierarchy
        ├── commands/         # Command implementations
        │   ├── tag.py
        │   ├── batch.py
        │   ├── config_cmd.py
        │   └── dataset.py
        ├── config/           # Configuration loading
        │   ├── loader.py
        │   └── settings.py   # Pydantic settings model
        ├── core/             # Core audio file handling
        │   ├── audio.py      # Audio file I/O
        │   ├── formats.py    # Supported format definitions
        │   ├── metadata.py   # Metadata field handling
        │   ├── reader.py     # Tag reading
        │   └── writer.py     # Tag writing
        ├── features/         # Feature-specific logic
        │   ├── compilations.py  # Multi-artist compilation detection
        │   ├── cover_art.py     # Cover art fetching
        │   └── lyrics.py        # Lyrics fetching & LRC handling
        ├── integrations/     # External service integrations
        │   ├── beets_client.py   # Beets/MusicBrainz lookup
        │   ├── cache.py          # Local caching layer
        │   ├── candidates.py     # Candidate merging & ranking
        │   ├── dataset.py        # Dataset index (SQLite)
        │   ├── dataset_raw.py    # Raw dataset download/extract
        │   ├── discogs_client.py # Discogs API integration
        │   ├── fallback.py       # Lookup fallback chain
        │   └── lookup.py         # Central lookup coordinator
        ├── llm/              # LLM integration (OpenRouter)
        │   ├── client.py     # LLM API client
        │   ├── cost.py       # Cost tracking & budgeting
        │   ├── fallback.py   # LLM-specific fallback logic
        │   ├── prompts.py    # Prompt templates
        │   ├── schemas.py    # Structured output schemas
        │   ├── selection.py  # Candidate selection via LLM
        │   └── types.py      # LLM-related types
        ├── quality/          # Quality assurance
        │   ├── audio_validation.py   # Audio file integrity checks
        │   ├── health.py             # Health report generation
        │   ├── lrc.py                # LRC file validation
        │   ├── metadata_validation.py # Tag consistency checks
        │   └── replaygain.py         # ReplayGain calculation
        ├── utils/            # Utilities
        │   ├── logging.py    # Logging configuration
        │   └── output.py     # Rich console output
        ├── ui/               # Terminal UI (Textual framework)
        │   ├── app.py            # App entry point, key bindings, screen management
        │   ├── state.py          # In-memory state (AppState, AlbumData, TrackData, TrackAuditResult)
        │   ├── undo.py           # Undo stack (UndoManager, TrackSnapshot)
        │   ├── workflow.py       # Subprocess management for auto-tag & audit (JSON streaming)
        │   ├── render_cover.py   # Cover art as terminal coloured blocks (half-block Unicode)
        │   ├── screens/
        │   │   ├── main_screen.py    # Main layout: toolbar, tag panel, track table, status bar
        │   │   └── settings_screen.py # Settings modal (auto-audit, LLM model, output format)
        │   └── widgets/
        │       ├── tag_panel.py       # Metadata form fields + cover art preview with validation
        │       ├── track_table.py     # Album browser + per-album track DataTable
        │       ├── toolbar.py         # Action buttons (Open, Auto-Tag, Stop, Undo, Filter, Settings)
        │       └── status_bar.py      # Filter input + file statistics with debounce
        └── workflows/        # Orchestration
            ├── album.py      # Single album workflow
            ├── artist.py     # Artist-level workflow
            ├── batch.py      # Batch processing workflow
            └── interactive.py # Interactive prompting workflow
```

---

## Tech Stack

| Category        | Technology                                      |
|-----------------|-------------------------------------------------|
| Language        | Python 3.10+                                    |
| CLI Framework   | Click                                           |
| TUI Framework   | Textual                                         |
| Settings        | Pydantic + pydantic-settings                    |
| Format          | YAML (config), JSON (health reports, output)    |
| Metadata        | Mutagen (read/write tags), MusicBrainz (lookup) |
| External        | Beets 2.0+, Discogs API                         |
| LLM Provider    | OpenRouter API (Claude Haiku, etc.)             |
| Audio Validation| ffmpeg / ffprobe                                |
| ReplayGain      | rgain3 or loudgain                              |
| Data Processing | OpenCC (Chinese text conversion)                |
| HTTP Client     | HTTPx                                           |
| Image           | Pillow (cover art rendering in TUI)             |
| Output          | Rich (terminal tables, formatting)              |
| Quality         | ruff (lint), mypy (types), pytest (tests)       |
| Build           | Hatchling + build                               |
| Package Manager | pip (dev), uv (lock file)                       |
| Task Runner     | just                                            |

---

## Available Commands

### `just` commands (development)

Prerequisite: Install [just](https://github.com/casey/just) (`brew install just`) and set up a virtual env (`just venv`).

#### Setup

| Command       | Description                                           |
|---------------|-------------------------------------------------------|
| `just venv`   | Create `.venv` and install the package with dev deps  |
| `just install`| Re-install the package in editable mode               |

#### Quality

| Command                    | Description                            |
|----------------------------|----------------------------------------|
| `just lint`                | Run ruff linter on `src/` and `tests/` |
| `just lint-fix`            | Auto-fix lint issues                   |
| `just typecheck`           | Run mypy type checker on `src/`        |
| `just test`                | Run pytest with coverage               |
| `just test-file <path>`    | Run a specific test file               |
| `just test-match <pattern>`| Run tests matching a keyword pattern   |
| `just check-all`           | Run lint + typecheck + test            |

#### Build

| Command     | Description                              |
|-------------|------------------------------------------|
| `just build`| Build source distribution + wheel        |
| `just show` | Show installed package info (`pip show`) |

#### CLI

| Command                   | Description                                       |
|---------------------------|---------------------------------------------------|
| `just run <args>`         | Run the CLI with arbitrary arguments               |
| `just tag <path>`         | Tag a single album (dry-run)                       |
| `just batch <path>`       | Batch process a library (dry-run)                  |

#### Dataset

| Command              | Description                                      |
|----------------------|--------------------------------------------------|
| `just dataset-status`| Check local dataset index status                 |
| `just dataset-plan`  | Preview dataset setup plan without downloading   |
| `just dataset-setup` | Download dataset and build local SQLite index    |

#### Cleanup

| Command     | Description                                   |
|-------------|-----------------------------------------------|
| `just clean`| Remove build artifacts, caches, `__pycache__` |
| `just nuke` | Remove everything including `.venv`           |

### CLI commands (auto-tag)

| Command / Flag                                               | Description                                                |
|--------------------------------------------------------------|------------------------------------------------------------|
| `auto-tag tag <path>`                                        | Tag a single album/directory                               |
| `  --dry-run`                                                | Preview changes without applying                           |
| `  --yolo`                                                   | Auto-approve all changes                                   |
| `  --interactive`                                            | Prompt before applying album changes                       |
| `  --health-report <path>`                                   | Write album health report JSON to this path                |
| `auto-tag batch <path>`                                      | Batch process an entire music library                      |
| `  --dry-run`                                                | Preview changes without applying                           |
| `  --yolo`                                                   | Auto-approve all changes                                   |
| `  --interactive`                                            | Prompt before applying each album                          |
| `  --parallel / -j <N>`                                      | Number of parallel processes (default: 1)                  |
| `  --health-report <path>`                                   | Write combined health report JSON for all albums           |
| `auto-tag config [key] [value]`                              | View or modify configuration                               |
| `auto-tag dataset status`                                   | Show local dataset setup status                            |
| `auto-tag dataset setup`                                    | Download dataset and build SQLite index                    |
| `  --service <name>`                                         | Dataset service(s) to install (can repeat; choices: `musicbrainz`, `spotify`, `tidal`, `deezer`) |
| `  --dry-run`                                                | Show setup plan without downloading                        |
| `auto-tag dataset build`                                    | Build SQLite index from already-staged dataset files       |
| `  --service <name>`                                         | Service(s) to index (can repeat; choices: `musicbrainz`, `spotify`, `tidal`, `deezer`) |
| `auto-tag clean <path>`                                      | Strip junk tags (description, comment, c) from audio files |
| `  --dry-run`                                                | Preview junk tags that would be removed                    |
| `auto-tag ui [path]`                                         | Launch the terminal UI for browsing and editing tags       |
| `auto-tag version`                                          | Show version information                                   |
| `auto-tag --help`                                            | Show full help                                             |

The `ui` subcommand has an optional `[path]` argument pointing to a music library directory.
It requires the `[ui]` extra: `pip install auto-tagger[ui]`.

Global CLI flags (before subcommand):

- `--config / -c <path>` — Path to YAML config file
- `--verbose / -v` — Enable verbose logging
- `--output / -o <format>` — Output format: `table`, `json`, or `plain`

---

## Configuration

Configuration is loaded (in priority order):
1. CLI flags
2. Environment variables (`AUTO_TAG_*`)
3. YAML config file (`~/.config/auto-tagger/config.yaml` or `./auto-tagger.yaml`)
4. Defaults

### Key environment variables

| Variable                      | Default                          | Description                  |
|-------------------------------|----------------------------------|------------------------------|
| `AUTO_TAG_LLM_API_KEY`        | —                                | OpenRouter API key           |
| `AUTO_TAG_LLM_ENDPOINT`       | `https://openrouter.ai/api/v1`   | LLM API endpoint             |
| `AUTO_TAG_LLM_MODEL`          | `deepseek/deepseek-v4-flash:free` | LLM model                    |
| `AUTO_TAG_OUTPUT_FORMAT`      | `table`                          | Output format                |
| `AUTO_TAG_VERBOSE`            | `false`                          | Verbose logging              |
| `AUTO_TAG_DATA_DIR`           | `~/.auto-tagger`                 | Data directory (datasets)    |
| `AUTO_TAG_LOG_PATH`           | `~/.auto-tagger/auto-tagger.log` | Log file path                |
| `AUTO_TAG_FFPROBE_PATH`       | `ffprobe`                        | Path to ffprobe binary       |
| `AUTO_TAG_REPLAYGAIN_COMMAND` | `rgain3`                         | ReplayGain command           |

---

## Testing

The test suite uses **pytest** with ~45 test files covering all major subsystems.

```
just test              # Run all tests with coverage
just test-file tests/test_cli.py  # Run a specific file
just test-match test_tag         # Run tests matching "test_tag"
```

**Test markers** (used via `pytest -m <marker>`):

| Marker             | Description                                  |
|--------------------|----------------------------------------------|
| `needs_ffmpeg`     | Tests requiring ffmpeg/ffprobe               |
| `needs_beets`      | Tests requiring the beets CLI                |
| `needs_rgain`      | Tests requiring rgain3 or loudgain           |
| `network`          | Tests requiring network access               |

---

## Common Workflows

### Quick start for development

```bash
just venv              # Set up virtual env
just lint              # Check code style
just typecheck         # Check types
just test              # Run tests
```

### Tagging an album

```bash
# Development
just run tag /path/to/Artist/Album --dry-run

# Installed
auto-tag tag /path/to/Artist/Album --interactive
```

### Full library batch

```bash
auto-tag batch /path/to/library --dry-run --parallel 4
```

### Dataset setup

```bash
auto-tag dataset setup --dry-run   # Preview
auto-tag dataset setup             # Execute
```

### Release

See `docs/release-checklist.md` for the full release process.

---

## Key Design Decisions

- **Fallback chain**: Beets/MusicBrainz lookup → folder-name parsing → LLM-generated tags
- **LLM cost target**: Under $0.01 per album (uses cost-efficient models via OpenRouter)
- **Zero-interaction goal**: Entire library can be tagged without prompts (use `--yolo`)
- **Local dataset**: MusicMoveArr SQLite index provides faster lookups and offline capability
- **Health reports**: Machine-readable JSON output for tracking tagging quality over time
- **Terminal UI**: Full interactive editor built on [Textual](https://textual.textualize.io/) with two-panel layout (album browser + per-album track view), live metadata editing with immediate disk writes, embedded undo stack, JSON-streaming subprocess for async auto-tag + audit, cover art preview as coloured terminal blocks (via Pillow + Unicode half-blocks), right-click cover context menu, multi-track batch editing with `<keep>` placeholders, field validation with audit flag overlays, debounced regex filter, and sortable DataTable views
