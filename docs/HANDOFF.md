# Auto Tagger — Handoff Document

**Date:** 2026-05-14  
**Branch:** (current)  
**State:** 282 tests passing (1 network-dependent test flaky)

---

## What This Tool Does

A Python CLI that automates audio metadata tagging for Navidrome music servers. It reads existing tags, looks up correct metadata via beets/MusicBrainz, validates audio/LRC/cover quality, and writes fixes — all without manual intervention.

```
auto-tag batch "/Volumes/downloads/ARTIST" --yolo
```

---

## Changes in This Session (2026-05-14)

### 1. Folder Artist Enforcement (album_artist always matches the folder)

**Problem**: Albums under e.g. `/久石让/` would get `album_artist=久石譲` when MusicBrainz returned the Japanese kanji variant, or worse, get a completely different artist name from LLM fallback. Since the folder IS the authoritative source for the artist identity, album_artist should always match it.

**Fix in `src/auto_tagger/workflows/album.py`**:

- `_write_candidate_metadata()` now accepts `folder_artist` and **enforces** it as `album_artist` on every written track, overriding whatever the candidate provides.
- `_fix_via_llm()` also enforces `folder_artist` as `album_artist`, and uses the folder name for MBID map lookups instead of the LLM-returned artist name.
- `_fix_metadata()` passes `request.artist_hint` (extracted from the parent folder name) through to both write methods.

This means: **all albums under an artist folder get the same album_artist**, regardless of whether the lookup returns a different script variant or the LLM returns a completely different name.

### 2. SC/TC-Aware MusicBrainz Artist ID Map

**Problem**: The `artist_mbid_map` stored artist names under casefolded keys like `久石譲`, but when looking up for an album whose candidate had `artist=久石让` (simplified Chinese), the key didn't match. This meant MBIDs weren't propagating across albums that used different script variants of the same artist name.

**Fix in `src/auto_tagger/workflows/album.py`**:

Added three module-level helpers:

- **`_artist_variant_keys(name)`** — generates all script variants of an artist name (original, simplified Chinese, traditional Chinese, Hong Kong, Taiwan, plus all known aliases from the alias file). Uses OpenCC for conversion.
- **`_store_mbid_in_map(map, name, mbid)`** — stores the MBID under every variant key so lookups always match regardless of script.
- **`_lookup_mbid_in_map(map, name)`** — checks every variant of the query name against the map. This means `_lookup_mbid_in_map(map, "久石让")` finds the MBID stored under `"久石譲"`.

All map lookups now use `_lookup_mbid_in_map()`, and all stores use `_store_mbid_in_map()`. Both are used in `_write_candidate_metadata()`, `_fix_via_llm()`, and `_fix_metadata()`.

### 3. LLM Genre Bug Fix and Prompt Update

**Bug**: `_fix_via_llm()` was using `genre=metadata.genre` (existing tags, likely `None`) instead of `genre=llm_track.genre or metadata.genre`, discarding the LLM's genre recommendation.

**Fix**: Changed to `genre=llm_track.genre or metadata.genre` so the LLM-generated genre from `FallbackTagResponse.genre` flows through to written tags.

**Prompt update**: `build_fallback_messages()` now explicitly requests Discogs-style comma-separated genre/style tags, e.g. `"Electronic, House, Deep House"`. The LLM is told to only include genre when confident.

### 4. LLM Genre Enrichment Stub

Added `_enrich_genre_from_llm(artist, album)` — a static method that currently returns `None`. This is the extension point for a future lightweight OpenRouter query that asks "what genre is this album?" when Discogs returns nothing. The method is called from `_write_candidate_metadata()` (after Discogs fails) and from `_enrich_genre_fallback()` (called after LLM tag generation).

---

## Real-World Results (Joe Hisaishi, 45 albums)

Ran `auto-tag batch "/Volumes/downloads/久石让" --yolo` after all session changes:

| Metric | Before session | After session | After folder enforcement |
|--------|---------------|--------------|-------------------------|
| With MB Artist ID | 0% (never extracted) | **~64%** (29/45) | **~70%+** (32/45 through propagation) |
| With MB Album ID | 0% | **~66%** | **~66%** |
| With Genre | 0% | **~46%** | **~58%** (Discogs enrichment) |
| With Year | 0% | **~60%** | **~60%** |
| Cover art fixed | N/A | **34/45 (76%)** | **34/45** |
| album_artist matches folder | N/A | N/A | **45/45 (100%)** |

**6 albums still missing both MBID and genre** — these are genuinely obscure releases, fan compilations, or non-standard titles with no MusicBrainz or Discogs entries:

- `Ni no Kuni: Wrath of the White Witch` (game soundtrack)
- `Princess Mononoke: Music From The Miramax Motion Picture` (non-standard title)
- `宫崎峻电影主题曲全集 - 与Ghibli同行` (fan compilation)
- `紅の豚 サウンドトラック` (Japanese script variant not found in MB)
- `Hisaishi _ Miyazaki _ Kitano` (fan compilation)
- `NHKスペシャル BRAIN&MIND` (niche documentary)

All 6 now have `album_artist=久石让` enforced from the folder. They are candidates for the future `_enrich_genre_from_llm()` implementation.

---

## Key Design Decisions

### Folder Artist as Ground Truth
In a batch run, albums are organized as `/ArtistName/AlbumName/`. The **parent folder name is the definitive artist identity** for that album. The workflow enforces this:
- `album_artist` always matches the folder name
- MB artist ID lookups use the folder name (not the candidate's artist name) so SC/TC variants resolve correctly
- `_artist_mismatches_folder()` detects when existing tags disagree with the folder

### SC/TC-Aware MDID Map
The `artist_mbid_map` uses OpenCC script conversion to handle all variant keys:
- Storage writes the MBID under every known variant of the artist name (original, s2t, t2s, tw2s, s2hk, hk2s, plus aliases)
- Lookups check all variants of the query name against stored keys
- This ensures `久石让` (folder) → `久石譲` (MB) → `hisaishi joe` (alias) all resolve to the same MBID

### Cross-Album MBID Propagation
`BatchWorkflow.run()` creates a single `artist_mbid_map` and passes it to each `AlbumWorkflow.run()` call. When an album's candidate has an MB artist ID, it's stored. When a later album's candidate is missing the ID, it's looked up by the folder artist. Over a 45-album run, this boosted MB artist ID coverage from ~64% to ~70%+.

### Health Gate
YOLO mode only writes metadata when `health_report.can_tag` is True — meaning zero ERROR-level issues. Warnings don't block writes but are reported.

### Cover Art Fix (Independent of Metadata)
Cover art fixing runs regardless of metadata health. Flow:
1. `{album_name}.jpg` (album-named cover)
2. `cover.jpg`, `folder.jpg`, `front.jpg`, `album.jpg/png` (generic names)
3. Cover Art Archive fetch (if MusicBrainz album ID exists)
4. Discogs cover art fetch (if CAA fails)
5. Embeds into all audio files via mutagen

### Genre Enrichment Cascade
1. **From candidate**: If the lookup candidate has genre data, use it
2. **From Discogs**: `_enrich_genre_from_discogs()` queries Discogs for genre+styles
3. **From LLM (stub)**: `_enrich_genre_from_llm()` placeholder for future implementation

### Simplified/Traditional Chinese Name Matching
`artist_matches_any()` in `aliases.py` does a cascading match:
1. Direct substring match (casefolded)
2. Full-string OpenCC variant match (s2t, t2s, s2tw, etc.)
3. Character-level overlap: per-character OpenCC conversion + overlap scoring ≥50%
4. Alias match (persisted JSON alias file)

### Beets Client
Real beets integration — `BeetsClient` creates `Item` objects from audio files for `tag_album`. Rate-limited to respect MusicBrainz API. Lookup results are cached in SQLite (`MatchCache`).

### LLM Prompt Genre Format
The fallback prompt requests Discogs-style genres (comma-separated genre/style tags) rather than MusicBrainz folksonomy because:
- Discogs genres are structured (genre + style taxonomy)
- The existing genre enrichment pipeline uses Discogs format
- MusicBrainz genres are free-form tags with no consistent taxonomy

### Supported Formats
FLAC (Vorbis comments), MP3 (ID3), M4A/MP4 (MP4 atoms), WAV (ID3)

---

## Project Structure (Key Files)

```
src/auto_tagger/workflows/
├── album.py          # AlbumWorkflow — single-album preview + YOLO apply
│                     # Contains: _fix_metadata, _fix_via_llm,
│                     #   _write_candidate_metadata, _enrich_genre_from_discogs,
│                     #   _enrich_genre_from_llm (stub),
│                     #   _artist_variant_keys, _store_mbid_in_map,
│                     #   _lookup_mbid_in_map (module-level helpers)
├── batch.py          # BatchWorkflow — library-wide batch processing
│                     # Creates and passes artist_mbid_map to AlbumWorkflow.run()
└── interactive.py    # Interactive prompt mode

src/auto_tagger/integrations/
├── aliases.py        # artist_matches_any(), save_alias(), get_aliases(),
│                     #   _convert_script(), _characters_overlap()
├── beets_client.py   # MusicBrainz lookup via beets (artist_id extraction)
├── candidates.py     # AlbumCandidate, TrackCandidate, LookupRequest models
├── discogs_client.py # Discogs API (genre enrichment, cover art)
├── lookup.py         # Lookup orchestration: cache → dataset → beets → folder
├── fallback.py       # Folder-name parsing
└── cache.py          # SQLite match cache

src/auto_tagger/llm/
├── prompts.py        # build_fallback_messages() — prompt templates
├── schemas.py        # FallbackTagResponse — genre field included
└── fallback.py       # FallbackTagGenerationService
```

---

## Cache

Lookup results are cached in `.planning/cache.db` (configurable via `AUTO_TAG_CACHE_PATH`). The cache stores serialized `AlbumCandidate` lists keyed by query hash. If you modify lookup behavior (new fields, new sources), **clear the cache**:

```bash
rm -f .planning/cache.db
```

---

## Test Infrastructure

### Fixture Factory (`tests/fixtures/factory.py`)
Generates synthetic test files using ffmpeg + mutagen:
- 11-track 潘玮柏 album (FLAC + LRC + cover.jpg)
- Multi-artist compilation
- Format samples (FLAC/MP3/M4A)
- Edge cases (empty tags, corrupt, missing cover, Unicode paths)

### Pytest Marks
| Mark | Condition |
|------|-----------|
| `needs_ffmpeg` | ffmpeg + ffprobe on PATH |
| `needs_beets` | `beet` on PATH |
| `needs_rgain` | rgain3 or loudgain on PATH |
| `network` | Requires network access |

### Key Test Files
| File | Tests | Domain |
|------|-------|--------|
| `test_album_workflow.py` | 5 | Dry-run, YOLO health gate, cover art fix, missing MBID fix |
| `test_batch_workflow.py` | 2 | Album discovery, failure handling with `artist_mbid_map` |
| `test_beets_client.py` | 4 | Beets client normalization, sorting, error wrapping |

---

## Known Gaps

| Gap | Details |
|-----|---------|
| **`_enrich_genre_from_llm()` stub** | Returns `None` — needs a real OpenRouter call. A lightweight prompt (~50 tokens) asking "What genre is this album?" with Discogs-style output would cost <$0.001 per album |
| **Single-album YOLO fix** | `auto-tag tag --yolo` doesn't route through `AlbumWorkflow` — cover art + metadata fixes only work in batch mode |
| **Japanese shinjitai mapping** | OpenCC doesn't handle Japanese shinjitai→simplified Chinese for some chars (e.g., `譲`→`让`). The character-level overlap heuristic handles most cases, but a dedicated mapping table would be more comprehensive |
| **Cache staleness** | No schema version or auto-invalidation. Must manually `rm -f .planning/cache.db` after code changes |
| **Streaming LLM** | `OpenRouterClient.stream_complete_json()` — 0% coverage, needs live API key |
| **Real dataset download** | `aria2c` + `7z` multi-GB torrent — code paths tested via mocked subprocess |
| **WAV write tests** | No synthetic WAV fixture in factory |
| **compilation/obscure albums** | Albums without MusicBrainz entries fall to LLM fallback, which can't provide MBIDs. MBID propagation helps when at least one album by the artist has an ID |

---

## How to Continue

### If you want to fix remaining albums
```bash
# Check health of any library
auto-tag batch "/path/to/library" --dry-run --health-report health.json

# Auto-fix everything that's safe (clear cache first)
rm -f .planning/cache.db
auto-tag batch "/path/to/library" --yolo
```

### If you want to add features (priority order)
1. **Implement `_enrich_genre_from_llm()`** — the stub in `album.py` needs a real OpenRouter call. Use a cheap model (Haiku/GPT-4o-mini), a short prompt (~50 tokens), and target <100 tokens output. The response should be a single `genre` string in Discogs format, or null if uncertain.

2. **Single-album YOLO fix** — route `auto-tag tag --yolo` through `AlbumWorkflow` so cover art + metadata fixes work in single-album mode. Currently only batch mode applies fixes.

3. **Cache-aware versioning** — include a schema version hash in the cache so it auto-invalidates after code changes. See `src/auto_tagger/integrations/cache.py`.

4. **Japanese shinjitai mapping table** — add a small dict for common pairs OpenCC misses:
   ```python
   SHINJITAI_MAP = {"譲": "让", "侮": "社", "僧": "僧", "猪": "猪", ...}
   ```

5. **Parallel batch processing** — the `parallel` CLI flag exists but `BatchWorkflow.run()` only does sequential execution. The `artist_mbid_map` is mutable and shared, so parallel mode would need a lock or a different approach.

### If you want to release
```bash
python -m build             # builds dist/
ruff check src tests        # must be clean
mypy src                    # must be clean
pytest -q --ignore=tests/test_dataset_commands.py  # must be 282/282
```

### Quick Start for a New Developer
```bash
# Set up
just venv

# Test
just test

# Run on a library (dry-run first!)
rm -f .planning/cache.db
just run batch "/Volumes/downloads/ARTIST" --dry-run

# Apply fixes
just run batch "/Volumes/downloads/ARTIST" --yolo
```

---

## Architecture Notes

### Data Flow for `auto-tag batch --yolo`
```
BatchWorkflow.run()
  │
  ├── artist_mbid_map = {}  # shared across all albums
  │
  ├── for each album:
  │     └── AlbumWorkflow.run(artist_mbid_map=artist_mbid_map)
  │           ├── read_metadata() → detect missing fields
  │           ├── if needs_fix:
  │           │     └── _fix_metadata(artist_mbid_map)
  │           │           ├── LookupService.lookup_album()
  │           │           │     ├── cache hit? return cached
  │           │           │     ├── dataset lookup
  │           │           │     ├── beets/MusicBrainz lookup
  │           │           │     ├── Discogs lookup (merge)
  │           │           │     └── folder fallback
  │           │           ├── _select_best_candidate()
  │           │           ├── if candidate found:
  │           │           │     ├── _lookup_mbid_in_map() → inject MBID
  │           │           │     ├── _enrich_genre_from_discogs()
  │           │           │     └── _write_candidate_metadata(folder_artist)
  │           │           │           ├── _store_mbid_in_map() → propagate
  │           │           │           ├── enforce folder_artist
  │           │           │           └── write_metadata() per file
  │           │           └── if no candidate:
  │           │                 └── _fix_via_llm(folder_artist)
  │           │                       ├── LLM generates tags
  │           │                       ├── enforce folder_artist
  │           │                       └── _enrich_genre_fallback()
  │           ├── if genre still missing:
  │           │     └── _enrich_genre_from_lookup()
  │           └── _fix_cover_art()
  │
  └── return BatchSummary
```

### artist_mbid_map Lifecycle
```
Created empty at BatchWorkflow.run() start
  │
  ├── Album A (久石让/空想美術館)
  │     └── _fix_metadata() finds MB Artist ID 44c64a30...
  │     └── _store_mbid_in_map("久石譲", "44c64a30...")
  │           └── stores under: 久石譲, 久石让, 久石让, 久石譲, ...
  │
  ├── Album B (久石让/菊次郎の夏)
  │     └── _lookup_mbid_in_map("久石让") → finds "44c64a30..."
  │     └── injects into candidate → writes to all tracks
  │
  └── ... continues for all 45 albums
```
