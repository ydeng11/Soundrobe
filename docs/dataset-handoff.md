# Dataset System Handoff

## Overview

Replaced the CSV-based ETL pipeline with a raw SQL import approach. Instead of building a separate denormalized index, the MusicMoveArr PostgreSQL dump files are imported directly into SQLite and JOINed at query time. A lightweight `dataset_lookup` table with pre-computed normalized columns enables sub-10ms album lookups.

## Current State

All 4 services imported from the **22 Feb 2026** dataset:

| Service      | Artists   | Albums    | Tracks    |
|-------------|-----------|-----------|-----------|
| musicbrainz | 2,654,567 | 5,089,880 | 51,151,455 |
| spotify     | 1,272,660 | 3,730,825 | 18,153,738 |
| tidal       | 8,082,380 | 29,350,649| 117,908,655|
| deezer      | 9,162,439 | 35,605,360| 182,555,505|

**Database:** `~/.auto-tagger/dataset-index.sqlite` (~160 GB)
**Lookup table:** `dataset_lookup` — 73,768,467 rows (denormalized artist+album with normalized columns)
**Query speed:** 7–12 ms per album lookup

## Data Layout

```
~/.auto-tagger/
├── datasets/                    # Raw .7z archives (32 GB)
│   ├── musicbrainz_22_feb_2026.7z
│   ├── spotify_22_feb_2026.7z
│   ├── tidal_22_feb_2026.7z
│   ├── deezer_22_feb_2026.7z
│   └── CSV.7z
├── staging/                     # Extracted .sql files (331 GB)
│   ├── musicbrainz_*.sql        # 7 files
│   ├── spotify_*.sql            # 6 files
│   ├── tidal_*.sql              # 12 files
│   └── deezer_*.sql             # 9 files
├── dataset-index.sqlite         # SQLite database (160 GB)
└── dataset-state.json           # State metadata
```

## Architecture

### Before (old approach)
```
.csv files → build_index_from_csv_tree() → dataset_albums + dataset_tracks tables
```
- Required buffering 51M+ track dicts in Python memory → OOM
- Slow one-by-one INSERTs with JSON serialization

### After (current approach)
```
.sql files → import_raw_tables() → raw SQL tables → dataset_lookup table
                                         ↓
                                    query_album() → JOIN + exact match (7ms)
```

### Key files

| File | Purpose |
|------|---------|
| `src/soundrobe/integrations/dataset_raw.py` | SQL dump parser, table importer, lookup query engine, lookup table builder |
| `src/soundrobe/integrations/dataset.py` | `DatasetIndexWriter` (kept for CSV backward compat), `add_albums_bulk()`, state management |
| `src/soundrobe/commands/dataset.py` | `execute_setup()`, `execute_build()`, `execute_status()` |
| `src/soundrobe/integrations/lookup.py` | `LookupService._lookup_dataset()` — wired to `query_album()` |
| `src/soundrobe/cli.py` | `dataset build` and `dataset status` commands |

### Database schema

Raw tables mirror the PostgreSQL dump structure (one table per source file). Examples:
- `musicbrainz_release` (artistid, releaseid, title, date, ...)
- `spotify_album` (albumid, artistid, name, releasedate, ...)
- `tidal_track` (trackid, albumid, title, isrc, duration, ...)
- `deezer_artist` (artistid, name, nbalbum, nbfan, ...)

Lookup table:
```sql
dataset_lookup (
    service TEXT,          -- 'musicbrainz', 'spotify', 'tidal', 'deezer'
    artist TEXT,           -- original artist name
    album TEXT,            -- original album title
    year TEXT,             -- release date/year
    album_id TEXT,         -- source-specific album ID
    artist_id TEXT,        -- source-specific artist ID
    normalized_artist TEXT, -- LOWER(REPLACE(...)) for exact matching
    normalized_album TEXT   -- LOWER(REPLACE(...)) for exact matching
)
```

Index: `idx_lookup_norm ON dataset_lookup (normalized_artist, normalized_album, service)`

### Query flow

1. `normalize_lookup_text()` strips punctuation and lowercases input hints
2. `SELECT ... FROM dataset_lookup WHERE normalized_artist = ? AND normalized_album = ?` — exact match via index (~7ms)
3. `_load_tracks_by_id()` fetches tracks from the raw track table using the album FK
4. Results ordered: musicbrainz → spotify → tidal → deezer

## How to Use

### Check status
```bash
auto-tag dataset status
```

### Build from staged data (no re-download)
```bash
auto-tag dataset build --service musicbrainz
auto-tag dataset build --service spotify --service tidal
```

### Full setup (download + build)
```bash
auto-tag dataset setup           # prompts for services
auto-tag dataset setup --dry-run # preview only
```

### Tag an album (uses dataset)
```bash
auto-tag tag /path/to/album
```

## Update / Refresh Dataset

When a new MusicMoveArr dataset version is released:

1. Download new `.7z` files to `~/.auto-tagger/datasets/`
2. Extract to `~/.auto-tagger/staging/`:
   ```bash
   7z x ~/.auto-tagger/datasets/<new>.7z -o~/.auto-tagger/staging -y
   ```
3. Delete old index and rebuild:
   ```bash
   rm ~/.auto-tagger/dataset-index.sqlite
   auto-tag dataset build --service musicbrainz --service spotify --service tidal --service deezer
   ```

Import time: ~5 hours for all 4 services on 16 GB RAM / Apple Silicon.

## Known Limitations

1. **No incremental updates** — the entire database is rebuilt from scratch. For a new dataset version, delete `dataset-index.sqlite` and re-import.
2. **No MusicBrainz flat CSV** — the 22 Feb 2026 dataset has MB as SQL only. The CSV-based fallback path in `build_index_from_csv_tree()` remains for older formats.
3. **Join tables imported unnecessarily** — `deezer_album_image_link` (177M rows) and similar join tables are imported but not used for lookup. They inflate the DB by ~50 GB. Could be excluded in `_table_name_from_file()`.
4. **Duplicate candidates** — multiple releases of the same album (different countries/formats) return as separate candidates. No deduplication by MusicBrainz release group.
