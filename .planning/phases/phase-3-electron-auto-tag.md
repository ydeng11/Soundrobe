# Phase 3: Auto-Tag — Electron Main Process

**Status:** Planned
**Depends on:** Phase 1–2 (Electron scaffold + React UI) — Done
**New deps:** `better-sqlite3`, `opencc-js`

---

## Overview

Implement the full auto-tag lookup chain in the Electron main process. When a user selects an album and clicks "Auto-Tag", the chain runs:

> **LLM hint enhancement** (pre-lookup) → **Cache** → **Local Dataset** → **MusicBrainz API** → **Discogs API** → **Folder fallback** → **Cache write** → **LLM selection**

All logic runs in the main process (Node.js). The renderer invokes via IPC and polls a `taskId`. No HTTP servers, no ports.

---

## Lookup Chain (mirrors Python v1)

```
autoTagAlbum(albumPath)
  │
  ├─ 1. Parse folder hints (fallback.ts: parse_album_with_tags)
  ├─ 2. LLM hint enhancement (openrouter.ts + prompts.ts)
  │     Only if folder name has ambiguous annotations ([香港首版], 《》「」, etc.)
  │     Cached per folder — only calls LLM once
  │
  ├─ 3. Cache check (cache.ts: get)
  │     Returns instantly if previously looked up
  │
  ├─ 4. Local SQLite dataset (dataset.ts: query)
  │     Reads `~/.soundrobe/` SQLite index via better-sqlite3
  │     Fast, offline, zero cost
  │
  ├─ 5. MusicBrainz API (musicbrainz.ts: searchAlbum)
  │     Raw fetch() — rate limited to 1 req/sec
  │     Tries: original → SC/TC variants → aliases → album-only
  │
  ├─ 6. Discogs API (discogs.ts: searchAlbum)
  │     Fetches artist images + album metadata
  │     Same variant probing as MusicBrainz
  │     Merged with existing candidates
  │
  ├─ 7. Folder fallback (fallback.ts: candidateFromFolder)
  │     Always included as safety net if:
  │     - No candidates at all, OR
  │     - All candidates verify as "mismatch"
  │
  ├─ 8. Cache write (cache.ts: set)
  │     Save all candidates for next time
  │
  └─ 9. LLM selection (openrouter.ts: selectBest)
        Port prompt from llm/prompts.py → selects best candidate
        Returns selected AlbumCandidate
```

---

## Files to Create/Modify

### New handler files (in `frontend/electron/handlers/`)

| File | Ported From | Purpose |
|---|---|---|
| `handlers/auto-tag.ts` | `integrations/lookup.py` | Orchestrator + task queue |
| `handlers/candidates.ts` | `integrations/candidates.py` | Types: AlbumCandidate, TrackCandidate, LookupRequest |
| `handlers/cache.ts` | `integrations/cache.py` | SQLite cache via better-sqlite3 |
| `handlers/fallback.ts` | `integrations/fallback.py` | Folder parsing + fallback candidate |
| `handlers/dataset.ts` | `integrations/dataset_raw.py` | Read `~/.soundrobe/` SQLite index |
| `handlers/musicbrainz.ts` | `integrations/beets_client.py` | Raw fetch() to MusicBrainz API |
| `handlers/discogs.ts` | `integrations/discogs_client.py` | Raw fetch() to Discogs API (artist album search) |
| `handlers/openrouter.ts` | `llm/client.py` + `llm/selection.py` | OpenRouter API client |
| `handlers/aliases.ts` | `integrations/aliases.py` | Artist alias management |
| `handlers/prompts.ts` | `llm/prompts.py` | Prompt templates |
| `handlers/schemas.ts` | `llm/schemas.py` | Structured output schemas |

### Modified files

| File | Change |
|---|---|
| `electron/main.ts` | Wire real IPC handlers, remove stubs for `album:auto-tag`, `task:progress`, `task:cancel`, `dataset:status`, `config:get`, `config:set` |
| `electron/preload.ts` | Add new API methods if needed (task progress polling) |
| `package.json` | Add `better-sqlite3` dependency |

---

## Implementation Waves

### Wave 3.1 — Types + Cache + Aliases (foundation)

1. **Install deps:** `npm install better-sqlite3 opencc-js`
2. **`handlers/candidates.ts`** — Port `AlbumCandidate`, `TrackCandidate`, `LookupRequest`, `LookupSource` enum, `verifyAlbumName()`, serialize/deserialize
3. **`handlers/cache.ts`** — Port `MatchCache`: SQLite schema, `get()`, `set()`, `getAlbumState()`, `setAlbumState()`, `getLlmExtraction()`, `setLlmExtraction()`
4. **`handlers/aliases.ts`** — Port `getAliases()`, `saveAlias()`, `artistMatchesAny()`, `getAllNameVariants()`, Chinese variant helpers
5. **Tests:** Unit tests for all three

### Wave 3.2 — Fallback + Dataset readers

1. **`handlers/fallback.ts`** — Port `parseAlbumPath()`, `parseAlbumWithTags()`, `candidateFromFolder()`, `extractYearFromName()`, `cleanFolderName()`, `trackHintsFromPath()`
2. **`handlers/dataset.ts`** — Port `queryAlbum()`: read `~/.soundrobe/` SQLite via `better-sqlite3`, SC/TC variant probing, progressive prefix fallback
3. **Tests:** Unit tests with fixture SQLite database

### Wave 3.3 — External API clients

1. **`handlers/musicbrainz.ts`** — Raw `fetch()` to MusicBrainz XML API: search by artist+album, parse XML, build `AlbumCandidate[]`. Rate limiting: 1 req/sec
2. **`handlers/discogs.ts`** — Raw `fetch()` to Discogs API: search releases by artist+album, parse JSON, build `AlbumCandidate[]`. Token from config
3. **`handlers/openrouter.ts`** — Raw `fetch()` to OpenRouter API: chat completions with structured JSON response. Retry logic, usage tracking
4. **`handlers/prompts.ts`** + **`handlers/schemas.ts`** — Port prompt builders (`buildSelectionMessages`, `buildFallbackMessages`, `buildFolderExtractionMessages`), TypeScript interfaces for structured outputs
5. **Tests:** Unit tests with mocked `fetch()`

### Wave 3.4 — Orchestrator + IPC wiring

1. **`handlers/auto-tag.ts`** — Port `LookupService`: orchestrate chain, task queue with progress events, cancellation support
2. **`electron/main.ts`** — Wire `album:auto-tag`, `task:progress`, `task:cancel`, `dataset:status`, `config:get`, `config:set`
3. **`electron/preload.ts`** — No changes needed (API surface already defined)
4. **Tests:** Integration tests with mocked sub-handlers

---

## Test Strategy

| Layer | Tool | Coverage |
|---|---|---|
| **Types + Cache + Aliases** | Vitest | Pure logic, no mocks needed. Test serialize/deserialize, SQLite schema, alias matching |
| **Fallback + Dataset** | Vitest | Mock `better-sqlite3` for dataset. Test folder parsing with real paths |
| **API clients** | Vitest + mocked fetch() | Test URL construction, response parsing, error handling, rate limiting |
| **Orchestrator** | Vitest + mocked sub-handlers | Test lookup chain ordering, cache hit skip, fallback insertion, task cancellation |
| **IPC handlers** | Manual + exploratory | `electron.launch` → `snapshot` → verify stubs replaced |
