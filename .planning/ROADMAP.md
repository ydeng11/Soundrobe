# Roadmap - Auto Tagger

## Overview

This roadmap defines the implementation phases for Auto Tagger v1. Phases are organized by dependency and logical grouping.

---

## Phase Structure

### Wave 1: Foundation
Phases 1-2 establish core infrastructure and tagging capabilities.

### Wave 2: Integration
Phases 3-4 integrate beets and LLM for smart tagging.

### Wave 3: Quality & Polish
Phases 5-6 add validation, quality checks, and distribution.

---

## Phase 1: Project Setup & Core Architecture

**Goal**: Establish project structure, configuration, and basic CLI

**Dependencies**: None

**Tasks**:
- Set up Python project structure (src layout)
- Configure build system (pyproject.toml)
- Implement configuration management (Pydantic Settings)
- Create basic CLI with Typer
- Set up logging and error handling framework

**Success Criteria**:
- Project builds and installs successfully
- CLI entry point works (`auto-tagger --help`)
- Configuration file loads correctly
- Logging outputs to console

**Plans**:
- `plan-1-1`: Project structure and build config
- `plan-1-2`: Configuration system
- `plan-1-3`: CLI framework and logging

---

## Phase 2: Core Tagging Engine

**Goal**: Implement basic audio file reading/writing and metadata tagging

**Dependencies**: Phase 1

**Tasks**:
- Integrate mutagen for audio metadata handling
- Implement tag reader for ID3v2, Vorbis, MP4 formats
- Implement tag writer with proper format handling
- Support multi-valued artist tags
- Handle ReplayGain tag format

**Success Criteria**:
- Can read tags from MP3, FLAC, M4A files
- Can write tags to all supported formats
- Multi-valued tags work correctly
- ReplayGain tags stored in correct format

**Plans**:
- `plan-2-1`: Audio file abstraction layer
- `plan-2-2`: Tag reader implementation
- `plan-2-3`: Tag writer implementation
- `plan-2-4`: Multi-value and ReplayGain support

---

## Phase 3: Beets Integration

**Goal**: Integrate beets for MusicBrainz lookup and candidate retrieval

**Dependencies**: Phase 2

**Tasks**:
- Set up beets library integration (non-CLI)
- Implement MusicBrainz lookup via beets.autotag
- Implement candidate retrieval and parsing
- Add folder structure parsing fallback
- Implement match caching (SQLite)

**Success Criteria**:
- Can query MusicBrainz via beets for artist/album
- Returns multiple candidates with metadata
- Folder parsing extracts artist/album from path
- Cache stores and retrieves matches

**Plans**:
- `plan-3-1`: Beets library setup and config
- `plan-3-2`: MusicBrainz lookup implementation
- `plan-3-3`: Folder parsing fallback
- `plan-3-4`: Match caching system

---

## Phase 4: LLM Integration

**Goal**: Integrate OpenRouter for match selection and fallback tag generation

**Dependencies**: Phase 3

**Tasks**:
- Implement OpenRouter API client
- Create match selection prompts
- Implement fallback tag generation prompts
- Add cost tracking and optimization
- Add response validation and error handling

**Success Criteria**:
- OpenRouter API calls work with cost-efficient models
- LLM selects best candidate from beets results
- LLM generates reasonable tags from folder metadata
- Cost tracking reports per-album spend
- Structured output reduces hallucination

**Plans**:
- `plan-4-1`: OpenRouter client implementation
- `plan-4-2`: Match selection prompting
- `plan-4-3`: Fallback tag generation
- `plan-4-4`: Cost optimization and tracking

---

## Phase 5: Quality Assurance & Validation

**Goal**: Implement file validation, health checks, and reporting

**Dependencies**: Phase 2, Phase 3, Phase 4

**Tasks**:
- Implement corrupt file detection (ffprobe)
- Add LRC file validation and encoding fix
- Implement metadata validation (required fields)
- Create health report generation
- Add ReplayGain calculation (rgain3)

**Success Criteria**:
- Detects unplayable/truncated audio files
- Converts LRC files to UTF-8
- Validates metadata completeness
- Health report lists all issues per album
- ReplayGain values calculated and tagged

**Plans**:
- `plan-5-1`: Audio file validation
- `plan-5-2`: LRC encoding validation
- `plan-5-3`: Metadata validation
- `plan-5-4`: Health report generation
- `plan-5-5`: ReplayGain calculation

---

## Phase 6: Navidrome Features & Distribution

**Goal**: Add Navidrome-specific features, complete CLI, and prepare distribution

**Dependencies**: Phase 5

**Tasks**:
- Implement cover art fetching (MusicBrainz, folder)
- Add compilation album detection and tagging
- Implement lyrics embedding and LRC sync
- Complete batch mode with progress and parallel processing
- Add interactive mode with preview
- Create Homebrew formula
- Publish to PyPI

**Success Criteria**:
- Missing covers fetched and embedded
- Compilation albums tagged correctly
- Lyrics handled (embedded + LRC)
- Batch mode processes entire library
- Interactive mode shows preview
- Homebrew formula installs tool
- PyPI package published

**Plans**:
- `plan-6-1`: Cover art handling
- `plan-6-2`: Compilation detection
- `plan-6-3`: Lyrics integration
- `plan-6-4`: Batch mode completion
- `plan-6-5`: Interactive mode
- `plan-6-6`: Homebrew distribution
- `plan-6-7`: PyPI publishing

---

## Phase Execution Order

```
Phase 1 ──┐
          ├──> Phase 2 ──┐
                         ├──> Phase 3 ──┐
                                        ├──> Phase 4 ──┐
                                                       ├──> Phase 5 ──> Phase 6
```

**Parallelization**: Phases within same wave can run parallel if no direct dependency.

---

## Milestone: v1 Release

**Completion Criteria**:
- All REQ-IDs implemented
- All success criteria met
- Tests pass for all modules
- Homebrew formula tested
- Documentation complete

**Deliverables**:
- Working CLI tool
- Homebrew formula
- PyPI package
- User documentation
- README with usage examples