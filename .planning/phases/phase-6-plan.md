# Phase 6 Execution Plan: Navidrome Features & Distribution

## Overview

**Goal**: Add remaining Navidrome-specific features, complete the CLI workflow, and prepare distribution artifacts

**Duration**: ~10-14 hours

**Dependencies**: Phase 5 complete

**Primary Requirements**:
- REQ-ND-002: Lyrics/LRC Support
- REQ-ND-003: Cover Art Handling
- REQ-ND-004: Compilation Albums
- REQ-CL-001: Batch Mode
- REQ-CL-002: Single Album Mode
- REQ-CL-003: YOLO Mode
- REQ-CL-004: Interactive Mode
- REQ-CL-005: Configuration
- REQ-CL-006: Homebrew Distribution

**Success Criteria**:
- Folder cover images are detected and MusicBrainz Cover Art Archive can be queried for missing covers
- Cover art can be embedded for MP3, FLAC/Vorbis, and MP4/M4A through format-specific helpers
- Compilation albums are detected and tagged with `album_artist=Various Artists` and `compilation=1`
- Embedded lyrics and adjacent LRC files are handled consistently for Navidrome
- Batch mode discovers albums, processes them with progress output, supports bounded parallelism, and emits a summary report
- Interactive mode previews before/after metadata and supports accept, skip, and abort choices
- YOLO mode applies supported changes without interactive prompts while still honoring health blockers
- Packaging metadata, README usage, build artifacts, PyPI release dry-run, and Homebrew formula template are ready for a release
- Unit tests cover new feature logic without live network calls or external publishing credentials

---

## Architecture Target

Phase 6 completes the v1 feature surface by adding media enrichment services and a real command workflow around the Phase 2-5 primitives. Keep the command layer thin: workflow modules should return structured results that are easy to test, render, and serialize.

**New modules**:
```
src/auto_tagger/features/
  __init__.py
  cover_art.py        # folder cover detection, CAA fetching, embedding plan
  compilations.py     # compilation detection and metadata transforms
  lyrics.py           # embedded lyrics/LRC mapping and tag application

src/auto_tagger/workflows/
  __init__.py
  album.py            # single-album orchestration for preview/apply
  batch.py            # library album discovery, parallel execution, summaries
  interactive.py      # prompt abstraction and accept/skip/edit decisions

packaging/
  homebrew/auto-tagger.rb
```

**New tests**:
```
tests/test_cover_art.py
tests/test_compilations.py
tests/test_lyrics.py
tests/test_album_workflow.py
tests/test_batch_workflow.py
tests/test_interactive_workflow.py
tests/test_distribution.py
```

**Dependency approach**:
- Reuse `httpx` for Cover Art Archive HTTP access.
- Reuse `mutagen` for artwork and lyrics embedding.
- Avoid adding release tools as runtime dependencies; put build/twine-style tooling in dev dependencies only if needed.
- Keep all network access behind injectable clients for deterministic tests.

---

## Wave 6.1: Cover Art Handling

**Objective**: Detect local artwork, fetch missing artwork from MusicBrainz Cover Art Archive, and embed images in supported audio formats.

### Task 6.1.1: Add cover art models and local discovery

**Action**: Add `features/cover_art.py`.

**Design**:
- `CoverArtImage`:
  - `path`
  - `mime_type`
  - `data`
  - `source`
- `CoverArtStatus` / `CoverArtResult`:
  - found local cover
  - fetched remote cover
  - already embedded
  - missing cover
  - fetch failed
- Local image names:
  - `cover.jpg`
  - `cover.jpeg`
  - `folder.jpg`
  - `front.jpg`
  - `album.jpg`
  - PNG equivalents
- Validate image extension and MIME signature before embedding.
- Prefer folder-local artwork over network fetches.

**Verification**:
```bash
.venv/bin/pytest tests/test_cover_art.py
```

---

### Task 6.1.2: Implement Cover Art Archive client

**Action**: Add a small HTTP client with injectable transport.

**Design**:
- Input: MusicBrainz album ID from lookup/metadata.
- Endpoint shape:
  - `/release/{mbid}/front`
  - fallback to JSON release cover-art metadata if needed
- Handle:
  - 200 image response
  - 404 no cover
  - timeout/network failure
  - non-image content
- Respect dry-run by returning planned fetch/embed actions without writing.

**Testing strategy**:
- Fake HTTP client responses for success, 404, timeout, and invalid content.
- Do not hit live Cover Art Archive in tests.

**Verification**:
```bash
.venv/bin/pytest tests/test_cover_art.py
```

---

### Task 6.1.3: Embed cover art per audio format

**Action**: Extend format-specific writer helpers or add focused artwork helpers.

**Design**:
- MP3/ID3: write `APIC` front-cover frame.
- FLAC/Vorbis: write `METADATA_BLOCK_PICTURE`.
- MP4/M4A: write `covr` atom.
- Preserve existing text metadata and unrelated artwork.
- If no cover is found, leave files untagged and add a health/report note rather than embedding a placeholder.

**Verification**:
```bash
.venv/bin/pytest tests/test_cover_art.py tests/test_formats.py tests/test_writer.py
```

---

## Wave 6.2: Compilation Detection and Tagging

**Objective**: Detect compilation albums and normalize tags for Navidrome.

### Task 6.2.1: Implement compilation detection

**Action**: Add `features/compilations.py`.

**Detection signals**:
- Folder or album artist is `Various Artists`.
- Album title or folder name includes soundtrack/OST/compilation indicators.
- Track artists vary significantly across an album.
- Beets/MusicBrainz candidate marks album as compilation when available.
- Existing compilation tag is present.

**Design**:
- `CompilationAnalysis`:
  - `is_compilation`
  - `confidence`
  - `reasons`
- Keep heuristics explainable for interactive preview.

**Verification**:
```bash
.venv/bin/pytest tests/test_compilations.py
```

---

### Task 6.2.2: Apply compilation metadata

**Action**: Add transform helpers that update `TrackMetadata`.

**Rules**:
- For compilations, set `album_artist="Various Artists"`.
- Preserve per-track `artist` and `artists`.
- Set a normalized `compilation` field once the core metadata model supports it.
- Add format mappings:
  - MP3/ID3: `TCMP` or `TXXX:COMPILATION`
  - Vorbis: `COMPILATION=1`
  - MP4: `cpil`
- Do not overwrite a specific album artist unless the analysis confidence is high or the user accepts the preview.

**Verification**:
```bash
.venv/bin/pytest tests/test_compilations.py tests/test_metadata.py tests/test_formats.py
```

---

## Wave 6.3: Lyrics Integration

**Objective**: Embed unsynchronized lyrics and preserve synchronized LRC files for Navidrome.

### Task 6.3.1: Add lyrics models and discovery

**Action**: Add `features/lyrics.py`.

**Design**:
- Reuse Phase 5 LRC discovery and validation.
- Discover sidecar lyrics:
  - matching `.lrc`
  - matching `.txt`
  - album-level lyric files when unambiguous
- `LyricsPayload`:
  - text
  - source path
  - synchronized boolean
  - encoding
- Validate UTF-8 conversion path before apply mode.

**Verification**:
```bash
.venv/bin/pytest tests/test_lyrics.py tests/test_lrc_validation.py
```

---

### Task 6.3.2: Embed lyrics per audio format

**Action**: Extend writer helpers for lyrics tags.

**Mappings**:
- MP3/ID3:
  - `USLT` for unsynchronized lyrics
  - keep synchronized LRC as sidecar unless a safe synchronized ID3 mapping is explicitly implemented
- FLAC/Vorbis:
  - `LYRICS`
  - optional `UNSYNCEDLYRICS` alias if needed
- MP4/M4A:
  - `©lyr`
- Preserve adjacent `.lrc` files for Navidrome sync support; do not delete or rename them.

**Verification**:
```bash
.venv/bin/pytest tests/test_lyrics.py tests/test_formats.py tests/test_writer.py
```

---

## Wave 6.4: Single Album Workflow Completion

**Objective**: Convert the current dry-run preview command into a reusable album workflow that can preview and apply supported changes.

### Task 6.4.1: Add album workflow orchestration

**Action**: Add `workflows/album.py`.

**Design**:
- `AlbumWorkflowResult`:
  - discovered files
  - current metadata
  - lookup candidates
  - selected/generated metadata
  - quality report
  - cover result
  - lyrics result
  - compilation result
  - replaygain result
  - planned writes
  - applied writes
- Workflow order:
  1. discover audio files
  2. read metadata
  3. validate health
  4. lookup/LLM/fallback metadata
  5. enrich cover/lyrics/compilation/ReplayGain
  6. preview or apply writes
- Treat health error issues as write blockers unless YOLO explicitly supports override later.

**Verification**:
```bash
.venv/bin/pytest tests/test_album_workflow.py tests/test_cli.py
```

---

### Task 6.4.2: Complete `auto-tag tag`

**Action**: Wire the command to the workflow.

**Behavior**:
- Default non-YOLO mode shows a preview and does not write unless interactive acceptance is provided.
- `--dry-run` always previews only.
- `--yolo` applies safe changes with no prompt.
- Output includes:
  - before/after metadata summary
  - selected candidate/fallback source
  - health blockers
  - cover/lyrics/ReplayGain/compilation actions
  - final applied/skipped count

**Verification**:
```bash
.venv/bin/pytest tests/test_album_workflow.py tests/test_cli.py
.venv/bin/auto-tag tag "潘玮柏/2006-反转地球" --dry-run
```

---

## Wave 6.5: Batch Mode Completion

**Objective**: Process a full music library with progress, bounded parallelism, and a summary report.

### Task 6.5.1: Discover albums in a library

**Action**: Add album discovery to `workflows/batch.py`.

**Design**:
- Walk a library root and group audio files by album directory.
- Respect `settings.recursive`, `settings.recursive_depth`, and exclude patterns.
- Skip directories with no supported audio files.
- Return deterministic sorted album paths for stable tests.

**Verification**:
```bash
.venv/bin/pytest tests/test_batch_workflow.py
```

---

### Task 6.5.2: Execute batch jobs with progress and summary

**Action**: Implement batch execution using the album workflow.

**Design**:
- Support `--parallel N` with bounded worker count.
- Default to sequential execution for deterministic local runs.
- Use Rich progress output for albums processed.
- Continue after per-album failures and collect:
  - processed
  - applied
  - skipped
  - failed
  - health error count
  - estimated LLM cost
- Write optional JSON summary if a CLI flag is added.

**Verification**:
```bash
.venv/bin/pytest tests/test_batch_workflow.py tests/test_cli.py
```

---

## Wave 6.6: Interactive Mode

**Objective**: Let users accept, skip, edit, or abort per album after seeing a preview.

### Task 6.6.1: Add prompt abstraction

**Action**: Add `workflows/interactive.py`.

**Design**:
- `PromptSession` protocol so tests can provide scripted answers.
- Choices:
  - accept
  - skip
  - edit basic fields
  - abort
- Show before/after rows and issue summary before prompting.
- Default to preview-only when stdin is not interactive.

**Verification**:
```bash
.venv/bin/pytest tests/test_interactive_workflow.py
```

---

### Task 6.6.2: Wire interactive CLI flags

**Action**: Add `--interactive` behavior to `tag` and `batch`.

**Rules**:
- `--dry-run` takes precedence and never writes.
- `--yolo` skips prompts and applies safe changes.
- `--interactive` prompts per album.
- If neither `--yolo` nor `--interactive` is set, default to preview-only for safety in v1 unless project settings explicitly opt into interactive default.

**Verification**:
```bash
.venv/bin/pytest tests/test_interactive_workflow.py tests/test_cli.py
```

---

## Wave 6.7: Distribution and Release Readiness

**Objective**: Prepare reliable package artifacts and installation instructions without requiring credentials during normal test runs.

### Task 6.7.1: Complete package metadata and README

**Action**: Update `pyproject.toml` and `README.md`.

**Changes**:
- Replace placeholder project URLs with real repository URLs if known.
- Add CLI examples for:
  - `tag --dry-run`
  - `tag --interactive`
  - `tag --yolo`
  - `batch --dry-run`
  - `--health-report`
- Document config file path and key environment variables.
- Document external tools:
  - `ffmpeg/ffprobe`
  - `rgain3` or `loudgain`
- Confirm package includes all new modules.

**Verification**:
```bash
.venv/bin/python -m build
.venv/bin/pytest tests/test_distribution.py
```

---

### Task 6.7.2: Add Homebrew formula template

**Action**: Add `packaging/homebrew/auto-tagger.rb`.

**Design**:
- Formula installs the PyPI sdist/wheel release using Homebrew Python virtualenv conventions.
- Include dependencies or resource placeholders as appropriate.
- Add comments or release checklist for updating URL and SHA256 after PyPI release.
- Test validates formula file contains expected class, homepage, license, and install/test blocks.

**Verification**:
```bash
.venv/bin/pytest tests/test_distribution.py
```

---

### Task 6.7.3: Add release checklist and PyPI dry-run

**Action**: Add credential-free release documentation.

**Design**:
- Document commands:
  - build artifacts
  - inspect wheel/sdist
  - upload to TestPyPI
  - upload to PyPI
  - update Homebrew formula URL/SHA
- Do not commit API tokens or publish automatically in CI.
- If a publishing workflow is added, require manual dispatch and trusted secrets.

**Verification**:
```bash
.venv/bin/python -m build
.venv/bin/pytest tests/test_distribution.py
```

---

## Cross-Wave Integration

### Task 6.X.1: Config expansion

**Candidate settings**:
- `cover_art_enabled`, default `True`
- `cover_art_archive_enabled`, default `True`
- `cover_art_timeout_seconds`, default `20`
- `lyrics_enabled`, default `True`
- `embed_lyrics`, default `True`
- `compilation_detection_enabled`, default `True`
- `batch_summary_path`, default `None`
- `interactive_default`, default `False`

**Verification**:
```bash
.venv/bin/pytest tests/test_config.py
```

---

### Task 6.X.2: End-to-end release candidate checks

**Action**: Run a complete local validation suite.

**Commands**:
```bash
.venv/bin/ruff check src tests
.venv/bin/mypy src
.venv/bin/pytest --cov=auto_tagger
.venv/bin/auto-tag tag "潘玮柏/2006-反转地球" --dry-run
.venv/bin/auto-tag batch "潘玮柏" --dry-run
.venv/bin/python -m build
```

---

## Risks and Mitigations

**Publishing requires credentials**:
- Real PyPI and Homebrew publishing should not happen during automated local execution.
- Mitigation: prepare artifacts, templates, and checklist; require explicit credentialed release action.

**Cover art network dependency**:
- Cover Art Archive may be unavailable or rate-limited.
- Mitigation: prefer local covers, use timeouts, cache/skip repeated failures, and test with fake clients.

**Media mutation risk**:
- Cover, lyrics, compilation, and ReplayGain writes modify user files.
- Mitigation: dry-run remains safe, interactive preview shows before/after, YOLO still honors health blockers.

**Batch scale**:
- Large libraries can produce long-running jobs and many network calls.
- Mitigation: bounded parallelism, progress output, per-album isolation, resumable JSON summaries when feasible.

**Compilation false positives**:
- Multi-artist albums are not always compilations.
- Mitigation: explainable confidence score and interactive confirmation before overwriting album artist in non-YOLO mode.

---

## Final Verification

Before marking Phase 6 complete, run:

```bash
.venv/bin/ruff check src tests
.venv/bin/mypy src
.venv/bin/pytest --cov=auto_tagger
.venv/bin/auto-tag tag "潘玮柏/2006-反转地球" --dry-run
.venv/bin/auto-tag batch "潘玮柏" --dry-run
.venv/bin/python -m build
```

If build tooling is not installed, add it to dev dependencies or document the blocked command explicitly before completion.
