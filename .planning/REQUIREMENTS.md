# Requirements - Auto Tagger v1

## Overview

This document defines the requirements for the Auto Tagger CLI tool v1 release. All features listed below are scoped for the initial release.

---

## REQ-CT: Core Tagging

### REQ-CT-001: Primary Metadata Fields
- **Description**: Tag audio files with essential metadata fields
- **Fields**: `artist`, `artists` (multi-valued), `album`, `album_artist`
- **Formats**: ID3v2.4 (MP3), Vorbis (FLAC), MP4 (M4A)
- **Priority**: HIGH
- **Validation**: Must support multi-valued artist tags using proper format conventions

### REQ-CT-002: Track Info Fields
- **Description**: Tag track-level information
- **Fields**: `track_number`, `disc_number`, `year`, `genre`
- **Format**: Track numbers as "N" or "N/TOTAL", disc numbers as "N" or "N/TOTAL"
- **Priority**: HIGH

### REQ-CT-003: MusicBrainz IDs
- **Description**: Store MusicBrainz identifiers for duplicate detection and future updates
- **Fields**: `musicbrainz_trackid`, `musicbrainz_albumid`, `musicbrainz_artistid`
- **Priority**: MEDIUM
- **Use Case**: Enables re-tagging with updated metadata, duplicate detection

### REQ-CT-004: Additional Artist Fields (Deferred to v2)
- **Fields**: `composer`, `lyricist`, `conductor`
- **Priority**: LOW
- **Reason**: Not critical for Navidrome core functionality

---

## REQ-ND: Navidrome Integration

### REQ-ND-001: ReplayGain Tags
- **Description**: Calculate and tag ReplayGain values for volume normalization
- **Fields**: `replaygain_track_gain`, `replaygain_track_peak`, `replaygain_album_gain`, `replaygain_album_peak`
- **Format**: Gain in dB (e.g., "-6.84 dB"), Peak as float (e.g., "0.987654")
- **Priority**: HIGH
- **Implementation**: Use rgain3 or loudgain for calculation

### REQ-ND-002: Lyrics/LRC Support
- **Description**: Handle both embedded lyrics and synchronized LRC files
- **Embedded**: USLT (ID3), LYRICS (Vorbis), ©lyr (MP4)
- **LRC Format**: `[mm:ss.xx]lyrics` timing, with metadata tags `[ar:]`, `[al:]`, `[ti:]`
- **Priority**: HIGH
- **Validation**: Convert non-UTF8 LRC files to UTF-8

### REQ-ND-003: Cover Art Handling
- **Description**: Fetch and embed missing cover art
- **Sources**: MusicBrainz cover art archive, folder images (cover.jpg, folder.jpg, etc.)
- **Embedding**: APIC (ID3), METADATA_BLOCK_PICTURE (Vorbis), covr (MP4)
- **Priority**: HIGH
- **Fallback**: If no cover found, leave untagged (no placeholder)

### REQ-ND-004: Compilation Albums
- **Description**: Detect and properly tag compilation albums
- **Detection**: Various Artists album, soundtrack, multiple artists per album
- **Tags**: `album_artist=Various Artists`, `compilation=1`
- **Priority**: HIGH

---

## REQ-BT: Beets Integration

### REQ-BT-001: MusicBrainz Lookup
- **Description**: Query MusicBrainz database via beets library
- **Implementation**: Use `beets.autotag.match_album()` and `match_track()`
- **Priority**: HIGH
- **Performance**: Respect MusicBrainz rate limits (1 req/sec per IP)

### REQ-BT-002: Candidate Selection
- **Description**: Retrieve multiple match candidates and select best via LLM
- **Process**: Get top N candidates from beets, send to LLM for selection
- **Priority**: HIGH
- **Fallback**: If no candidates, proceed to folder parsing

### REQ-BT-003: Folder Structure Fallback
- **Description**: Parse artist/album from folder structure when beets fails
- **Assumption**: Files organized as `/Artist/Album/Track.mp3`
- **Priority**: HIGH
- **Edge Cases**: Handle "Various Artists", soundtracks, unknown folders

### REQ-BT-004: Match Caching
- **Description**: Cache successful beets matches to avoid repeated lookups
- **Storage**: SQLite database in `.planning/cache.db`
- **Priority**: MEDIUM
- **Benefit**: Significant speed improvement for batch processing

---

## REQ-LM: LLM Integration

### REQ-LM-001: OpenRouter Client
- **Description**: Integrate OpenRouter API for LLM calls
- **Models**: Cost-efficient models (claude-3.5-haiku, gemini-flash-lite)
- **Priority**: HIGH
- **Configuration**: API key via config file or environment variable

### REQ-LM-002: Match Selection
- **Description**: Use LLM to choose best candidate from beets results
- **Input**: Artist/album hint + candidate metadata
- **Output**: Best match ID or "none" if all poor
- **Priority**: HIGH
- **Prompting**: Structured output to avoid hallucination

### REQ-LM-003: Fallback Tag Generation
- **Description**: Generate tags from file metadata when no match found
- **Input**: Folder name, file names, existing partial tags
- **Output**: Structured metadata (artist, album, title guesses)
- **Priority**: HIGH
- **Validation**: Check output against basic sanity rules

### REQ-LM-004: Cost Optimization
- **Description**: Minimize LLM API costs per album
- **Target**: Under $0.01 per album
- **Strategies**: 
  - Use cheapest viable models
  - Minimize prompt tokens
  - Cache common patterns
  - Batch requests where possible
- **Priority**: HIGH
- **Reporting**: Display cost summary after batch run

---

## REQ-QA: Quality Assurance

### REQ-QA-001: Corrupt File Detection
- **Description**: Detect unplayable or truncated audio files
- **Method**: ffprobe validation, check file integrity
- **Priority**: HIGH
- **Output**: Flag in health report, skip tagging

### REQ-QA-002: LRC Encoding Validation
- **Description**: Validate and fix LRC file encoding
- **Issues**: Non-UTF8 encoding, malformed timing tags
- **Fix**: Convert to UTF-8, validate format structure
- **Priority**: HIGH

### REQ-QA-003: Metadata Validation
- **Description**: Validate tagged metadata meets requirements
- **Checks**: Required fields present, valid track/disc numbers, consistent album metadata
- **Priority**: HIGH
- **Output**: Report issues per album

### REQ-QA-004: Health Report
- **Description**: Generate comprehensive health report for processed albums
- **Content**: Corrupt files, missing tags, encoding issues, cover status
- **Priority**: HIGH
- **Format**: Console output + optional JSON report file

---

## REQ-CL: CLI & Distribution

### REQ-CL-001: Batch Mode
- **Description**: Process entire music library in one run
- **Command**: `auto-tagger batch /path/to/library`
- **Priority**: HIGH
- **Features**: Progress bar, parallel processing, summary report

### REQ-CL-002: Single Album Mode
- **Description**: Tag a specific album path
- **Command**: `auto-tagger tag /path/to/Artist/Album`
- **Priority**: HIGH
- **Features**: Detailed output, preview changes

### REQ-CL-003: YOLO Mode
- **Description**: Auto-approve all tagging changes without preview
- **Flag**: `--yolo` or config setting
- **Priority**: HIGH
- **Use Case**: Initial bulk tagging of library

### REQ-CL-004: Interactive Mode
- **Description**: Preview changes and accept/reject per album
- **Flag**: `--interactive` (default if not YOLO)
- **Priority**: HIGH
- **Features**: Show before/after tags, accept/skip/edit options

### REQ-CL-005: Configuration
- **Description**: Manage tool configuration
- **Sources**: CLI args → env vars → YAML config file → defaults
- **Priority**: HIGH
- **Location**: `~/.config/auto-tagger/config.yaml`

### REQ-CL-006: Homebrew Distribution
- **Description**: Distribute via Homebrew for easy installation
- **Formula**: Python virtualenv with explicit dependencies
- **Priority**: HIGH
- **PyPI**: Also publish to PyPI for pip install

---

## Deferred to v2

- Audio fingerprinting (AcoustID)
- Cover art resizing/optimization
- Automatic folder organization
- Music player integration
- Cloud storage handling
- Additional artist fields (composer, lyricist, conductor)
- Undo/rollback functionality
- Plugin system

---

## Success Criteria

- Can process 1000+ album library under $10 LLM costs
- 95%+ accuracy on beets-matchable albums
- Graceful fallback for edge cases
- Single command execution for entire library
- All v1 requirements implemented and tested