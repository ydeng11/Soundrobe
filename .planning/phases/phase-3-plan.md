# Phase 3 Execution Plan: Beets Integration

## Overview

**Goal**: Integrate beets for MusicBrainz lookup and candidate retrieval

**Duration**: ~5-7 hours

**Dependencies**: Phase 2 complete

**Primary Requirements**:
- REQ-BT-001: MusicBrainz Lookup
- REQ-BT-002: Candidate Selection
- REQ-BT-003: Folder Structure Fallback
- REQ-BT-004: Match Caching

**Success Criteria**:
- Can configure beets as a library without loading the user's beets config
- Can query MusicBrainz through beets for album and track candidates
- Returns normalized candidates with album, artist, tracks, MusicBrainz IDs, distance, and source fields
- Folder parsing extracts artist/album hints from `/Artist/Album` paths when lookup has no candidates
- SQLite cache stores and retrieves lookup results deterministically
- Unit tests cover candidate mapping, folder parsing, cache behavior, and mocked lookup flows

---

## Architecture Target

Phase 3 should keep beets behind an integration boundary. Core metadata services from Phase 2 remain independent of beets, and CLI commands should call a project-owned lookup service rather than importing beets directly.

**New modules**:
```
src/auto_tagger/integrations/
  __init__.py
  beets_client.py    # Beets config isolation and lookup wrapper
  cache.py           # SQLite candidate cache
  candidates.py      # Normalized lookup/candidate models
  fallback.py        # Folder structure parsing fallback
  lookup.py          # Orchestrates cache -> beets -> fallback
```

**New tests**:
```
tests/test_beets_candidates.py
tests/test_beets_client.py
tests/test_cache.py
tests/test_fallback.py
tests/test_lookup.py
```

**Dependency update**:
- Add `beets>=2.0.0` to runtime dependencies.

---

## Wave 3.1: Beets Library Setup and Candidate Models

**Objective**: Add the dependency and establish a clean internal API for lookup candidates.

### Task 3.1.1: Add beets dependency

**Action**: Update `pyproject.toml` runtime dependencies with `beets>=2.0.0`.

**Verification**:
```bash
.venv/bin/python -m pip install -e ".[dev]"
.venv/bin/python -c "import beets; print(beets.__version__)"
```

---

### Task 3.1.2: Create normalized candidate models

**Action**: Add `src/auto_tagger/integrations/candidates.py`.

**Design**:
- `LookupSource` enum: `BEETS`, `FOLDER`
- `TrackCandidate` dataclass:
  - `title`
  - `artist`
  - `artists`
  - `track_number`
  - `track_total`
  - `disc_number`
  - `disc_total`
  - `musicbrainz_trackid`
  - `length`
- `AlbumCandidate` dataclass:
  - `artist`
  - `artists`
  - `album`
  - `album_artist`
  - `album_artists`
  - `year`
  - `genre`
  - `musicbrainz_albumid`
  - `musicbrainz_artistid`
  - `tracks`
  - `distance`
  - `source`
- `LookupRequest` dataclass:
  - `path`
  - `artist_hint`
  - `album_hint`
  - `tracks`
- JSON serialization helpers for cache persistence.

**Verification**:
```bash
.venv/bin/pytest tests/test_beets_candidates.py
```

---

### Task 3.1.3: Create isolated beets configuration wrapper

**Action**: Add setup helpers in `src/auto_tagger/integrations/beets_client.py`.

**Design**:
- Do not import or use `beets.ui`.
- Initialize beets config with `user_config=False`.
- Use a temporary or configured library path isolated from the user's real beets database.
- Disable expensive or interactive behavior for lookup-only usage:
  - no importer prompts
  - no chroma/fingerprinting in Phase 3
  - no file moves/copies
- Provide `BeetsClient` with injectable `match_album`/`match_track` callables for tests.

**Verification**:
```bash
.venv/bin/pytest tests/test_beets_client.py
```

---

## Wave 3.2: MusicBrainz Lookup Implementation

**Objective**: Query beets and convert beets proposals into stable project candidate models.

### Task 3.2.1: Implement album lookup wrapper

**Action**: Add `BeetsClient.lookup_album(request: LookupRequest) -> list[AlbumCandidate]`.

**Design**:
- Use Phase 2 `read_album_metadata` to collect existing track/title hints when needed.
- Call beets autotag album matching with artist/album hints and track metadata.
- Return candidates sorted by ascending distance or best match first.
- Cap returned candidates with a configurable `max_candidates` default, e.g. 5.
- Wrap beets/network errors in `TaggingError` with path and query context.

**Testing strategy**:
- Mock the beets match function; do not hit MusicBrainz in unit tests.
- Include one fixture with two candidates and verify normalized candidate fields.

**Verification**:
```bash
.venv/bin/pytest tests/test_beets_client.py
```

---

### Task 3.2.2: Implement track lookup wrapper

**Action**: Add `BeetsClient.lookup_track(path: Path) -> list[TrackCandidate]`.

**Design**:
- Use Phase 2 `read_metadata` for local file hints.
- Call beets track matching when a single file path is supplied.
- Normalize title, artist, track number, MusicBrainz track ID, and distance if available.
- Keep track lookup separate from album lookup so later CLI decisions can choose the right mode.

**Verification**:
```bash
.venv/bin/pytest tests/test_beets_client.py
```

---

### Task 3.2.3: Add rate-limit guard

**Action**: Add a small rate limiter used before live MusicBrainz-backed beets calls.

**Rules**:
- Enforce at least 1 second between live lookup calls by default.
- Make the limiter injectable/testable with fake time/sleep functions.
- Do not sleep when returning cached results.

**Verification**:
```bash
.venv/bin/pytest tests/test_beets_client.py tests/test_lookup.py
```

---

## Wave 3.3: Folder Structure Fallback

**Objective**: Extract reliable artist/album hints when beets returns no candidates or cannot run.

### Task 3.3.1: Implement folder parser

**Action**: Add `src/auto_tagger/integrations/fallback.py`.

**Design**:
- `parse_album_path(path: Path) -> LookupRequest`
- Assumes `/Artist/Album` for album directories.
- For a file path, use the file's parent as album and grandparent as artist.
- Ignore obvious non-semantic folders such as empty path parts.
- Preserve original casing and punctuation.
- Mark `Various Artists`, `VA`, `Soundtrack`, and `OST` as compilation-like hints without forcing tags yet.

**Verification**:
```bash
.venv/bin/pytest tests/test_fallback.py
```

---

### Task 3.3.2: Build folder fallback candidate

**Action**: Add `candidate_from_folder(request: LookupRequest) -> AlbumCandidate`.

**Design**:
- Source is `LookupSource.FOLDER`.
- Candidate includes artist/album hints and track count.
- Tracks should use existing Phase 2 metadata when available; otherwise use sorted filenames as title hints.
- Do not invent MusicBrainz IDs.
- This is a fallback candidate for later LLM enrichment, not final authoritative metadata.

**Verification**:
```bash
.venv/bin/pytest tests/test_fallback.py
```

---

## Wave 3.4: Match Caching System

**Objective**: Avoid repeated MusicBrainz/beets lookup work by caching normalized candidates.

### Task 3.4.1: Implement SQLite cache schema

**Action**: Add `src/auto_tagger/integrations/cache.py`.

**Design**:
- `MatchCache(cache_path: Path)`
- Create parent directory automatically.
- Table `lookup_cache`:
  - `query_hash TEXT PRIMARY KEY`
  - `query_json TEXT NOT NULL`
  - `response_json TEXT NOT NULL`
  - `created_at TEXT NOT NULL`
  - `source TEXT NOT NULL`
- Use JSON serialization from `candidates.py`.
- Use stable hash of artist hint, album hint, track titles, and track count.

**Verification**:
```bash
.venv/bin/pytest tests/test_cache.py
```

---

### Task 3.4.2: Implement lookup orchestration

**Action**: Add `src/auto_tagger/integrations/lookup.py`.

**Design**:
- `LookupService.lookup_album(path: Path) -> list[AlbumCandidate]`
- Flow:
  1. Parse folder/request hints.
  2. Check SQLite cache.
  3. If cache miss, call `BeetsClient.lookup_album`.
  4. If beets returns candidates, cache and return them.
  5. If no candidates or beets fails recoverably, return folder fallback candidate.
- Do not cache transient errors.
- Do cache folder fallback results separately from beets results.

**Verification**:
```bash
.venv/bin/pytest tests/test_lookup.py
```

---

### Task 3.4.3: Add CLI preview integration

**Action**: Extend `auto-tag tag PATH --dry-run` output with lookup candidates when Phase 3 lookup is available.

**Design**:
- Keep Phase 2 metadata preview.
- Add a "Lookup candidates" table showing source, artist, album, year, distance, and MusicBrainz album ID.
- If lookup fails, show a warning and still display local metadata preview.
- Do not write tags in Phase 3.

**Verification**:
```bash
.venv/bin/pytest tests/test_cli.py tests/test_lookup.py
```

---

## Implementation Notes

- Unit tests should mock beets calls. Live MusicBrainz access belongs in optional/manual smoke tests, not default CI.
- Keep the cache storing project-owned candidate JSON, not raw beets objects.
- Keep beets config isolated from user config and user beets library databases.
- Do not implement LLM match selection in Phase 3. Phase 3 should only provide candidates and fallback hints.
- Do not implement cover art, lyrics, ReplayGain calculation, or health reports here.
- Use Phase 2 `TrackMetadata` where practical, but keep lookup candidate models separate because they represent possible metadata, not current file tags.

## Risks and Mitigations

**Beets API instability**: The internal autotag APIs can be awkward and may differ by beets version.
- Mitigation: isolate beets access inside `BeetsClient` and test mapping with fake proposal objects.

**Network-dependent tests**: MusicBrainz lookups are slow and rate-limited.
- Mitigation: mock beets lookup functions in tests and leave live smoke tests manual.

**User beets config interference**: Loading user config could move files, enable plugins, or point at an existing library database.
- Mitigation: call beets config with user config disabled and use isolated temp/configured paths.

**Cache staleness**: MusicBrainz data can change.
- Mitigation: include `created_at` in cache rows and keep TTL invalidation as an explicit extension point.

**Fallback overconfidence**: Folder parsing can be wrong.
- Mitigation: mark fallback candidates with `LookupSource.FOLDER` and no MusicBrainz IDs so Phase 4 can treat them differently.

## Final Verification

Run:
```bash
.venv/bin/ruff check src tests
.venv/bin/mypy src
.venv/bin/pytest --cov=auto_tagger
auto-tag tag <sample-album-path> --dry-run
```

Phase 3 is complete when all success criteria pass, Beets requirements are reflected in `STATE.md`, and Phase 4 is ready for planning.
