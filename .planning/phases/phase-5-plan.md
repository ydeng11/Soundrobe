# Phase 5 Execution Plan: Quality Assurance & Validation

## Overview

**Goal**: Implement file validation, health checks, reporting, and ReplayGain calculation

**Duration**: ~7-10 hours

**Dependencies**: Phase 2, Phase 3, Phase 4 complete

**Primary Requirements**:
- REQ-QA-001: Corrupt File Detection
- REQ-QA-002: LRC Encoding Validation
- REQ-QA-003: Metadata Validation
- REQ-QA-004: Health Report
- REQ-ND-001: ReplayGain Tags

**Success Criteria**:
- Audio validation detects unplayable, truncated, or malformed files before tagging
- LRC files are validated for UTF-8 encoding and timing-tag structure
- Metadata validation flags missing required tags, invalid track/disc numbers, and album-level inconsistencies
- Health report aggregates all album issues for console output and optional JSON export
- ReplayGain values can be calculated and written through the existing Phase 2 tag writer
- Unit tests cover quality checks and ReplayGain orchestration without requiring live external binaries

---

## Architecture Target

Phase 5 adds an `soundrobe.quality` layer that inspects albums and produces structured health issues. Validation code should be reusable by CLI flows and future batch processing. External tools must sit behind injectable runner boundaries so tests do not depend on local `ffprobe`, `rgain3`, or `loudgain` installations.

**New modules**:
```
src/soundrobe/quality/
  __init__.py
  audio_validation.py      # ffprobe/mutagen integrity validation
  health.py                # issue/result/report models and JSON serialization
  lrc.py                   # LRC encoding, timing validation, UTF-8 conversion
  metadata_validation.py   # required-field and album-consistency checks
  replaygain.py            # rgain3/loudgain orchestration and tag application
```

**New tests**:
```
tests/test_audio_validation.py
tests/test_health_report.py
tests/test_lrc_validation.py
tests/test_metadata_validation.py
tests/test_replaygain.py
```

**CLI integration target**:
- Extend `auto-tag tag --dry-run` with a health summary for discovered album files.
- Add an optional JSON health report output flag if the existing command shape can accept it cleanly.
- Keep actual file mutations, including LRC conversion and ReplayGain tag writes, disabled in dry-run mode.

**Dependency approach**:
- Do not add a Python dependency for `ffprobe`; call the system binary through an injectable subprocess runner.
- Prefer a ReplayGain command wrapper that supports `rgain3` first and can fall back to `loudgain` when configured or detected.
- Only add a runtime Python dependency if implementation proves `rgain3` can be safely used as a library without pulling fragile platform dependencies into import time.
- Avoid heuristic encoding dependencies unless needed; start with strict UTF-8 validation plus conservative fallback decode attempts for common legacy encodings.

---

## Wave 5.1: Audio File Validation

**Objective**: Detect corrupt, unplayable, truncated, and structurally invalid audio before tagging.

### Task 5.1.1: Define audio validation result models

**Action**: Add structured result types in `quality/audio_validation.py`.

**Design**:
- `AudioValidationStatus` enum or literal values:
  - `valid`
  - `warning`
  - `invalid`
  - `skipped`
- `AudioValidationIssue` dataclass:
  - `path`
  - `code`
  - `message`
  - `severity`
  - `details`
- `AudioValidationResult` dataclass:
  - `path`
  - `status`
  - `duration_seconds`
  - `format_name`
  - `codec_name`
  - `issues`
- Include conversion helpers into `HealthIssue` once health models exist.

**Verification**:
```bash
.venv/bin/pytest tests/test_audio_validation.py
```

---

### Task 5.1.2: Implement ffprobe validation boundary

**Action**: Add an injectable `FFProbeValidator`.

**Design**:
- Run:
  - `ffprobe -v error -show_format -show_streams -print_format json <path>`
- Parse JSON output for:
  - at least one audio stream
  - codec name
  - duration present and positive
  - format duration consistent enough with stream duration when both are available
- Treat non-zero exit codes, timeout, missing binary, invalid JSON, and empty stream data as validation issues.
- Use a short timeout per file to avoid hanging batch runs.
- Do not mutate files.

**Testing strategy**:
- Mock runner success with valid JSON.
- Mock truncated/unreadable file exit.
- Mock missing `ffprobe`.
- Mock invalid JSON.
- Mock no audio streams.

**Verification**:
```bash
.venv/bin/pytest tests/test_audio_validation.py
```

---

## Wave 5.2: LRC Encoding and Format Validation

**Objective**: Validate synchronized lyric files and safely convert non-UTF-8 files when explicitly applying changes.

### Task 5.2.1: Implement LRC discovery and parsing

**Action**: Add `quality/lrc.py`.

**Design**:
- Discover `.lrc` files adjacent to audio files and within the album folder.
- Parse supported lines:
  - timing lines: `[mm:ss.xx]lyrics`
  - repeated timestamps on one line
  - metadata tags: `[ar:]`, `[al:]`, `[ti:]`, `[by:]`, `[offset:]`
  - blank lines
- Flag malformed timing tags, negative/invalid times, and lines that look like broken tags.
- Preserve unknown text lines as warnings rather than hard failures unless they are malformed bracket tags.

**Verification**:
```bash
.venv/bin/pytest tests/test_lrc_validation.py
```

---

### Task 5.2.2: Validate and convert encoding

**Action**: Implement strict UTF-8 validation and conservative conversion.

**Design**:
- Try strict UTF-8 decode first.
- If it fails, try a small ordered set of likely encodings:
  - `utf-8-sig`
  - `gb18030`
  - `big5`
  - `cp1252`
  - `latin-1`
- Return an issue when a file is not UTF-8 but can be decoded.
- Provide `convert_lrc_to_utf8(path, *, dry_run=True)`:
  - dry-run returns the proposed encoding and issue list
  - apply mode writes UTF-8 bytes only after successful full-file decode
  - never performs partial/best-effort rewrites
- Keep timestamps and lyric content unchanged except for encoding and optional BOM removal.

**Testing strategy**:
- UTF-8 file passes with no conversion.
- UTF-8 BOM file is identified.
- Legacy encoded file reports conversion opportunity.
- Undecodable file reports an error and is not rewritten.
- Apply mode writes valid UTF-8 bytes using a temporary test file.

**Verification**:
```bash
.venv/bin/pytest tests/test_lrc_validation.py
```

---

## Wave 5.3: Metadata Validation

**Objective**: Validate that album metadata meets Navidrome and project requirements after read, lookup, LLM selection, or write operations.

### Task 5.3.1: Implement track-level metadata validation

**Action**: Add `quality/metadata_validation.py`.

**Checks**:
- Required fields:
  - `title`
  - `artist`
  - `album`
  - `album_artist`
  - `track_number`
- Track number must be positive.
- Disc number must be positive when present.
- Total tracks/discs must be positive when present.
- Track number must not exceed total tracks when both are present.
- Disc number must not exceed total discs when both are present.
- ReplayGain fields, when present, should match expected gain/peak shape.

**Testing strategy**:
- Complete metadata passes.
- Each missing required field creates a focused issue.
- Invalid numeric fields produce errors.
- ReplayGain shape warnings cover malformed gain and peak values.

**Verification**:
```bash
.venv/bin/pytest tests/test_metadata_validation.py
```

---

### Task 5.3.2: Implement album-level consistency checks

**Action**: Add album validation across a sequence of `TrackMetadata`.

**Checks**:
- Album title is consistent across tracks.
- Album artist is consistent across tracks unless compilation logic says otherwise.
- Track numbers are unique per disc.
- Track sequence gaps are warnings.
- Total track count is consistent where present.
- Disc totals are consistent where present.
- Artist variance is allowed, but should suggest compilation handling when album artist is inconsistent or missing.

**Verification**:
```bash
.venv/bin/pytest tests/test_metadata_validation.py
```

---

## Wave 5.4: Health Report Generation

**Objective**: Aggregate all validation output into a report that is useful in console workflows and machine-readable in JSON.

### Task 5.4.1: Define health report models

**Action**: Add `quality/health.py`.

**Models**:
- `HealthSeverity`:
  - `info`
  - `warning`
  - `error`
- `HealthIssue`:
  - `category`
  - `severity`
  - `path`
  - `code`
  - `message`
  - `details`
- `TrackHealth`
  - `path`
  - `issues`
  - `can_tag`
- `AlbumHealthReport`
  - `album_path`
  - `tracks_checked`
  - `lrc_files_checked`
  - `issues`
  - `can_tag`
  - `summary`
- JSON helpers should only emit plain built-in types.

**Verification**:
```bash
.venv/bin/pytest tests/test_health_report.py
```

---

### Task 5.4.2: Implement report generation and console rendering

**Action**: Add a report builder that combines audio, metadata, LRC, cover, and ReplayGain signals.

**Design**:
- `build_album_health_report(album_path, audio_files, metadata_by_path, *, include_lrc=True)`
- Mark `can_tag=False` when any error severity issue affects an audio file.
- Provide Rich table/tree rendering for CLI summary.
- Provide JSON serialization for `--health-report <path>` if CLI flag is added.
- Include cover status as an issue category placeholder if cover detection is not yet implemented in Phase 5; Phase 6 can fill the richer cover-art checks.

**CLI integration**:
- In `tag --dry-run`, display health summary after file discovery/metadata preview.
- Skip actual tag writes for files with error-level validation failures once non-dry-run flows are added.
- Keep report generation resilient: a failure in one validator should become a health issue, not abort the whole album unless no audio files can be read.

**Verification**:
```bash
.venv/bin/pytest tests/test_health_report.py tests/test_cli.py
```

---

## Wave 5.5: ReplayGain Calculation

**Objective**: Calculate ReplayGain values and apply them through the existing normalized metadata writer.

### Task 5.5.1: Implement ReplayGain command wrapper

**Action**: Add `quality/replaygain.py`.

**Design**:
- `ReplayGainCalculator` with injectable runner and settings.
- Support command selection:
  - default: `rgain3`
  - optional fallback/configured command: `loudgain`
- Validate the selected command exists before running.
- Calculate at album scope where possible so album gain/peak values are available.
- Return structured `ReplayGainCalculation` results keyed by path:
  - track gain
  - track peak
  - album gain
  - album peak
  - command output/errors
- Normalize output to the existing `ReplayGainTags` model from Phase 2.

**Testing strategy**:
- Mock successful command output.
- Mock missing command.
- Mock non-zero exit.
- Mock partial result for one failed file in an album.
- Confirm normalized gain values include `dB` and peaks are strings/floats accepted by writer tests.

**Verification**:
```bash
.venv/bin/pytest tests/test_replaygain.py
```

---

### Task 5.5.2: Apply ReplayGain tags safely

**Action**: Wire ReplayGain results into existing metadata writer paths.

**Design**:
- Add service function:
  - `calculate_replaygain_for_album(paths, *, dry_run=True)`
  - `apply_replaygain_tags(results, *, dry_run=True)`
- Dry-run prints planned ReplayGain updates without modifying files.
- Apply mode updates only ReplayGain fields and preserves all other metadata.
- If calculation fails, surface health issues and skip tag writes for affected files.
- Do not calculate ReplayGain for files already marked corrupt by audio validation.

**Verification**:
```bash
.venv/bin/pytest tests/test_replaygain.py tests/test_writer.py
```

---

## Cross-Wave Integration

### Task 5.X.1: Settings and command options

**Action**: Add configuration fields only as needed by implementation.

**Candidate settings**:
- `ffprobe_path`, default `ffprobe`
- `ffprobe_timeout_seconds`, default `20`
- `replaygain_command`, default `rgain3`
- `replaygain_timeout_seconds`, default based on album size or a conservative fixed value
- `lrc_convert_encoding`, default `False`

**Verification**:
```bash
.venv/bin/pytest tests/test_config.py tests/test_cli.py
```

---

### Task 5.X.2: End-to-end dry-run flow

**Action**: Ensure `auto-tag tag <album> --dry-run` runs the health pipeline without writing.

**Expected output additions**:
- Count of audio files checked
- Count of LRC files checked
- Error/warning summary
- ReplayGain availability or pending calculation status
- Clear message when external tools are missing

**Verification**:
```bash
.venv/bin/auto-tag tag "潘玮柏/2006-反转地球" --dry-run
```

---

## Risks and Mitigations

**External binary availability**:
- `ffprobe`, `rgain3`, and `loudgain` may be absent in local or CI environments.
- Mitigation: injectable runners, unit tests with mocked output, graceful CLI warnings, and no hard import-time dependency on binaries.

**ReplayGain tool behavior differences**:
- `rgain3` and `loudgain` differ in command shape and output.
- Mitigation: isolate parsers per command and normalize into Phase 2 `ReplayGainTags`.

**Encoding false positives**:
- Legacy text encodings can decode incorrectly.
- Mitigation: report detected conversion as a warning, rewrite only after full-file decode, and keep dry-run default.

**Large album performance**:
- Running `ffprobe` and ReplayGain over many tracks can be slow.
- Mitigation: per-file timeouts, album-level ReplayGain runs, and predictable progress output.

**Unintended mutation**:
- LRC conversion and ReplayGain tag writing can modify user media.
- Mitigation: dry-run by default in current CLI flow, explicit apply path later, and writer tests that preserve unrelated tags.

---

## Final Verification

Before marking Phase 5 complete, run:

```bash
.venv/bin/ruff check src tests
.venv/bin/mypy src
.venv/bin/pytest --cov=soundrobe
.venv/bin/auto-tag tag "潘玮柏/2006-反转地球" --dry-run
```

Optional environment checks:

```bash
ffprobe -version
rgain3 --version
loudgain --version
```

External-tool checks are informational unless the implementation path requires the binary for a specific manual validation step.
