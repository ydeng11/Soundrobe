# Auto Tagger — Handoff Document

**Date:** 2026-05-10  
**Branch:** `codex/local-dataset-setup`  
**State:** 233 tests passing, 89% coverage, all lints clean

---

## What This Tool Does

A Python CLI that automates audio metadata tagging for Navidrome music servers. It reads existing tags, looks up correct metadata via beets/MusicBrainz, validates audio/LRC/cover quality, and writes fixes — all without manual intervention.

```
auto-tag batch "/Volumes/downloads/5566" --yolo --health-report health.json
```

---

## Project Structure

```
src/auto_tagger/
├── cli.py                  # Click CLI: tag, batch, dataset, config commands
├── commands/               # Command entry points (thin orchestration)
│   ├── tag.py              # Single-album preview
│   ├── batch.py            # Library-wide batch processing
│   ├── dataset.py          # Dataset setup/status (torrent download, bencode, CSV import)
│   └── config_cmd.py       # Config view/set
├── config/                 # Settings (env vars, YAML, CLI, defaults)
├── core/                   # Audio I/O
│   ├── audio.py            # Format detection, file discovery, mutagen loading
│   ├── formats.py          # Tag read/write for FLAC/MP3/M4A/WAV
│   ├── metadata.py         # Normalized TrackMetadata model
│   ├── reader.py           # read_metadata(), read_album_metadata()
│   └── writer.py           # write_metadata()
├── integrations/           # External data sources
│   ├── beets_client.py     # Beets/MusicBrainz lookup (album + track)
│   ├── cache.py            # SQLite match cache
│   ├── candidates.py       # AlbumCandidate, TrackCandidate, LookupRequest models
│   ├── dataset.py          # Local MusicMoveArr dataset SQLite index
│   ├── fallback.py         # Folder-name parsing + clean_folder_name()
│   └── lookup.py           # Lookup orchestration: cache → dataset → beets → folder
├── llm/                    # OpenRouter AI integration
│   ├── client.py           # OpenRouterClient (chat completions)
│   ├── selection.py        # CandidateSelectionService
│   ├── fallback.py         # LLM-based tag generation from filenames
│   ├── schemas.py          # Pydantic response schemas
│   ├── prompts.py          # Prompt templates
│   └── cost.py             # Token cost estimation
├── quality/                # Validation
│   ├── health.py           # AlbumHealthReport, build_album_health_report()
│   ├── audio_validation.py # FFProbeValidator
│   ├── metadata_validation.py  # Track/album metadata checks
│   ├── lrc.py              # LRC discovery, encoding validation, UTF-8 conversion
│   └── replaygain.py       # ReplayGainCalculator (rgain3/loudgain)
├── features/               # Navidrome enrichment
│   ├── cover_art.py        # discover_local_cover_art(), CoverArtArchiveClient, embed_cover_art()
│   ├── compilations.py     # Multi-artist detection
│   └── lyrics.py           # LRC/TXT discovery and embedding
├── workflows/
│   ├── album.py            # AlbumWorkflow: preview + YOLO apply + cover art fix
│   ├── batch.py            # BatchWorkflow: discover albums → process each
│   └── interactive.py      # Interactive prompt mode
└── utils/                  # Logging, console output, table rendering
```

---

## Key Design Decisions

### Health Gate
YOLO mode (`--yolo`) only writes metadata when `health_report.can_tag` is True — meaning zero ERROR-level issues. Warnings don't block writes but are reported.

### Cover Art Fix (Independent of Metadata)
Cover art fixing runs regardless of metadata health. Flow:
1. `{album_name}.jpg` (album-named cover)
2. `cover.jpg`, `folder.jpg`, `front.jpg`, `album.jpg/png` (generic names)
3. Cover Art Archive fetch (if MusicBrainz album ID exists in metadata)
4. Embeds into all audio files via mutagen

### Folder Name Cleanup
`clean_folder_name()` strips date prefixes, bookmarks, and suffixes from folder names before using them as MusicBrainz lookup hints:
- `2003-04《挚爱》(FLAC分轨)` → `挚爱`

### Beets Client
Real beets integration — `BeetsClient` creates `Item` objects from audio files for `tag_album`. Rate-limited to respect MusicBrainz API.

### Supported Formats
FLAC (Vorbis comments), MP3 (ID3), M4A/MP4 (MP4 atoms), WAV (ID3)

---

## Test Infrastructure

### Fixture Factory (`tests/fixtures/factory.py`)
Generates synthetic test files using ffmpeg + mutagen:
- 11-track 潘玮柏 album (FLAC + LRC + cover.jpg)
- Multi-artist compilation
- Format samples (FLAC/MP3/M4A)
- Edge cases (empty tags, corrupt, missing cover, Unicode paths)

Files go to `tests/fixtures/data/` (gitignored), generated lazily once per session.

### Pytest Marks
| Mark | Condition |
|------|-----------|
| `needs_ffmpeg` | ffmpeg + ffprobe on PATH |
| `needs_beets` | `beet` on PATH |
| `needs_rgain` | rgain3 or loudgain on PATH |

---

## Test Suite (233 tests)

### Test Files and What They Cover

| File | Tests | Domain |
|------|-------|--------|
| `test_fixtures.py` | 16 | Fixture factory validation |
| `test_cli.py` | 16 | CLI arg parsing, help, version |
| `test_cli_e2e.py` | 10 | End-to-end dry-run (tag + batch) |
| `test_health_report_integration.py` | 14 | All 6 issue types from real libraries + fix paths |
| `test_album_workflow.py` | 5 | AlbumWorkflow: dry-run, YOLO health gate, cover art fix |
| `test_batch_workflow.py` | 2 | Batch discovery, failure continuation |
| `test_beets_client.py` | 4 | Beets client normalization, sorting, error wrapping |
| `test_beets_integration.py` | 5 | Real beets library: configure, track lookup, album lookup, rate limiter |
| `test_dataset_write.py` | 8 | DatasetIndexWriter, CSV import, Chinese normalization |
| `test_dataset_download.py` | 19 | Bencode parser, torrent paths, service selection, archive matching |
| `test_dataset_cli.py` | 3 | Dataset CLI status/setup |
| `test_dataset_commands.py` | 3 | Dataset command entry points |
| `test_dataset_index.py` | 2 | Dataset client lookup |
| `test_write_mutation.py` | 7 | Non-dry-run writes for FLAC/MP3/M4A |
| `test_formats.py` | 2 | MP3/MP4 tag round-trips |
| `test_fallback.py` | 3 | Folder-structure fallback |
| `test_folder_cleanup.py` | 8 | clean_folder_name() patterns |
| `test_wav.py` | 2 | WAV format detection + discovery |
| `test_lookup.py` | 6 | LookupService: cache → dataset → beets → folder |
| `test_llm_client.py` | 4 | OpenRouter: structured completion, retry, API key required |
| `test_llm_selection.py` | 3 | Candidate selection service |
| `test_llm_fallback.py` | 2 | LLM fallback tag generation |
| `test_llm_schemas.py` | 3 | Response schema validation |
| `test_llm_prompts.py` | 2 | Prompt construction |
| `test_llm_cost.py` | 2 | Cost estimation |
| `test_cover_art.py` | 5 | Local discovery, CAA fetch, embedding |
| `test_audio_validation.py` | 4 | FFProbeValidator (mocked) |
| `test_ffprobe_integration.py` | 3 | Real ffprobe on fixtures |
| `test_metadata_validation.py` | 6 | Track/album metadata validation |
| `test_health_report.py` | 3 | Health report structure |
| `test_lrc_validation.py` | 5 | LRC encoding, timing, conversion |
| `test_replaygain.py` | 4 | ReplayGain calculator (mocked) |
| `test_replaygain_integration.py` | 3 | Real ReplayGain (skip if no binary) |
| `test_compilations.py` | 3 | Compilation detection + tagging |
| `test_lyrics.py` | 4 | LRC/TXT discovery + embedding |
| `test_config.py` | 7 | Settings: defaults, env, YAML, validation |
| `test_cache.py` | 2 | Match cache store/load |
| `test_distribution.py` | 3 | Pyproject, Homebrew, release checklist |
| `test_command_entry_points.py` | 3 | tag.execute() direct invocation |
| `test_logging.py` | 3 | Log level setup |
| `test_edge_cases.py` | 7 | Unicode paths, corrupt files, permission errors |
| `test_audio.py` | 4 | Format detection, file discovery |
| `test_reader.py` | 2 | FLAC metadata reading |
| `test_writer.py` | 2 | Dry-run write |
| `test_interactive_workflow.py` | 2 | Interactive prompt mode |
| `test_beets_candidates.py` | 3 | Candidate model serialization |
| `test_metadata.py` | 4 | Position parsing, display formatting |

---

## Known Gaps

| Gap | Details |
|-----|---------|
| Streaming LLM | `OpenRouterClient.stream_complete_json()` — 0% coverage, needs live API key |
| Real dataset download | `aria2c` + `7z` multi-GB torrent — code paths tested via mocked subprocess |
| `__main__.py` | 3-line entry point wrapper — trivial |
| Permissions-edge | OS-level tests need root/chmod |
| Single-album YOLO apply | `auto-tag tag` doesn't use `AlbumWorkflow` — cover art fix only works in batch mode |
| WAV write tests | No synthetic WAV fixture (ffmpeg WAV generation not added to factory) |

---

## How to Continue

### If you want to fix remaining albums
```bash
# Check health of any library
auto-tag batch "/Volumes/downloads/ARTIST" --dry-run --health-report health.json

# Auto-fix everything that's safe
auto-tag batch "/Volumes/downloads/ARTIST" --yolo
```

### If you want to add features
1. **Single-album YOLO fix** — route `auto-tag tag --yolo` through `AlbumWorkflow` so cover art + metadata fixes work in single-album mode
2. **Streaming LLM tests** — mock the streaming protocol for `stream_complete_json`
3. **Pre-commit hook** — run `ruff check && pytest -q` before commits

### If you want to release
```bash
python -m build          # builds dist/
ruff check src tests     # must be clean
mypy src                 # must be clean
pytest                   # must be 233/233
```

---

## Commit History (This Session)

```
296344d feat: discover_local_cover_art checks album-name cover first
2ccde3c feat: automatic cover art fix in YOLO mode
30d4f89 test: health report integration tests — all 6 issue types + fix paths
bd4fc53 feat: add --health-report to batch command
caaa8a7 feat: add cover art check to health report
5154e5b test: deep-integration tests — beets fix, dataset download, writes
08cf2fd fix: beets album lookup creates Items from audio files
200b3ad test: complete test hardening (+60 tests, 85%→89%)
f0e3f04 feat: WAV file support + smart folder name cleanup
```
