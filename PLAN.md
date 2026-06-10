# Plan: Auto-Number Tracks Button

## Context

Users need a way to quickly assign sequential track numbers to all tracks in an album. Currently, track numbers must be entered manually one-by-one in the MetadataEditor or via Convert filename patterns. A dedicated "Number Tracks" button with user-selectable ordering rules (filename, title, existing track number, creation time, duration) would save significant manual effort.

## Approach

1. **New service** (`TrackNumberingService.ts`) — pure function that computes sort order and assigns `trackNumber`/`trackTotal` for an album's tracks. No state, no side effects → easily testable.
2. **UI** — a "Number" button in the TitleBar toolbar. Clicking it shows a small dropdown/popover listing ordering rules. Selecting a rule immediately applies the numbering.
3. **Backend reuse** — the existing `tracks:batch-write` IPC handler (`frontend/electron/handlers/tracks.ts`, registered on `"tracks:batch-write"`) already supports `trackNumber` and `trackTotal` fields. No new backend handler needed.
4. **State** — the `App.tsx` `handleNumberTracks` callback sorts tracks within the active album, builds update payloads, pushes undo snapshots, calls `window.api.writeTracks()`, and updates the store.

## Files to Modify

| File | Change |
|------|--------|
| `frontend/electron/services/TrackNumberingService.ts` | **NEW** — core numbering logic |
| `frontend/electron/services/index.ts` | **NEW** (if needed) — barrel export |
| `frontend/src/App.tsx` | Add `handleNumberTracks` callback; pass to TitleBar |
| `frontend/src/components/TitleBar.tsx` | Add "Number" button with dropdown menu |
| `frontend/test/services/TrackNumberingService.test.ts` | **NEW** — unit tests for numbering logic |
| `frontend/test/components/TitleBar.test.tsx` | Add tests for Number button & dropdown |
| `frontend/e2e/number-tracks.electron.spec.ts` | **NEW** — e2e test |

## Reuse

- **`window.api.writeTracks(updates)`** (preload.ts / `"tracks:batch-write"` handler in tracks.ts) → sends `{ path, fields: { trackNumber, trackTotal } }` for each track
- **UndoManager** — existing `PUSH_UNDO` / `handleRevert` pattern with `TrackSnapshot`
- **`basename`** from `frontend/src/utils/path.ts` — used in FileGrid already
- **TrackData interface** — already has `trackNumber`, `trackTotal`, `path`, `title`, `artist`, `duration`, `bitrate`, `sizeBytes`

## Steps

### Step 1: Create TrackNumberingService

Create `frontend/electron/services/TrackNumberingService.ts`:

```typescript
export type OrderingRule =
  | "filename-asc"
  | "filename-desc"
  | "title-asc"
  | "title-desc"
  | "existing-track-asc"
  | "existing-track-desc"
  | "creation-time-asc"   // sorted by file modification time
  | "creation-time-desc"
  | "duration-asc"
  | "duration-desc";

export interface NumberingInput {
  path: string;
  title: string | null;
  trackNumber: number | null;
  duration: number;
  // file modification time can be passed optionally
  mtimeMs?: number;
}

export interface NumberingUpdate {
  path: string;
  fields: { trackNumber: number; trackTotal: number };
}

export function computeNumberedTracks(
  tracks: NumberingInput[],
  rule: OrderingRule,
  startFrom: number = 1,
): NumberingUpdate[] { ... }
```

The function:
1. Sorts tracks according to the chosen rule (case-insensitive for text fields; undefined/null values sort to the end)
2. Assigns sequential numbers from `startFrom` (default 1)
3. Sets `trackTotal` to the total track count
4. Returns an array of `{ path, fields: { trackNumber, trackTotal } }`

Ordering rule details:
- **`filename-asc` / `filename-desc`** — extracts the basename of `path` using `path.basename`-like logic
- **`title-asc` / `title-desc`** — uses `title` field; tracks with null title sort by path basename as fallback
- **`existing-track-asc` / `existing-track-desc`** — uses current `trackNumber`; tracks with null trackNumber sort to end (asc) or beginning (desc)
- **`creation-time-asc` / `creation-time-desc`** — uses `mtimeMs` (file modification time from the filesystem, fetched from the backend via a simple stat call)
- **`duration-asc` / `duration-desc`** — uses `duration` field

### Step 2: Add IPC handler for batch stat calls (optional)

If we want "creation time" ordering, we need to get file modification times for tracks in the album. We can either:
- Read this from the existing `TrackData` (but it's not currently exposed — only `sizeBytes` and `bitrate` are)
- Add a lightweight IPC handler `"tracks:batch-stat"` that takes an array of paths and returns `{ path, mtimeMs }[]`

Actually, looking at the existing `TrackData`, `sizeBytes` is available which comes from `fs.statSync()`. We could add `mtimeMs` to `TrackData`, but that would require changes to the backend TrackData interface. 

A simpler approach: no new IPC handler. Just read the current modification times by adding a new minimal IPC handler `"tracks:stat-batch"` that does `fs.statSync` for each path. But let's keep it simple — the two most useful rules are "by filename" and "by title". We can skip "creation time" for v1, or add a lightweight call.

**Decision**: Skip "creation time" ordering for v1. The dropdown will offer: filename A-Z, filename Z-A, title A-Z, title Z-A, existing track number asc, existing track number desc, duration short→long, duration long→short.

### Step 3: Add Number button + dropdown to TitleBar

In `TitleBar.tsx`:
- Add a "Number" toolbar button (matching the style of Convert, Auto-Tag, etc.)
- When clicked, show a dropdown/popover positioned below the button listing ordering rules
- Clicking a rule calls `onNumberTracks(rule)` prop
- The dropdown closes when a rule is selected or when clicking outside

The dropdown should be lightweight (no modal dialog) — just a simple absolutely-positioned list below the button.

Props to add:
```typescript
onNumberTracks: (rule: OrderingRule) => void;
activeAlbumPath: string | null;
```

### Step 4: Add number handling to App.tsx

In `App.tsx`:
1. Import `TrackNumberingService`, `OrderingRule`, `computeNumberedTracks`
2. Add `handleNumberTracks(rule: OrderingRule)` callback:
   - Guard: requires `activeAlbumPath` and at least one track in that album
   - Get tracks scoped to active album (use `filteredTracks`)
   - Map them to `NumberingInput[]` (path, title, trackNumber, duration)
   - Call `computeNumberedTracks(tracks, rule)`
   - Build undo snapshots from current track state
   - Push undo
   - Call `window.api.writeTracks(updates)` (batch-write)
   - Update local state with the new track numbers
3. Pass `handleNumberTracks` and `activeAlbumPath` to `TitleBar`

```typescript
const handleNumberTracks = useCallback(
  async (rule: OrderingRule) => {
    if (!state.activeAlbumPath) return;
    const albumTracks = state.tracks.filter(
      (t) => t.path.startsWith(state.activeAlbumPath + "/")
    );
    if (albumTracks.length === 0) return;

    const inputs = albumTracks.map((t) => ({
      path: t.path,
      title: t.title,
      trackNumber: t.trackNumber,
      duration: t.duration,
    }));

    const updates = computeNumberedTracks(inputs, rule);

    // Undo snapshots
    const snapshots: TrackSnapshot[] = albumTracks.map((t) => ({
      path: t.path,
      fields: { trackNumber: t.trackNumber, trackTotal: t.trackTotal },
    }));

    dispatch({
      type: "PUSH_UNDO",
      description: `Number tracks (${rule})`,
      snapshots,
    });

    dispatch({ type: "SET_SAVING", saving: true });
    try {
      const results = await window.api.writeTracks(updates);
      dispatch({ type: "UPDATE_TRACKS", tracks: results });
    } catch (err) {
      dispatch({
        type: "SET_ERROR",
        error: err instanceof Error ? err.message : "Numbering failed",
      });
    } finally {
      dispatch({ type: "SET_SAVING", saving: false });
    }
  },
  [state.activeAlbumPath, state.tracks],
);
```

### Step 5: Unit tests for TrackNumberingService

`frontend/test/services/TrackNumberingService.test.ts`:
- Test sort order for each rule (filename-asc, filename-desc, title-asc, etc.)
- Test tracks with null/undefined fields sort correctly
- Test startFrom parameter (e.g., start counting from 2)
- Test trackTotal is set correctly
- Test empty array returns empty array
- Test single track returns [{ trackNumber: 1, trackTotal: 1 }]
- Test tie-breaking (same title → stable sort by path)

### Step 6: Component tests for Number button in TitleBar

`frontend/test/components/TitleBar.test.tsx`:
- Test that "Number" button renders
- Test that clicking the button shows the dropdown with ordering rule options
- Test that selecting a rule calls `onNumberTracks` with the correct rule
- Test that the dropdown closes after selection
- Test that the button is disabled when `activeAlbumPath` is null (no album selected)

### Step 7: E2E test

`frontend/e2e/number-tracks.electron.spec.ts`:
- Create a temp library with an album containing several FLAC files with unordered track numbers
- Open the app, select the album
- Click Number → select "By filename A-Z"
- Verify that track numbers are now sequential (1, 2, 3...) matching filename order

## Verification

1. **Unit tests**: `cd frontend && npx vitest run test/services/TrackNumberingService.test.ts`
2. **Component tests**: `cd frontend && npx vitest run test/components/TitleBar.test.tsx` (ensure existing tests still pass)
3. **E2E test**: `cd frontend && npx playwright test e2e/number-tracks.electron.spec.ts`
4. **Manual**: Open the app, select an album, use Number button with different ordering rules, verify track numbers update in the grid and inspector, verify undo (⌘Z) restores previous numbers
