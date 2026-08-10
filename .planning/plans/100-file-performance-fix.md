# 100-File Performance Fix Plan

## Goal

Make albums with around 100 local audio files feel immediate in the Electron app:

- Row hover and selection should respond within one frame.
- File loading should be concurrent and, ideally, progressive.
- Tag writes should not freeze the renderer or Electron main process.
- Batch writes should use one coordinated operation instead of many single-file IPC calls.

## Current Findings

### Renderer hover and selection delay

`frontend/src/components/FileGrid.tsx` uses `transition-all duration-100` on every row. This creates an explicit 100ms hover/focus feel even when the app is otherwise idle. The row also uses `active:scale-[1.001]`, which can trigger extra paint work during interaction.

Selection also triggers cover lookup from `frontend/src/App.tsx`. `cover:data-url` may scan album files and parse metadata to find embedded art, so rapid selection can become I/O-bound.

### File loading is serialized

`App.loadAlbumTracks()` reads albums one at a time, and `readAlbum()` reads each audio file one at a time. A 100-file album therefore pays the full sum of metadata parse time instead of a bounded concurrent workload.

### Writes block the Electron main process

`frontend/electron/handlers/writer.ts` exposes async functions, but the implementation uses synchronous file and metadata work:

- `NodeID3.read` / `NodeID3.write`
- `fs.readFileSync`
- `fs.writeFileSync`
- full-buffer FLAC, OGG, and MP4 rewrites

Because this runs inside the Electron main process, long writes can make the app feel stuck and can cause window responsiveness issues.

### Batch edit uses the single-file path

`handleBatchSave()` loops over `window.api.writeTrack()` for each selected file even though `writeTracks()` already exists in the preload API. That means many IPC round trips and many renderer updates.

## Fix Phases

### Phase 1: Add Baseline Timing

Add development-only timing around the hot paths before changing behavior:

- `FileGrid` selection-to-render timing.
- `App.loadAlbumTracks`, `handleSelectTrack`, and `handleBatchSave`.
- `tracks.ts` `readAlbum` and per-file `readTrackMetadata`.
- `writer.ts` per-file write duration by extension.
- `cover.ts` `cover:data-url` duration.

Example target output:

```text
readAlbum: 100 files in 4200ms, p95 readTrackMetadata 85ms
writeTags: .flac 144ms
cover:data-url: 220ms
```

### Phase 2: Immediate Renderer Responsiveness

Update `frontend/src/components/FileGrid.tsx`:

- Remove `transition-all duration-100` from file rows.
- Prefer no transition, or use only `transition-colors duration-75`.
- Remove `active:scale-[1.001]`.
- Extract a memoized `FileGridRow` component with stable props.
- Derive the selected-path set from props with `useMemo` instead of mirroring it into local state.
- Memoize visible columns and avoid rebuilding repeated row display data where practical.

Expected result: hover and selection should feel immediate even before backend work is optimized.

### Phase 3: Cache And Defer Cover Lookup

Update selection handling in `frontend/src/App.tsx`:

- Dispatch track selection immediately.
- Cache cover URLs by album directory in a `Map<string, string | null>`.
- Debounce cover requests by about 50-100ms during rapid navigation.
- Ignore stale cover responses if the selected track changed.
- Consider a small backend cache in `frontend/electron/handlers/cover.ts` for `albumPath -> dataUrl/null`.

Expected result: clicking through rows remains fast even if embedded cover parsing is slow.

### Phase 4: Parallelize Metadata Loading With A Limit

Update `frontend/electron/handlers/tracks.ts`:

- Add a small `mapLimit` helper.
- Parse files in `readAlbum()` with concurrency around 4-6.
- Preserve output order.
- Keep existing fallback behavior for unreadable files.
- Avoid unlimited `Promise.all`, which can thrash disk and memory on large albums.

Sketch:

```ts
const tracks = await mapLimit(audioFiles, 6, async (audioFile) => {
  try {
    return await readTrackMetadata(audioFile);
  } catch {
    const fileStat = fs.statSync(audioFile);
    errorCount++;
    return minimalTrack(audioFile, fileStat.size);
  }
});
```

Expected result: album load time drops substantially without overwhelming local disk.

### Phase 5: Progressive Loading

After bounded parallel reads are stable, make large album loading progressive:

- Add an IPC progress event, such as `album:read-progress`.
- Emit parsed track batches every 10-20 files or every 100ms.
- Let the renderer display partial rows while scan/loading continues.
- Keep the final `album:read` return for compatibility.

Expected result: a 100-file album becomes usable before every file has finished parsing.

### Phase 6: Use Batch IPC For Batch Saves

Update `frontend/src/App.tsx` and `frontend/src/state/AppState.ts`:

- Change `handleBatchSave()` to call `window.api.writeTracks(updates)` once.
- Add an `UPDATE_TRACKS` reducer action so returned tracks can be applied in one render.
- Keep optimistic local updates if needed, but avoid 100 separate IPC calls and 100 separate reducer updates.

Expected result: batch save becomes less chatty and smoother.

### Phase 7: Move Heavy Writes Off The Main Process

This is the key stability fix for app freezes.

Create a worker-backed write queue:

- New worker entry, for example `frontend/electron/workers/tag-worker.ts`.
- Main-process queue module, for example `frontend/electron/handlers/write-queue.ts`.
- Worker handles:
  - `writeTrack`
  - `writeTracks`
  - `writeExtraTags`
  - `writeExtraTagsBatch`
- Start with write concurrency `1` for safety.
- Main process remains responsive while the worker performs synchronous file mutation.
- Return updated metadata after writes, either from the worker or via a controlled read queue.

Why concurrency 1: tag writing mutates files and sometimes rewrites full buffers. The primary win is moving blocking work out of the main process, not parallelizing destructive writes.

Expected result: writes may still take real disk time, but the app should not freeze.

### Phase 8: Add Write Progress And Cancel

Once writes are queued:

- Add `savingProgress: { current, total, message } | null` to app state.
- Add a write progress IPC event, such as `write:event`.
- Show progress for batch saves instead of only a generic saving pulse.
- Optionally support canceling queued, not-yet-started writes.

Expected result: long writes feel controlled and observable.

### Phase 9: Guard Window State

The hide-from-desktop symptom is likely main-process starvation, but add a defensive window-state guard in `frontend/electron/main.ts`:

- Validate restored width and height, not only x/y.
- Center with default size if saved bounds are suspicious.
- Avoid persisting invalid bounds.

Expected result: bad saved window state cannot strand the app off-screen.

## Verification

Run focused tests, then the full frontend quality gate:

```bash
cd /Users/ihelio/code/soundrobe/frontend
npm test -- test/handlers/tracks.test.ts
npm test -- test/components/FileGrid.test.tsx
npm test
npm run typecheck
npm run build
```

Manual checks:

1. Open an album or folder with around 100 audio files.
2. Confirm rows appear quickly.
3. Move cursor across rows and verify hover is immediate.
4. Click rapidly through tracks; selected row should update immediately.
5. Confirm cover art can lag slightly but never blocks selection.
6. Batch edit about 100 tracks.
7. Confirm the window remains responsive during writes.
8. Confirm written tags persist after re-read.

## Recommended Implementation Order

1. Renderer quick wins: row CSS, memoized rows, cover cache.
2. Use batch IPC for batch saves and add `UPDATE_TRACKS`.
3. Parallelize `readAlbum()` with a bounded concurrency limit.
4. Add timing instrumentation and verify before/after.
5. Move writes to a worker-thread queue.
6. Add progress events.
7. Add window-state guard.

The first three items are the fast fix. The worker-backed write queue is the deeper fix that should address full-app freezes.
