# Plan — Improve save+reload UX

## Context

The app currently re-reads all track data from disk on every sidebar navigation (album click, "All Files"). After save operations (auto-tag, audit), it re-scans the entire library even when only a few albums changed. This causes unnecessary UI blocking (`scanning: true`), progress bars, and I/O for simple view switches.

## Approach

1. **In-memory track filtering**: Load all tracks once on library open. Sidebar navigation only changes `activeAlbumPath` — the renderer filters `state.tracks` at render time. No disk reads, no `scanning` flag.
2. **Scoped refresh after bulk ops**: Auto-tag and audit re-read only modified albums, not the full library.
3. **Rollback freshness guard**: On save failure, only roll back if the in-memory state hasn't changed since the optimistic write.
4. **Manual refresh button**: `Cmd+R` / `Ctrl+R` or a toolbar button triggers a full re-scan.

## Files to modify

| File | Change |
|------|--------|
| `frontend/src/App.tsx` | Core: rewrite `handleSelectAlbum` to filter in-memory, change `handleSaveMetadata` rollback guard, scope auto-tag/audit refresh, add `handleRefresh` |
| `frontend/src/state/AppState.ts` | Add `SET_ALBUM_FILTER` action concept — but actually no new actions needed. `SET_ACTIVE_ALBUM` already exists and just sets the filter key. The change is in how the renderer uses tracks. |
| `frontend/src/components/Sidebar.tsx` | No changes needed (just provides album path, already works) |
| `frontend/src/components/TitleBar.tsx` | Add refresh button + wire up `onRefresh` prop |
| `frontend/src/components/FileGrid.tsx` | Receive `activeAlbumPath` prop to filter tracks at render time |
| Test files | Update tests to match new behavior |

## Reuse

- `handleSelectAlbum` already dispatches `SET_ACTIVE_ALBUM` — we keep that, but skip the disk read
- `loadAlbumTracks` already exists for initial load — we keep it, but only call it on library open and manual refresh
- `ScanProgressBar` — no longer triggered by navigation, only by actual scanning

## Steps

### Step 1 — Add `handleRefresh` to App.tsx

Extract the "full re-scan" logic into a reusable `handleRefresh` function that calls `scanLibrary` + `loadAlbumTracks` and sets the scanning flag.

Verification: `handleRefresh` exists as a useCallback, dispatches scanning states, calls `scanLibrary` and `loadAlbumTracks`.

**Automated test** (test/state/app-reducer.test.ts):
- Add a test for `SET_TRACKS` to verify it handles the selected track correctly after re-scan

### Step 2 — Rewrite `handleSelectAlbum` for in-memory navigation

Change `handleSelectAlbum(albumPath)` to:
- Dispatch `SET_ACTIVE_ALBUM` (keep existing)
- Dispatch `CLEAR_SELECTION` only if the selected track isn't in the new filter scope
- Do NOT set `scanning: true`
- Do NOT call `readAlbum` or `scanLibrary`
- Do NOT dispatch `SET_TRACKS`

Instead, the renderer filters at display time.

Verification:
- Clicking an album should instantly show filtered tracks, no progress bar
- Clicking "All Files" should instantly show all tracks, no re-scan

**Automated test** (test/state/app-reducer.test.ts):
- Test that `SET_ACTIVE_ALBUM` changes `activeAlbumPath` without modifying `tracks`
- Test that `SET_ACTIVE_ALBUM` clears selection when the selected track isn't in the album scope
- Test that `SET_ACTIVE_ALBUM` preserves selection when it is in scope

### Step 3 — Update FileGrid to filter by `activeAlbumPath`

Pass `activeAlbumPath` prop to FileGrid. Filter tracks in the render:
```tsx
const displayTracks = useMemo(() => {
  if (!activeAlbumPath) return tracks;
  return tracks.filter(t => t.path.startsWith(activeAlbumPath + '/'));
}, [tracks, activeAlbumPath]);
```

Verification: FileGrid shows only tracks for the active album when one is selected.

**Automated test** (test/components/FileGrid.test.tsx):
- Test that FileGrid filters by activeAlbumPath
- Test that FileGrid shows all tracks when activeAlbumPath is null

### Step 4 — Add rollback freshness guard to `handleSaveMetadata`

In `handleSaveMetadata`, before the catch block's rollback:
```ts
// Capture the optimistic state at save time
const preSaveSnapshot = { title: updatedTrack.title, artist: updatedTrack.artist, ... };
```

On failure:
```ts
catch (err) {
  const current = state.tracks.find(t => t.path === track.path);
  // Only roll back if the in-memory state still matches our optimistic write
  if (current && current.title === preSaveSnapshot.title && current.artist === preSaveSnapshot.artist && ...) {
    dispatch({ type: "UPDATE_TRACK", path: track.path, track });
  }
  // Otherwise the user made further edits — skip rollback to avoid stomping them
}
```

Better approach: store a monotonic version counter or content hash alongside each track in state to compare freshness. Simplest: track a `saveGeneration` counter that increments on each `handleSaveMetadata` call, and the catch block checks if the generation is still the latest.

Actually simplest: capture a JSON snapshot of the relevant fields at save time, and on failure compare those fields against current state:

```ts
// At save start
const snapForGuard = JSON.stringify({
  title: updatedTrack.title,
  artist: updatedTrack.artist,
  album: updatedTrack.album,
  albumArtist: updatedTrack.albumArtist,
  year: updatedTrack.year,
  trackNumber: updatedTrack.trackNumber,
  discNumber: updatedTrack.discNumber,
  genre: updatedTrack.genre,
  composer: updatedTrack.composer,
  comment: updatedTrack.comment,
});

// On failure
catch (err) {
  const current = state.tracks.find(t => t.path === track.path);
  const currentSnap = JSON.stringify({
    title: current?.title,
    artist: current?.artist,
    ...
  });
  if (currentSnap === snapForGuard) {
    dispatch({ type: "UPDATE_TRACK", path: track.path, track });
  }
}
```

Verification: If a save fails while the track has been further edited, the rollback does not fire.

**Automated test** (test/state/app-reducer.test.ts):
- Test that UPDATE_TRACK only updates when path matches
- Test the freshness check indirectly through the reducer

### Step 5 — Scoped auto-tag refresh

In `handleAutoTag`, replace the final full re-scan with scoped re-reads:
```ts
// Instead of:
const albums = await window.api.scanLibrary(state.libraryPath);
dispatch({ type: "SET_ALBUMS", albums });
// ...full re-read...

// Do:
// Re-read only the albums that were tagged
const updatedAlbums = state.albums.map(album => {
  if (targetPaths.includes(album.path)) {
    // Just update track count & artist hints from the disk
    return { ...album }; // Actually re-read from disk
  }
  return album;
});
```

Wait, this is trickier because `scanLibrary` returns album names, artist hints, and track counts. We should still call `scanLibrary` to get the album metadata, but only re-read tracks for tagged albums.

Actually, the simplest approach: after auto-tag, call `scanLibrary` to get updated album metadata (lightweight — just stats), then call `readAlbum` only for the albums that were tagged to get full track data.

```ts
// After auto-tag:
const albums = await window.api.scanLibrary(state.libraryPath);
dispatch({ type: "SET_ALBUMS", albums });

// Only re-read tracks for tagged albums
const taggedAlbumSet = new Set(targetPaths);
const updatedTracks: TrackData[] = [];
for (const album of albums) {
  if (taggedAlbumSet.has(album.path)) {
    const detail = await window.api.readAlbum(album.path);
    updatedTracks.push(...detail.tracks);
  } else {
    // Keep existing tracks for unchanged albums
    const albumTracks = state.tracks.filter(t => dirPath(t.path) === album.path);
    updatedTracks.push(...albumTracks);
  }
}
dispatch({ type: "SET_TRACKS", tracks: updatedTracks });
```

Wait, but if we keep all tracks in memory (from Step 2/3), we need a different approach for `SET_TRACKS`. Since all tracks are in `state.tracks`, we can't just replace them with only the tagged albums' tracks. We need to **merge** the re-read tracks into the existing array.

Actually, we already have `UPDATE_TRACKS` for batch updates. Let me check if that works...

`UPDATE_TRACKS` action:
```ts
case "UPDATE_TRACKS": {
  const updated = new Map(action.tracks.map((t) => [t.path, t]));
  return {
    ...state,
    tracks: state.tracks.map((t) => updated.get(t.path) ?? t),
    selectedTrack: ...,
  };
}
```

This is perfect — it updates tracks by path, preserving everything else. We can call `readAlbum` for each tagged album and dispatch `UPDATE_TRACKS` with the results.

For the album metadata (names, track counts, artist hints in sidebar), we need updated `AlbumInfo[]`. `scanLibrary` is lightweight (reads album dir names and file counts). We can call it without setting `scanning: true`.

Verification: After auto-tag, only tagged albums' tracks are re-read. Untagged albums retain their in-memory track data.

### Step 6 — Scoped audit refresh

Similar to Step 5 but for audit. The audit handler already knows which albums were audited. After audit completes, call `readAlbum` only for those.

The scope depends on context: selected tracks → find their albums; active album → just that album; entire library → re-read everything (this is rare).

Verification: After audit, only audited albums are re-read.

### Step 7 — Add refresh button to TitleBar

- Add `onRefresh` prop to TitleBar
- Add a refresh button with a reload icon
- Bind `Cmd+R` / `Ctrl+R` keyboard shortcut
- The button triggers `handleRefresh` in App.tsx

`handleRefresh` does a full re-scan:
```ts
const handleRefresh = useCallback(async () => {
  if (!state.libraryPath) return;
  dispatch({ type: "SET_SCANNING", scanning: true });
  try {
    const albums = await window.api.scanLibrary(state.libraryPath);
    dispatch({ type: "SET_ALBUMS", albums });
    await loadAlbumTracks(albums);
  } catch {
    // ignore
  } finally {
    dispatch({ type: "SET_SCANNING", scanning: false });
  }
}, [state.libraryPath, loadAlbumTracks]);
```

**Automated test** (test/components/TitleBar.test.tsx):
- Test that refresh button calls onRefresh when clicked

### Step 8 — Update keyboard shortcuts

In the `useEffect` for keyboard handlers in App.tsx, add `Cmd+R`:
```ts
if ((e.metaKey || e.ctrlKey) && e.key === "r") {
  e.preventDefault();
  handleRefresh();
}
```

### Step 9 — Update tests

- `test/state/app-reducer.test.ts`: Add tests for `SET_ACTIVE_ALBUM` behavior (preserves tracks, clears selection only when out of scope)
- `test/components/FileGrid.test.tsx`: Add tests for activeAlbumPath filtering
- `test/components/TitleBar.test.tsx`: Add test for refresh button callback

## Risks & Open Questions

1. **Memory**: Keeping all tracks in memory for large libraries. An average music library of 10,000 tracks with ~2KB per TrackData object = ~20MB. Acceptable.
2. **Stale album list**: If files are added/removed externally while the app is open, the album list in the sidebar could be stale until manual refresh. The existing `visibilitychange` handler still fires a backend `onFocus()` call — we should keep that and wire it to `handleRefresh`.
3. **FileGrid filter performance**: Filtering 10k tracks by path prefix is O(n) and instantaneous. No concern.
4. **Batch save rollback**: The same freshness guard pattern should apply to `handleBatchSave` and `handleSaveExtraTags`. Include them in Step 4 scope.

## Verification Plan

### Automated
1. `vitest run test/state/app-reducer.test.ts` — all existing + new reducer tests pass
2. `vitest run test/components/TitleBar.test.tsx` — refresh button test
3. `vitest run test/components/FileGrid.test.tsx` — filtering test
4. `vitest run` — all tests pass

### Manual
1. Open library → tracks load once
2. Click album in sidebar → tracks appear instantly, no progress bar, no scan state
3. Click "All Files" → all tracks appear instantly
4. Edit a track, blur → save happens, no reload, UI stays synced
5. Edit a track, switch to different track while save is in-flight → no stale rollback
6. Click "Auto-Tag" → tagging proceeds, only tagged albums refresh after
7. Click "Audit" → only audited albums refresh after
8. Click refresh button → full re-scan with progress bar
9. Press Cmd+R → full re-scan
