# Auto Tagger

## Vision

A Python CLI tool that automates audio file tagging using beets and LLM (via OpenRouter). Eliminates the tedious manual work of tagging while handling edge cases where beets struggles.

## Problem Statement

Manual audio tagging is tedious. Beets helps but has pain points:
- Interactive import prompts slow down workflow
- Search/lookup can be slow
- Wrong matches require manual correction
- Some releases aren't in MusicBrainz databases

## Solution

Automated tagging pipeline with intelligent fallback chain:
1. **Beets lookup** - Query MusicBrainz for metadata
2. **LLM match selection** - When beets returns multiple candidates, LLM picks the best match
3. **Fallback parsing** - Extract artist/album from folder structure when beets fails
4. **LLM tag generation** - Create tags from file metadata when no match found

## Core Value

**Zero-interaction tagging with high accuracy.** The system should handle an entire music library without user intervention while maintaining correct metadata.

## Target Users

- Music collectors with large libraries
- Navidrome/Open Sonic server owners
- People who value organized, properly tagged music libraries

## Key Features

### Metadata Tagging
- Primary tags: `artist`, `artists` (multi-artist), `album`, `album_artist`
- Navidrome/Open Sonic fields: ReplayGain, lyrics, synchronized LRC files
- Smart multi-artist detection and tagging

### Quality Assurance
- Corrupt file detection (unplayable/truncated audio)
- Missing cover art detection and fetching
- LRC file encoding validation (UTF-8 conversion)

### Workflow Modes
- **Batch mode**: Process entire library at once
- **Single album mode**: Tag one album at a time

### Cost Efficiency
- Target: under $0.01 per album for LLM calls
- Uses cost-efficient models via OpenRouter
- Minimizes token usage through smart prompting

### Distribution
- Homebrew formula for easy installation
- Python package with CLI entry point

## Technical Approach

### Stack
- **Language**: Python 3.11+
- **Metadata source**: beets + MusicBrainz
- **LLM**: OpenRouter API (cost-efficient models)
- **Audio processing**: mutagen, ffmpeg
- **Distribution**: Homebrew, PyPI

### Fallback Priority
1. Beets database lookup
2. File path parsing (`/Artist/Album/Track.mp3`)
3. LLM-based tag generation from available metadata

### Assumptions
- Audio files are already organized by `Artist/Album/` folder structure
- User has OpenRouter API access
- Target system is Navidrome/Open Sonic compatible

## Success Metrics

- Can tag 1000+ album library under $10 in LLM costs
- 95%+ accuracy on beets-matchable albums
- Graceful degradation for edge cases
- Single command execution for entire library

## Out of Scope (v1)

- Audio fingerprinting (AcoustID)
- Album art resizing/optimization
- Automatic folder organization
- Music player integration
- Cloud storage handling