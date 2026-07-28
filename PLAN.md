# Plan: Manual Search Button

## Summary

Add a **Search** button next to Auto-Tag that opens a two-step dialog flow:
1. **Search dialog** — pick provider (MusicBrainz/Discogs), enter query params, browse paginated release summaries without fetching full detail, then select a release.
2. **Confirm dialog** — fetch full release detail, run preview matching against local tracks, display an editable mapping table with per-track dropdown/field editing, then write via existing `apply_candidate_tags` with undo snapshots.

The design reuses existing search, candidate conversion, track matching, and write methods without changing current auto-tag behavior.

---

## 1. Tauri Commands (Rust)

### 1a. New command: `album:search-releases`

**Path:** `frontend/src-tauri/src/commands/album_search.rs` (new file)

**Purpose:** Lightweight paged search that returns **summaries only** — no per-result release detail fetch.

```rust
#[derive(Deserialize)]
struct SearchReleasesRequest {
    provider: String,                            // "musicbrainz" | "discogs"
    artist: String,                              // optional, empty = none
    album: String,                               // optional, empty = none
    year: Option<u32>,
    country: Option<String>,
    format: Option<String>,
    catalog_number: Option<String>,              // Discogs catno / MB catalog-number
    barcode: Option<String>,                     // MB barcode
    page: Option<u32>,                           // default 1
    page_size: Option<u32>,                      // default 10, max 50
}

#[derive(Serialize)]
struct ReleaseSearchResult {
    provider: String,
    id: String,
    kind: Option<String>,       // "release" | "master" — Discogs masters cannot be resolved as releases
    title: String,
    artist: Option<String>,
    year: Option<String>,
    country: Option<String>,
    formats: Vec<String>,
    catalog_number: Option<String>,
    barcode: Option<String>,
}

#[derive(Serialize)]
struct ReleaseSearchPage {
    results: Vec<ReleaseSearchResult>,
    page: u32,
    page_size: u32,
    total: Option<u32>,         // provider-specific count
    has_next: bool,
}

#[tauri::command]
async fn album_search_releases(
    request: SearchReleasesRequest,
    providers: State<'_, ProviderState>,
    config: State<'_, ConfigState>,
) -> Result<ReleaseSearchPage, String>
```

**Field mapping to provider APIs:**

| Field | MusicBrainz Lucene | Discogs `database/search` |
|-------|-------------------|---------------------------|
| artist | `artist:"VALUE"` | `artist` param |
| album | `release:"VALUE"` | `release_title` param |
| year | `date:VALUE*` | `year` param |
| country | `country:VALUE` | `country` param |
| format | `format:(VALUE)` | `format` param |
| catalog_number | `catno:VALUE` | `catno` param |
| barcode | `barcode:VALUE` | `barcode` param |
| combined query | Lucene `query` param | `q` param (fallback) |
| pagination | `limit` + `offset` (from `(page - 1) * page_size`) | `per_page` + `page` |

**Implementation approach:**

Add new `search_album_paged()` / `search_album_paged()` methods to `MusicBrainzClient` and `DiscogsClient` in `providers.rs`. These return `Result<...>` (whereas existing `search_album()` returns `Vec<ProviderAlbum>` silently). Existing `search_album()` and `search_album_type()` are left unchanged to preserve auto-tag regression.

- **MusicBrainz:** Build a Lucene query string from non-empty fields. Call `release` endpoint with `query`, `limit`, `offset`, `fmt=json`. Parse `releases` array and `release-count` into summary structs. No `release/{id}` detail fetch.
- **Discogs:** Call `database/search` with mapped params. Parse `results` (use existing `SearchResponse` type). Deduce `kind` from `type` field. Respect master vs release: Discogs master IDs need special handling — when opening details for a master, use `masters/{id}` endpoint. Return `pagination.items`, `pagination.pages` for `total`/`has_next`.

### 1b. New command: `album:resolve-release`

```rust
#[tauri::command]
async fn album_resolve_release(
    provider: String,          // "musicbrainz" | "discogs"
    release_id: String,
    kind: Option<String>,      // "release" | "master" — Discogs only
) -> Result<ProviderAlbum, String>
```

Reuses:
- `MusicBrainzClient::release_by_id(release_id)` — already returns `Option<ProviderAlbum>` with full tracks
- `DiscogsClient::release_metadata(release_id)` — for releases
- **Discogs master:** `discogs.get_json("masters/{id}")` → parse to `ProviderAlbum` (uses existing `parse_discogs_release` or a new `parse_discogs_master` function)

If `kind` is `"master"` for Discogs, resolve via the master endpoint and flatten track list from the main release.

### 1c. New command: `album:preview-release-match`

```rust
#[derive(Deserialize)]
struct PreviewMatchRequest {
    album_path: String,
    provider: String,
    release_id: String,
    kind: Option<String>,       // "release" | "master" — Discogs only
}

#[derive(Serialize)]
struct TrackMappingRow {
    local_index: usize,
    local_title: Option<String>,
    local_artist: Option<String>,
    remote_index: Option<usize>,   // None = unmatched
    remote_title: Option<String>,
    remote_artist: Option<String>,
    remote_track_number: Option<u32>,
    evidence: Option<String>,       // "musicbrainzTrackId" | "tagTitle" | "filenameTitle" | "position" | etc
}

#[derive(Serialize)]
struct PreviewMatchResult {
    release: ProviderAlbum,
    candidates: Vec<TrackMappingRow>,        // one per local track, in local order
    unused_remote: Vec<usize>,              // remote indices not assigned to any local track
    album_candidate: AlbumCandidate,        // initial candidate (before user edits)
}

#[tauri::command]
async fn album_preview_release_match(
    request: PreviewMatchRequest,
    providers: State<'_, ProviderState>,
) -> Result<PreviewMatchResult, String>
```

Flow:
1. Resolve release → `ProviderAlbum` (reuses `album_resolve_release` logic internally)
2. Convert to `AlbumCandidate` via `musicbrainz_candidate()` / `discogs_candidate()`
3. Load local tracks via `collect_audio_files(album_path)` and `read_album()` equivalent
4. Run `match_remote_candidate_tracks()` → get `matched_candidate` with evidence per row
5. Return `PreviewMatchResult` with the mapping rows, unused remote indices, and the initial `AlbumCandidate`

### 1d. New command: `album:search-apply-candidate`

```rust
#[tauri::command]
async fn album_search_apply_candidate(
    album_path: String,
    candidate: AlbumCandidate,   // user-edited, in local track order
) -> Result<BatchWriteResult, String>
```

**Pre-write validation:**
- Verify `album_path` exists and is a directory
- Local audio file count matches `candidate.tracks` length
- No empty `TrackCandidate` entries that would nullify existing tags (skip/None semantics must be explicit)

**Write:**
- Calls existing `apply_candidate_tags()` from `auto_tag.rs`
- Returns `BatchWriteResult` (existing type with `tracks` + `failures`)

**Undo snapshot requirement:** The caller (`App.tsx`) must capture undo snapshots before calling this command — see section 4.

### 1e. Wiring

- Add `pub mod album_search;` to `commands/mod.rs`.
- Register `album_search::commands::*` in `generate_handler!` in `lib.rs`.
- Ensure `album_search.rs` has access to `auto_tag::*` and `track_matcher::*` (both in the same crate).

### 1f. Existing provider additions (in `providers.rs`)

Add to `MusicBrainzClient`:
```rust
pub async fn search_release_summaries(
    &self,
    query_fields: HashMap<&str, &str>,
    limit: u32,
    offset: u32,
) -> Result<ReleaseSummaryPage, String>
```

Add to `DiscogsClient`:
```rust
pub async fn search_release_summaries(
    &self,
    query_params: HashMap<&str, &str>,
    page: u32,
    per_page: u32,
) -> Result<ReleaseSummaryPage, String>
```

Add `master_release_tracks(master_id)` for resolving Discogs master releases.

---

## 2. Desktop API (Renderer Contract)

**Path:** `frontend/src/shared/desktop-api.ts`

Add new types and methods:

```typescript
export interface ReleaseSearchResult {
  provider: "musicbrainz" | "discogs";
  id: string;
  kind?: string;           // "release" | "master"
  title: string;
  artist?: string;
  year?: string;
  country?: string;
  formats: string[];
  catalogNumber?: string;
  barcode?: string;
}

export interface ReleaseSearchPage {
  results: ReleaseSearchResult[];
  page: number;
  pageSize: number;
  total?: number;
  hasNext: boolean;
}

export interface TrackMappingRow {
  localIndex: number;
  localTitle?: string;
  localArtist?: string;
  remoteIndex?: number;       // undefined = unmatched
  remoteTitle?: string;
  remoteArtist?: string;
  remoteTrackNumber?: number;
  evidence?: string;
}

export interface PreviewMatchResult {
  release: ProviderAlbum;
  candidates: TrackMappingRow[];
  unusedRemoteIndices: number[];
  albumCandidate: AlbumCandidate;
}

export interface AlbumCandidate {
  artist?: string;
  artists: string[];
  album?: string;
  albumArtist?: string;
  albumArtists: string[];
  year?: string;
  genre?: string;
  musicbrainzAlbumId?: string;
  musicbrainzArtistId?: string;
  discogsReleaseId?: string;
  discogsArtistId?: string;
  tracks: TrackEdit[];
}

export interface TrackEdit {
  title?: string;
  artist?: string;
  artists: string[];
  trackNumber?: number;
  trackTotal?: number;
  discNumber?: number;
  discTotal?: number;
  musicbrainzTrackId?: string;
}
```

Methods to add:

```typescript
searchReleases: (request: {
  provider: "musicbrainz" | "discogs";
  artist: string;
  album: string;
  year?: string;
  country?: string;
  format?: string;
  catalogNumber?: string;
  barcode?: string;
  page?: number;
  pageSize?: number;
}) => Promise<ReleaseSearchPage>;

resolveRelease: (
  provider: "musicbrainz" | "discogs",
  releaseId: string,
  kind?: string
) => Promise<ProviderAlbum>;

previewReleaseMatch: (request: {
  albumPath: string;
  provider: "musicbrainz" | "discogs";
  releaseId: string;
  kind?: string;
}) => Promise<PreviewMatchResult>;

searchApplyCandidate: (
  albumPath: string,
  candidate: AlbumCandidate
) => Promise<BatchWriteResult>;
```

**Adapter wiring** (`tauri-adapter.ts`):
- `searchReleases` → `invoke("album:search-releases", { request })`
- `resolveRelease` → `invoke("album:resolve-release", { provider, releaseId, kind })`
- `previewReleaseMatch` → `invoke("album:preview-release-match", { albumPath, provider, releaseId, kind })`
- `searchApplyCandidate` → `invoke("album:search-apply-candidate", { albumPath, candidate })`

---

## 3. UI Components

### 3a. TitleBar — Search button

Add button **immediately after Auto-Tag**:

```tsx
<button
  onClick={onSearch}
  disabled={!activeAlbumPath || autoTagging || saving}
  ...
>
  <SearchIcon /> Search
</button>
```

New `TitleBarProps`:
- `onSearch: () => void` — only called when a search can start (album loaded, not already tagging)

**Enablement conditions:**
- **Enabled:** `activeAlbumPath` is set, not currently auto-tagging, not saving.
- **Disabled:** no active album, auto-tagging in progress, or conflicting writes running.

### 3b. `SearchDialog` component (new file)

**Path:** `frontend/src/components/SearchDialog.tsx`

Props: `open: boolean`, `albumPath: string`, `onClose: () => void`, `onSelectRelease: (release: ProviderAlbum) => void`

A modal dialog (`role="dialog"`, `aria-label="Search releases"`) with three phases:

**Search form:**
- Provider selector: `MusicBrainz` | `Discogs`
- Artist input (required if album is empty)
- Album input (required if artist is empty)
- Year (optional, numeric)
- Country (optional, text)
- Format (optional, text)
- Catalog Number (optional)
- Barcode (optional)
- Search button → `window.api.searchReleases({...})` — loading state during request

**Results list with pagination:**
- Cards showing: artist, album title, year, country, formats, catalog number
- Source label/badge: "MusicBrainz" or "Discogs" (with master vs release distinction)
- Pagination bar: `[< Prev] Page N [Next >]` — each page triggers a fresh `searchReleases({...page})` call (re-query, not client-side cut). Total count shown when available.
- Click card → calls `window.api.resolveRelease(provider, id, kind)` to get full detail, then navigates to detail view
- Back button from detail view → returns to results (preserves query + page number in local state)
- Empty state: "No releases found"
- Error state: inline error banner with retry

**Detail view (after resolving a release):**
- Full release info card: title, artist, year, genre, track listing
- Select button → stores provider + releaseId, calls `onSelectRelease({provider, releaseId, kind})`, closes dialog
- Back button → returns to result list

**State:** Local useState/useReducer. Search query, current page, results, and selected release are kept in component state.

### 3c. `ConfirmWriteDialog` component (new file)

**Path:** `frontend/src/components/ConfirmWriteDialog.tsx`

Props: `open: boolean`, `albumPath: string`, `albumTracks: TrackData[]`, `previewResult: PreviewMatchResult | null`, `loading: boolean`, `onConfirm: (candidate: AlbumCandidate) => void`, `onCancel: () => void`

Receives the `PreviewMatchResult` from `window.api.previewReleaseMatch` (called by the parent `App.tsx` after the search dialog selects a release).

**Editable mapping table:**

| Row | Local title | Local artist | Remote track (dropdown) | Remote title | Remote artist | Other fields |
|-----|------------|-------------|------------------------|-------------|-------------|--------------|

- One row per local track
- **Remote track column:** `<select>` dropdown listing: all remote tracks by index+title + "Do not update" option. Pre-selected based on `previewResult.candidates[].remoteIndex`. Unmatched rows pre-selected as "Do not update".
- **Editable fields:** Remote title (`<input>`), remote artist (`<input>`), track number (`<input type="number">`), disc number (`<input type="number">`) — pre-filled from the selected remote track's values
- **Duplicate prevention:** If two local rows select the same remote index, show a warning on the second row. The dropdown disables already-assigned remote indices once selected.
- **Unmatched local tracks:** "Do not update" dropdown selection means the local track's existing tags are preserved (no `TrackCandidate` entry is sent or a `None`-value entry that the writer would skip).
- Unused remote tracks shown in a summary section below the table

**Controls:**
- Summary: "N of M local tracks matched to remote tracks"
- **Cancel button:** closes dialog without writing. Verifies via prop `onCancel`.
- **Confirm & Write button:** serializes table rows into an `AlbumCandidate` (one `TrackEdit` per local track, omitting "Do not update" rows entirely). Calls `onConfirm(candidate)`.

**Write progress:**
- After `onConfirm`, parent sets `writing=true`. Shows spinner + progress bar.
- On success (`BatchWriteResult`): refresh album, close dialog.
- On error: show error message, keep dialog open, user can cancel or retry.

---

## 4. State Management (App.tsx changes)

### New state variables

```typescript
const [showSearchDialog, setShowSearchDialog] = useState(false);
const [selectedRelease, setSelectedRelease] = useState<{ provider: string; releaseId: string; kind?: string } | null>(null);
const [previewResult, setPreviewResult] = useState<PreviewMatchResult | null>(null);
const [showConfirmDialog, setShowConfirmDialog] = useState(false);
const [searchWriting, setSearchWriting] = useState(false);
```

### Flow orchestration (in App.tsx handler)

```typescript
const handleSearch = useCallback(() => {
  if (state.autoTagging || !state.activeAlbumPath) return;
  setShowSearchDialog(true);
}, [state.autoTagging, state.activeAlbumPath]);

const handleSelectRelease = useCallback(async (release: { provider: string; releaseId: string; kind?: string }) => {
  setSelectedRelease(release);
  setShowSearchDialog(false);
  setShowConfirmDialog(true);
  // Immediately fetch preview match
  setPreviewResult(null);
  const result = await window.api.previewReleaseMatch({
    albumPath: state.activeAlbumPath!,
    ...release,
  });
  setPreviewResult(result);
}, [state.activeAlbumPath]);

const handleConfirmWrite = useCallback(async (candidate: AlbumCandidate) => {
  setSearchWriting(true);
  // 1. Capture undo snapshots (mirrors handleAutoTag at App.tsx:645)
  const snapshots = await buildAutoTagUndoSnapshots(
    [state.activeAlbumPath!],
    tracks, // current visible tracks
    window.api.readAlbum,
  );
  // 2. Register undo
  undoManagerRef.current.push({
    label: "Manual search tag",
    snapshots,
    applyExtra: false,
  });
  // 3. Write via existing apply_candidate_tags wrapped in search-apply-candidate
  const result = await window.api.searchApplyCandidate(
    state.activeAlbumPath!,
    candidate,
  );
  // 4. Refresh album
  await handleRefresh();
  setShowConfirmDialog(false);
  setSearchWriting(false);
}, [state.activeAlbumPath, tracks, handleRefresh, undoManagerRef]);
```

### ActiveAlbumPath requirement

The Search button requires `activeAlbumPath` (the currently selected album in the sidebar). This is the same path used by auto-tag and ensures local tracks are available for preview matching.

### Progress verification

`apply_candidate_tags` currently calls `write_track_queued()` which does NOT emit `onTrackWriteEvent`. For v1, a simple spinner during the write RPC is sufficient. If per-track progress is needed later, a dedicated event can be added to `search-apply-candidate`.

---

## 5. Testing Plan

### 5a. Rust unit tests (`album_search.rs` inline `#[cfg(test)]`)

- **Query field mapping:** Each field (artist, album, year, country, format, catno, barcode) maps to the correct Lucene field for MusicBrainz and the correct Discogs param. Test at least one non-trivial combination.
- **Pagination:** `search_release_summaries` sends correct `limit`/`offset` (MB) and `per_page`/`page` (Discogs). Parses `release-count` / `pagination` properly.
- **No detail fetch:** Search summaries never call `release/{id}`, `releases/{id}`, or `masters/{id}` (mock the HTTP client to prove this).
- **Discogs master vs release:** Master results are returned with `kind: "master"`. Resolving a master fetches from `masters/{id}` endpoint.
- **Preview matching:** `album_preview_release_match` maps all local tracks, produces correct evidence strings, handles uneven track counts.
- **Apply validation:** `album_search_apply_candidate` rejects mismatched track count, missing album path.
- `apply_candidate_tags` integration: test that a well-formed candidate writes through the queue.
- **Error propagation:** Network errors, invalid release IDs, empty results are surfaced as `Result::Err`.

### 5b. Rust test helpers

Use `ProviderState::at()` with a local `httpmock` or `wiremock` server. Test with prerecorded response bodies or inline mock JSON.

### 5c. Vitest: `SearchDialog.test.tsx`

- Search form renders all fields: provider selector, artist, album, year, country, format, catalog number, barcode
- Provider switch shows/hides barcode label (MB = "Barcode", Discogs = "Catalog #")
- Validation: requires artist or album; shows inline error if both empty
- Submit calls `window.api.searchReleases` with correct params
- Results render as cards with title, artist, year, badges
- Pagination: previous/next buttons call searchReleases with updated page; total count displayed
- Click result card → calls `window.api.resolveRelease` → shows detail view
- Detail view back preserves search query + page number
- Empty state renders
- Error state shows retry button
- Cancel/close never calls `resolveRelease` or `previewReleaseMatch`
- Select release calls `onSelectRelease` with correct provider + releaseId

### 5d. Vitest: `ConfirmWriteDialog.test.tsx`

- Renders mapping table with one row per local track
- Dropdown selects remote track (pre-selected from preview result)
- Editing title/artist/track number fields works
- "Do not update" renders as no-op (not included in serialized candidate)
- Duplicate remote assignment shows warning
- Unused remote tracks section displayed
- **Cancel:** calls `onCancel`, does not write, no undo snapshots captured
- **Confirm:** serializes edited rows into `AlbumCandidate`, calls `onConfirm`
- Serialization omits "Do not update" rows, includes user-edited values
- Progress/error states

### 5e. Vitest: `TitleBar.test.tsx`

- Search button rendered immediately after Auto-Tag
- Disabled when `activeAlbumPath` is null
- Disabled when `autoTagging` is true
- Disabled when `saving` is true
- Enabled when album selected and not tagging/saving

### 5f. Vitest: `App.test.tsx` integration

- Search button → opens SearchDialog
- SearchDialog selection → opens ConfirmWriteDialog with preview loading
- Confirm write → captures undo snapshots → calls `searchApplyCandidate` → refreshes album
- Cancel at confirm step → no undo snapshots, no write call
- Verify undo snapshot capture matches existing `buildAutoTagUndoSnapshots` pattern

### 5g. E2E (`frontend/e2e-tauri/workflows.spec.ts`)

- Open album → click Search → select MusicBrainz → fill artist+album → see paginated results → click release → see detail → back → select different release → confirm with edited mapping → verify metadata written via tag read
- Same flow with Discogs (including master-type results)
- Test cancel at confirm step → verify no metadata changes on disk
- Test field-level edit in mapping table (change remote title before confirm) → written title matches edit

**E2E mock strategy:** Use `ProviderState::at()` constructor to inject a local mock server URL. Run a lightweight HTTP mock (e.g. `basic-script` or `mockoon`-adjacent Rust `wiremock`) that returns prerecorded JSON fixtures for MusicBrainz and Discogs search/resolve endpoints. The e2e test sets the provider base URL via an environment variable. This ensures deterministic, offline test execution.

### 5h. Regression
- Full existing test suite passes: `just fe-check`
- All existing auto-tag Rust tests unchanged

---

## 6. Files Changed / Created

### New files:
| File | Purpose |
|------|---------|
| `frontend/src-tauri/src/commands/album_search.rs` | Tauri commands: search, resolve, preview, apply |
| `frontend/src/components/SearchDialog.tsx` | Search + browse releases UI with pagination |
| `frontend/src/components/ConfirmWriteDialog.tsx` | Editable track mapping table + confirm write |
| `frontend/test/components/SearchDialog.test.tsx` | Vitest suite for search dialog |
| `frontend/test/components/ConfirmWriteDialog.test.tsx` | Vitest suite for confirm dialog |
| (in `providers.rs`) new methods `search_release_summaries` on both clients | Paged search helpers |

### Modified files:
| File | Changes |
|------|---------|
| `frontend/src-tauri/src/state/providers.rs` | Add `search_release_summaries` to MB + Discogs clients; add `master_release_tracks` helper |
| `frontend/src-tauri/src/commands/mod.rs` | Add `pub mod album_search;` |
| `frontend/src-tauri/src/lib.rs` | Register `album_search::*` in `generate_handler!` |
| `frontend/src/shared/desktop-api.ts` | Add types + 4 new methods to `DesktopAPI` |
| `frontend/src/shared/tauri-adapter.ts` | Wire new methods to `invoke` |
| `frontend/src/components/TitleBar.tsx` | Add Search button next to Auto-Tag, `onSearch` prop |
| `frontend/src/App.tsx` | Add search/confirm state, orchestration handlers, undo capture |
| `frontend/test/components/TitleBar.test.tsx` | Test Search button presence + enablement states |

---

## 7. Constraints & Assumptions

1. **Button placement:** Immediately after Auto-Tag in `TitleBar.tsx`.
2. **Enablement:** Disabled without `activeAlbumPath`, during auto-tagging, or during save operations. Enabled when an album is selected and no conflicting operations are running.
3. **Paged search summaries:** `album:search-releases` never fetches full release detail for each result. Detail fetches happen only in `album:resolve-release` (triggered by clicking a result) and `album:preview-release-match` (triggered by selecting a release).
4. **Preview matching runs on Rust side** in `album:preview-release-match`, reusing `musicbrainz_candidate`, `discogs_candidate`, and `match_remote_candidate_tracks`. The initial candidate is returned to the renderer for editing.
5. **Editable mapping:** Renderer-side. Dropdown per local track selects remote track or "Do not update". Editable fields: title, artist, track/disc numbers. Warnings on duplicate remote assignment. "Do not update" rows are omitted from the final `AlbumCandidate`.
6. **Never write without explicit confirm.** Search, browse, preview, and editing never trigger a write. Only the confirm button in `ConfirmWriteDialog` does.
7. **Undo snapshots** are captured by `App.tsx` before calling `search-apply-candidate`, mirroring the `handleAutoTag` pattern at `App.tsx:645`.
8. **Write progress:** `apply_candidate_tags` does not emit per-track events. V1 uses a spinner. A progress event can be added later.
9. **Discogs master releases** are identified by `kind: "master"` in search results. Resolving a master fetches from the `masters/{id}` endpoint.
10. **No existing auto-tag behavior changes.** New provider methods (`search_release_summaries`) are additive; existing `search_album`, `search_album_type`, and `search_artist_by_name` are unchanged.
11. **E2E deterministic mocking** uses `ProviderState::at()` with a configurable mock server URL. Prerecorded JSON fixtures are served by `wiremock` (Rust) or an equivalent Node HTTP mock started before the e2e test.
12. **New state is local to components** — no changes to `AppState.ts` reducer.
