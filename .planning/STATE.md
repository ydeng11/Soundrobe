# STATE.md - Auto Tagger Project Memory

## Project Status

**Current Phase**: Phase 4 Planned
**Last Updated**: 2026-05-09
**Next Action**: Run `/gsd:execute-phase 4` to start Phase 4: LLM Integration

---

## Project Overview

**Name**: Auto Tagger
**Type**: Python CLI tool
**Purpose**: Automate audio file tagging using beets + LLM (OpenRouter)
**Distribution**: Homebrew, PyPI

---

## Key Decisions

### Technical Stack
- **Language**: Python 3.11+
- **Audio Library**: mutagen (recommended)
- **CLI Framework**: Typer + Rich
- **LLM**: OpenRouter API (claude-3.5-haiku, gemini-flash-lite)
- **Metadata Source**: beets + MusicBrainz
- **ReplayGain**: rgain3
- **Configuration**: Pydantic Settings

### Architecture
- **Structure**: src layout with `cli.py`, `commands/`, `core/`, `config/`
- **Config Priority**: CLI args → env vars → YAML file → defaults
- **Caching**: SQLite for beets matches
- **Logging**: Rich console output

### Workflow
- **Mode**: YOLO (auto-approve)
- **Planning Depth**: Standard (5-8 phases)
- **Execution**: Parallel (independent plans run simultaneously)

---

## Progress Tracking

### Completed Phases
- [x] Phase 0: Project Initialization
- [x] Phase 1: Project Setup & Core Architecture (COMPLETE)
- [x] Phase 2: Core Tagging Engine (COMPLETE)
- [x] Phase 3: Beets Integration (COMPLETE)

### Current Phase
- Phase 4: LLM Integration
- Status: **PLANNED** (Ready for execution)
- Plans: plan-4-1, plan-4-2, plan-4-3, plan-4-4

### Upcoming Phases
- Phase 4: LLM Integration
- Phase 5: Quality Assurance & Validation
- Phase 6: Navidrome Features & Distribution

---

## Requirements Status

**Total v1 Requirements**: 26 REQ-IDs
**Implemented**: 7
**In Progress**: 1
**Pending**: 18

### Category Breakdown
- Core Tagging (REQ-CT): 3 implemented, 1 deferred
- Navidrome Integration (REQ-ND): 1 partial (ReplayGain tag read/write), 3 active
- Beets Integration (REQ-BT): 4 implemented
- LLM Integration (REQ-LM): 4 active
- Quality Assurance (REQ-QA): 4 active
- CLI & Distribution (REQ-CL): 6 active

---

## Research Findings Summary

### Stack Research
- mutagen: Best for multi-format metadata handling
- Typer: Modern CLI with type hints and Rich integration
- beets: Use as library via `beets.autotag` modules
- OpenRouter: Python SDK with async support

### Features Research
- Navidrome requires: artist, album_artist, album, title, track number
- Multi-artist: Use `ARTISTS`/`ALBUMARTISTS` tags (preferred over separators)
- ReplayGain: TXXX frames for ID3v2, field=value for Vorbis
- LRC: `[mm:ss.xx]lyrics` format, UTF-8 encoding critical

### Architecture Research
- src layout recommended for CLI tools
- Pydantic Settings for config management
- Homebrew: `virtualenv_install_with_resources` pattern
- Error codes: 0=success, 1-4=specific errors

### Pitfalls Research
- Beets: Rate limiting (1 req/sec/IP), database corruption risk
- Encoding: ID3v2.4 incompatible with Windows Media Player
- LLM: Hallucination risk, validate against MusicBrainz IDs
- Performance: Memory management critical for >10K files

---

## Cost Targets

**LLM Cost**: Under $0.01 per album
**Models**: 
- claude-3.5-haiku (accuracy/cost balance)
- gemini-flash-lite (fastest/cheapest)

**Optimization Strategies**:
- Minimize prompt tokens
- Cache common patterns
- Batch requests
- Use structured outputs

---

## Dependencies

### External Tools
- beets (installed via pip)
- ffmpeg/ffprobe (for validation)
- MusicBrainz database (via beets)

### Python Packages
- mutagen (metadata handling)
- typer[all] (CLI framework)
- rich (console output)
- pydantic-settings (configuration)
- openrouter (LLM client)
- rgain3 (ReplayGain calculation)
- sqlite3 (built-in for caching)

---

## Configuration

**Config File**: `~/.config/auto-tagger/config.yaml`
**Environment Variables**: AUTO_TAGGER_OPENROUTER_KEY, etc.
**CLI Flags**: --yolo, --interactive, --config, etc.

---

## Notes

- All v1 requirements scoped for initial release (no deferred features except non-critical)
- Folder structure assumption: `/Artist/Album/Track.mp3`
- Target: 1000+ albums under $10 LLM costs
- Success: 95%+ accuracy on beets-matchable albums

---

## Session History

### 2026-05-02: Project Initialization
- Created PROJECT.md with vision
- Set workflow: YOLO mode, standard depth, parallel execution
- Completed domain research (stack, features, architecture, pitfalls)
- Defined 26 v1 requirements across 6 categories
- Created roadmap with 6 phases
- Ready to start Phase 1

### 2026-05-02: Phase 1 Execution (COMPLETE)
- Wave 1.1: Project structure & build config (7 tasks)
  - Created src layout with auto_tagger package
  - Set up pyproject.toml with hatchling backend
  - Added Click, Rich, Pydantic, PyYAML dependencies
  - Created README, LICENSE, .gitignore
  - Installed package in development mode
- Wave 1.2: Configuration system (7 tasks)
  - Implemented custom exception hierarchy (5 error types)
  - Created Settings model with Pydantic Settings
  - Added YAML config loader with auto-discovery
  - Created example config files
  - Added configuration tests (7 test cases)
- Wave 1.3: CLI framework & logging (12 tasks)
  - Created logging module with Rich formatting
  - Added output formatting utilities
  - Implemented CLI with Click (tag, batch, config commands)
  - Added __main__.py entry point
  - Created stub implementations for commands
  - Added comprehensive CLI tests (12 test cases)
  - Fixed test bug (CliRunner argument issue)
  
**Phase 1 Metrics**:
- 26 tasks completed across 3 waves
- 19 tests passing (100% pass rate)
- 78% code coverage
- 7 atomic commits
- All success criteria met

**Verification Results**:
✓ Package installs successfully
✓ CLI entry point works (`auto-tag --help`)
✓ Configuration loads from YAML/env/CLI
✓ Logging outputs to console with Rich formatting
✓ All tests pass
✓ Coverage > 75%

Ready for Phase 2: Core Tagging Engine

### 2026-05-09: Phase 2 Planning
- Created `.planning/phases/phase-2-plan.md`
- Defined Phase 2 architecture target for `auto_tagger.core`
- Broke work into 4 waves:
  - Wave 2.1: Audio file abstraction layer
  - Wave 2.2: Metadata model and tag reader
  - Wave 2.3: Tag writer implementation
  - Wave 2.4: Multi-value and ReplayGain support
- Next action: execute Phase 2

### 2026-05-09: Phase 2 Execution (COMPLETE)
- Added `mutagen>=1.47.0` runtime dependency
- Implemented core audio file detection/discovery/loading
- Added normalized `TrackMetadata` and `ReplayGainTags` models
- Implemented metadata readers and writers for MP3/ID3, FLAC/Vorbis-style tags, and MP4/M4A atoms
- Added multi-valued artist and album artist mapping for Navidrome-compatible tags
- Added ReplayGain tag read/write support (calculation remains Phase 5)
- Updated `auto-tag tag --dry-run` to preview discovered file metadata
- Added Phase 2 unit coverage for audio discovery, metadata normalization, format mappings, reader, writer, and CLI preview

**Verification Results**:
✓ `ruff check src tests`
✓ `mypy src`
✓ `pytest --cov=auto_tagger` (34 passed, 83% coverage)

Ready for Phase 3: Beets Integration

### 2026-05-09: Phase 3 Planning
- Created `.planning/phases/phase-3-plan.md`
- Defined an isolated Beets integration boundary under `auto_tagger.integrations`
- Broke work into 4 waves:
  - Wave 3.1: Beets library setup and candidate models
  - Wave 3.2: MusicBrainz lookup implementation
  - Wave 3.3: Folder structure fallback
  - Wave 3.4: Match caching system
- Next action: execute Phase 3

### 2026-05-09: Phase 3 Execution (COMPLETE)
- Added `beets>=2.0.0` runtime dependency
- Implemented normalized lookup candidate models and JSON serialization
- Added isolated Beets client boundary with injectable lookup functions and rate limiting
- Implemented folder structure fallback candidates for `/Artist/Album` paths
- Implemented SQLite match cache at project-local `.planning/cache.db`
- Added lookup orchestration: cache -> Beets -> folder fallback
- Extended `auto-tag tag --dry-run` with lookup candidate preview
- Added Phase 3 unit coverage for candidate models, cache, folder fallback, Beets client, lookup service, and CLI integration

**Verification Results**:
✓ `ruff check src tests`
✓ `mypy src`
✓ `pytest --cov=auto_tagger` (48 passed, 84% coverage)
✓ `auto-tag tag "潘玮柏/2006-反转地球" --dry-run`

Ready for Phase 4: LLM Integration

### 2026-05-09: Phase 4 Planning
- Created `.planning/phases/phase-4-plan.md`
- Verified current OpenRouter API surface from official docs:
  - `/api/v1/chat/completions`
  - `response_format` structured-output support
  - `usage` token statistics in responses
- Defined an `auto_tagger.llm` layer for client, prompts, schemas, selection, fallback generation, and cost tracking
- Broke work into 4 waves:
  - Wave 4.1: OpenRouter client and cost models
  - Wave 4.2: Structured prompts and response schemas
  - Wave 4.3: Match selection service
  - Wave 4.4: Fallback tag generation and cost reporting
- Next action: execute Phase 4
