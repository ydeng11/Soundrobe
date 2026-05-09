# Phase 2 Execution Plan: Core Tagging Engine

## Overview

**Goal**: Implement basic audio file reading/writing and metadata tagging

**Duration**: ~4-6 hours

**Dependencies**: Phase 1 complete

**Primary Requirements**:
- REQ-CT-001: Primary Metadata Fields
- REQ-CT-002: Track Info Fields
- REQ-CT-003: MusicBrainz IDs
- REQ-ND-001: ReplayGain Tags

**Success Criteria**:
- Can detect supported audio files: MP3, FLAC, M4A
- Can read normalized metadata from ID3v2, Vorbis comments, and MP4 atoms
- Can write normalized metadata to all supported formats
- Multi-valued artist tags preserve proper per-format conventions
- ReplayGain tags are stored in the expected per-format fields
- Unit tests cover format mapping, reader/writer behavior, and error handling

---

## Architecture Target

Phase 2 creates the `auto_tagger.core` layer. The CLI should not manipulate mutagen objects directly; commands call a small service API that returns normalized project models.

**New modules**:
```
src/auto_tagger/core/
  audio.py          # AudioFile abstraction and supported-file detection
  metadata.py       # Normalized metadata and ReplayGain models
  reader.py         # Read tags from files into normalized metadata
  writer.py         # Write normalized metadata back to files
  formats.py        # Format-specific tag mapping helpers
```

**New tests**:
```
tests/test_audio.py
tests/test_metadata.py
tests/test_reader.py
tests/test_writer.py
tests/fixtures/audio/
```

**Dependency update**:
- Add `mutagen>=1.47.0` to runtime dependencies.

---

## Wave 2.1: Audio File Abstraction Layer

**Objective**: Establish a safe, tested boundary around supported audio files and mutagen loading.

### Task 2.1.1: Add mutagen dependency

**Action**: Update `pyproject.toml` runtime dependencies with `mutagen>=1.47.0`.

**Verification**:
```bash
python -m pip install -e ".[dev]"
python -c "import mutagen; print(mutagen.version_string)"
```

---

### Task 2.1.2: Create normalized audio types

**Action**: Add `src/auto_tagger/core/audio.py`.

**Design**:
- `AudioFormat` enum: `MP3`, `FLAC`, `M4A`
- `SUPPORTED_EXTENSIONS`: `.mp3`, `.flac`, `.m4a`, `.mp4`
- `AudioFile` dataclass with `path`, `format`, `mutagen_file`
- `detect_audio_format(path: Path) -> AudioFormat`
- `load_audio_file(path: Path) -> AudioFile`
- Raise `FileProcessingError` for missing, unsupported, or unreadable files.

**Verification**:
```bash
pytest tests/test_audio.py
```

---

### Task 2.1.3: Add audio discovery helper

**Action**: Add helper for album paths.

**Design**:
- `iter_audio_files(path: Path, recursive: bool = False) -> list[Path]`
- Return sorted paths for deterministic output and tests.
- Skip unsupported extensions instead of raising.
- Raise `FileProcessingError` if no supported audio files are found.

**Verification**:
```bash
pytest tests/test_audio.py
```

---

## Wave 2.2: Metadata Model and Tag Reader

**Objective**: Read format-specific tags into a single normalized model.

### Task 2.2.1: Create metadata models

**Action**: Add `src/auto_tagger/core/metadata.py`.

**Design**:
- `ReplayGainTags` dataclass:
  - `track_gain`
  - `track_peak`
  - `album_gain`
  - `album_peak`
- `TrackMetadata` dataclass:
  - `title`
  - `artist`
  - `artists`
  - `album`
  - `album_artist`
  - `album_artists`
  - `track_number`
  - `track_total`
  - `disc_number`
  - `disc_total`
  - `year`
  - `genre`
  - `musicbrainz_trackid`
  - `musicbrainz_albumid`
  - `musicbrainz_artistid`
  - `replaygain`
- Helper methods for empty-value cleanup and track/disc number formatting.

**Verification**:
```bash
pytest tests/test_metadata.py
```

---

### Task 2.2.2: Implement per-format read mappings

**Action**: Add read helpers in `src/auto_tagger/core/formats.py`.

**Mapping targets**:
- MP3 ID3:
  - `TIT2`, `TPE1`, `TPE2`, `TALB`, `TRCK`, `TPOS`, `TDRC`, `TCON`
  - `TXXX:ARTISTS`, `TXXX:ALBUMARTISTS`
  - MusicBrainz via `TXXX` and common UFID handling where simple
  - ReplayGain via `TXXX:REPLAYGAIN_*`
- FLAC/Vorbis:
  - `TITLE`, `ARTIST`, `ARTISTS`, `ALBUM`, `ALBUMARTIST`, `ALBUMARTISTS`
  - `TRACKNUMBER`, `TOTALTRACKS`, `DISCNUMBER`, `TOTALDISCS`, `DATE`, `GENRE`
  - `MUSICBRAINZ_*`
  - `REPLAYGAIN_*`
- MP4/M4A:
  - `©nam`, `©art`, `aART`, `©alb`, `trkn`, `disk`, `©day`, `©gen`
  - `----:com.apple.iTunes:ARTISTS`, `----:com.apple.iTunes:ALBUMARTISTS`
  - `----:com.apple.iTunes:MUSICBRAINZ_*`
  - `----:com.apple.iTunes:REPLAYGAIN_*`

**Verification**:
```bash
pytest tests/test_reader.py
```

---

### Task 2.2.3: Implement tag reader service

**Action**: Add `src/auto_tagger/core/reader.py`.

**Design**:
- `read_metadata(path: Path) -> TrackMetadata`
- `read_album_metadata(path: Path, recursive: bool = False) -> dict[Path, TrackMetadata]`
- Use `load_audio_file` and per-format mapping helpers.
- Wrap mutagen errors in `TaggingError` or `FileProcessingError` with path context.

**Verification**:
```bash
pytest tests/test_reader.py
```

---

## Wave 2.3: Tag Writer Implementation

**Objective**: Write normalized metadata to supported audio formats without leaking format details to CLI code.

### Task 2.3.1: Implement write mappings

**Action**: Extend `src/auto_tagger/core/formats.py` with write helpers.

**Rules**:
- Do not write empty or `None` values.
- Preserve unrelated tags by default.
- Create tags when missing where mutagen requires explicit initialization.
- Use ID3v2.4 and UTF-8 for MP3.
- Store Vorbis comments as uppercase field names.
- Encode MP4 freeform values as UTF-8 bytes.

**Verification**:
```bash
pytest tests/test_writer.py
```

---

### Task 2.3.2: Implement writer service

**Action**: Add `src/auto_tagger/core/writer.py`.

**Design**:
- `write_metadata(path: Path, metadata: TrackMetadata, dry_run: bool = False) -> TrackMetadata`
- Return the metadata that would be written for preview/reporting.
- Skip `save()` when `dry_run=True`.
- Raise `TaggingError` on write/save failures.

**Verification**:
```bash
pytest tests/test_writer.py
```

---

### Task 2.3.3: Add a minimal CLI integration point

**Action**: Update `src/auto_tagger/commands/tag.py`.

**Design**:
- For Phase 2, `auto-tag tag PATH --dry-run` should read and print metadata for supported files.
- Non-dry-run should remain conservative unless explicit metadata is supplied by later phases.
- Replace the Phase 2 stub message with real read/discovery output.

**Verification**:
```bash
pytest tests/test_cli.py tests/test_reader.py
auto-tag tag /path/to/album --dry-run
```

---

## Wave 2.4: Multi-Value and ReplayGain Support

**Objective**: Ensure Navidrome-critical multi-artist and ReplayGain fields survive round trips.

### Task 2.4.1: Normalize multi-valued artists

**Action**: Add helpers in `metadata.py` or `formats.py`.

**Rules**:
- `artist` remains the display artist.
- `artists` is the individual artist list.
- `album_artist` remains the display album artist.
- `album_artists` is the individual album artist list.
- If multi-value fields are missing, default list fields from display fields instead of inventing splits.
- Avoid splitting on ambiguous names such as `AC/DC`.

**Verification**:
```bash
pytest tests/test_metadata.py tests/test_reader.py tests/test_writer.py
```

---

### Task 2.4.2: Normalize track, disc, and ReplayGain fields

**Action**: Implement parser/formatter helpers.

**Rules**:
- Track and disc values support `"N"` and `"N/TOTAL"`.
- ReplayGain gain values preserve strings like `"-6.84 dB"`.
- ReplayGain peak values preserve decimal strings.
- Do not calculate ReplayGain in Phase 2; only read/write provided values.

**Verification**:
```bash
pytest tests/test_metadata.py tests/test_reader.py tests/test_writer.py
```

---

### Task 2.4.3: Add round-trip tests

**Action**: Add tests proving write/read consistency.

**Test cases**:
- MP3: primary tags, multi-valued `TXXX` artists, ReplayGain `TXXX` frames
- FLAC: repeated Vorbis fields for artists and ReplayGain fields
- M4A: MP4 atoms and iTunes freeform fields
- Dry-run writer does not mutate files
- Unsupported extension raises a project exception

**Verification**:
```bash
pytest
```

---

## Implementation Notes

- Use mutagen directly in `core` only. Keep `commands/` focused on CLI presentation.
- Prefer small pure helper functions for format mapping so tests can cover behavior without requiring many binary fixture files.
- Binary audio fixtures should be minimal and committed only if they are tiny. If fixture generation is more reliable, add a deterministic pytest fixture instead.
- Do not implement beets lookup, LLM selection, cover art, lyrics, or ReplayGain calculation in Phase 2. Those belong to later phases.
- Keep writes conservative: preserve unrelated tags, avoid deleting existing metadata unless an explicit empty value policy is added later.

## Risks and Mitigations

**Mutagen format differences**: MP3, FLAC, and MP4 expose different APIs.
- Mitigation: isolate in `formats.py` and keep the public model stable.

**Binary fixture churn**: Audio fixtures can bloat the repo.
- Mitigation: use tiny generated fixtures or test mapping helpers directly where possible.

**ID3 compatibility**: Some players prefer ID3v2.3, but project research recommends ID3v2.4 for true multi-value support.
- Mitigation: use ID3v2.4 for Phase 2 and leave compatibility mode as a future config option if needed.

**Over-eager CLI behavior**: Phase 2 does not yet know final metadata from beets/LLM.
- Mitigation: CLI integration reads/previews tags only; automated writing is introduced when candidate metadata exists in later phases.

## Final Verification

Run:
```bash
ruff check src tests
mypy src
pytest --cov=auto_tagger
auto-tag --help
auto-tag tag <sample-album-path> --dry-run
```

Phase 2 is complete when all success criteria pass and `STATE.md` is updated to mark Phase 2 complete and Phase 3 ready for planning.
