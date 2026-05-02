# STATE.md - Auto Tagger Project Memory

## Project Status

**Current Phase**: Initialization Complete
**Last Updated**: 2026-05-02
**Next Action**: Run `/gsd:plan-phase 1` to start Phase 1: Project Setup & Core Architecture

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

### Current Phase
- Phase 1: Project Setup & Core Architecture
- Status: **PLANNED** (Ready for execution)
- Plan: `.planning/phases/phase-1-plan.md`
- Tasks: 26 atomic tasks across 3 waves

### Upcoming Phases
- Phase 2: Core Tagging Engine
- Phase 3: Beets Integration
- Phase 4: LLM Integration
- Phase 5: Quality Assurance & Validation
- Phase 6: Navidrome Features & Distribution

---

## Requirements Status

**Total v1 Requirements**: 26 REQ-IDs
**Implemented**: 0
**In Progress**: 0
**Pending**: 26

### Category Breakdown
- Core Tagging (REQ-CT): 3 active, 1 deferred
- Navidrome Integration (REQ-ND): 4 active
- Beets Integration (REQ-BT): 4 active
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