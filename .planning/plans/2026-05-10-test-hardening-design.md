# Test Hardening Design

**Goal:** Raise test suite from 85% breadth-only coverage to 90%+ with deep real-integration validation and end-to-end coverage.

**Approach:** Fixture-Driven Layered — build synthetic fixtures once, then test everything against them in four phases.

## Architecture

```
tests/fixtures/
├── __init__.py
├── factory.py          # Generates all synthetic audio/metadata fixtures
├── plugin.py           # Pytest fixtures (session-scoped, auto-generated)
├── conftest.py         # Shared marks: needs_ffmpeg, needs_beets, needs_rgain
└── data/               # .gitignore'd — generated at test time
    ├── 潘玮柏/
    │   └── 2006-反转地球/    # 11 FLAC + 11 LRC + cover.jpg
    ├── compilation/
    │   └── Various Artists/Compilation/  # multi-artist fixture
    ├── formats/
    │   ├── test.flac
    │   ├── test.mp3
    │   └── test.m4a
    └── edge_cases/
        ├── empty_tags/           # FLAC with no tags
        ├── corrupt/              # non-audio file renamed as .flac
        └── missing_cover/        # album with no cover art
```

## Fixture Factory Design

Uses `ffmpeg` (subprocess) to generate 0.1s silent audio in FLAC/MP3/M4A, then `mutagen` to write Vorbis/ID3/MP4 tags. LRC files are plain text with timing. Cover art is a 1×1 JPEG generated via pure Python (PIL not required — raw JPEG bytes).

Fixtures mimic the existing 潘玮柏/2006-反转地球 album:
- 11 FLAC tracks with Chinese metadata (artist `潘玮柏`, album `反转地球`)
- 11 LRC files with synchronized lyrics
- 1 cover.jpg

All generated files go to `tests/fixtures/data/` which is `.gitignore`'d. Generation happens lazily on first `pytest` run if the directory is missing. A `--regenerate-fixtures` flag forces rebuild.

## Phase Layering

| Phase | What | Coverage Target | New Tests |
|-------|------|-----------------|-----------|
| 1 | Fixture factory + validation | — | ~8 |
| 2 | Real-integration (beets, ffprobe, rgain3) | +5% | ~12 |
| 3 | E2E CLI dry-run | +3% | ~10 |
| 4 | Remaining gaps (dataset write, commands, logging, edges) | +5% | ~20 |

## Pytest Marks

- `@pytest.mark.needs_ffmpeg` — skip if ffmpeg/ffprobe not found
- `@pytest.mark.needs_beets` — skip if `import beets` fails
- `@pytest.mark.needs_rgain` — skip if no rgain3/loudgain in PATH
- `@pytest.mark.network` — skip if `--no-network` flag or no connectivity

## Key Design Decisions

1. **Fixtures are generated, not committed.** Keeps repo small. Generation is idempotent and fast (<1s for all fixtures).
2. **Real-integration tests skip gracefully.** `pytest.mark.skipif` with descriptive reasons. CI opt-in via markers.
3. **E2E tests use CliRunner (no subprocess).** Same pattern as existing `test_cli.py` — fast, debuggable.
4. **No new dependencies.** Uses only `ffmpeg`, `mutagen`, `beets` — already in the project's toolchain.
