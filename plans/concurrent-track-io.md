# Plan: Concurrent Track I/O via Partitioned Map-Reduce

## Context

The app processes albums with potentially hundreds of tracks. Currently:
- **Read path (`read_album`)**: Reads tracks sequentially per album folder — one thread, synchronous.
- **Write path (`tracks_batch_write`)**: All updates go through a single global `WriteQueue` (tokio `Mutex<()>`) and `batch_write_queued` iterates them in a flat sequential `for` loop. No parallelism at all.
- **Write interface**: The renderer sends a flat `Vec<TrackUpdate>` with no folder grouping — every track is treated independently.

Neither path exploits the natural parallelism boundary: **album folders are disjoint filesystem partitions** — no two folders share files, so reads and writes can safely run concurrently across folders.

## Approach — Partitioned Map-Reduce

### Partition: Album folder is the unit of parallelism

Every track file belongs to exactly one album folder. By grouping reads/writes by folder we get natural disjoint file partitions — no locks, no duplicates, no races.

### Read Phase (already partially exists)

| Step | What | Where |
|------|------|-------|
| **Map** | One worker per album folder, sequentially reading all tracks in that folder | `commands/tracks.rs::read_album` (exists, synchronous) |
| **Reduce** | Merge per-folder `AlbumDetail` into a unified view | New function in `commands/tracks.rs` or `commands/library.rs` |

The existing `read_album` already does this per-folder. The gap is that callers currently invoke it one album at a time. We add a parallel `read_albums(album_paths: &[PathBuf]) -> Vec<AlbumDetail>` that spawns one `tokio::task::spawn_blocking` per folder (since Lofty's I/O is synchronous), then merges results.

### Write Phase (new design)

| Step | What | Where |
|------|------|-------|
| **Partition** | Group incoming `Vec<TrackUpdate>` by parent album folder | New function (extract `parent` from each `track.path`) |
| **Map** | One write worker per album folder, via per-folder partition in the `WriteQueue` | New per-folder `tokio::sync::Mutex` inside `WriteQueue`, replacing (or layered within) the single global mutex |
| **Sub-batch** | Albums >20 tracks: writes grouped in blocks of 20, *sequential within the folder worker* | Inside per-folder worker |
| **Reduce** | Collect per-folder results into a flat `Vec<TrackData>` (readback) | Merged after all folder workers complete |

### Changes to `WriteQueue`

Current `WriteQueue` has one `Mutex<()>` global gate. We need per-folder granularity:

```rust
pub struct WriteQueue {
    // Per-folder mutexes: key = canonical album path, value = folder-level lock
    folder_gates: Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>,
    active: AtomicUsize,
}
```

The `run` method becomes `run_for_folder(folder: &Path, operation: F) -> T` which:
1. Looks up or creates a per-folder `Mutex<()>` 
2. Locks only that folder's mutex (other folders proceed in parallel)
3. Runs the operation
4. Releases the lock

The global `active` counter still prevents quitting while any write is in-flight.

### Error semantics — accepted behavioral difference

**Important**: With concurrent per-folder workers, folders that succeed commit before an error in another folder is detected. This is the **same non-transactional semantics as Electron** — there is no rollback. The plan makes this explicit rather than claiming "first error stops the batch" (which was misleading for concurrent semantics). A `CancellationToken` could abort in-flight workers early when one folder fails, but rollback is impossible.

### Changes to `batch_write_queued`

Current:
```rust
async fn batch_write_queued(queue, updates) {
    queue.run(async move {
        spawn_blocking(move || {
            for update in updates {  // ALL sequential, one global lock
                write_track_dispatch(path, patch)?;
            }
        }).await
    }).await
}
```

New (using `Arc<WriteQueue>` since `WriteQueue` is not `Clone`):
```rust
async fn batch_write_queued(queue: &Arc<WriteQueue>, updates: Vec<TrackUpdate>) -> Result<(), ApiError> {
    // 1. Partition by folder (Path::parent() — no syscall)
    let folder_groups: HashMap<PathBuf, Vec<TrackUpdate>> = group_by_folder(updates);
    
    // 2. Spawn one task per folder (concurrent across folders)
    //    Each task acquires only its folder-scoped lock.
    let mut handles = Vec::new();
    for (folder, folder_updates) in folder_groups {
        let q = Arc::clone(queue);
        handles.push(tokio::spawn(async move {
            q.run_for_folder(&folder, async move {
                tokio::task::spawn_blocking(move || {
                    for batch in folder_updates.chunks(SUBBATCH_SIZE) {
                        for update in batch {
                            write_track_dispatch(Path::new(&update.path), &update.fields)?;
                        }
                    }
                    Ok::<(), ApiError>(())
                }).await
                .map_err(|e| ApiError::WriteTask(e.to_string()))?
            }).await
        }));
    }
    
    // 3. Reduce: wait for all folders. Folders that already committed
    //    succeeded before an error in another folder was detected.
    //    This matches Electron's non-transactional semantics — no rollback.
    for handle in handles {
        handle.await.map_err(|e| ApiError::WriteTask(e.to_string()))??;
    }
    Ok(())
}
```

### Readback — after ALL folder workers complete

`batch_write_with_readback` must collect readbacks **after** all folder workers finish, iterating the original `updates` paths in order to preserve the API contract:

```rust
async fn batch_write_with_readback(
    queue: &Arc<WriteQueue>,
    original_updates: Vec<TrackUpdate>,
) -> Result<Vec<TrackData>, ApiError> {
    batch_write_queued(queue, original_updates.clone()).await?;
    // Readbacks after all writes complete, in original request order
    original_updates
        .iter()
        .map(|update| read_track_with_fallback(Path::new(&update.path)))
        .collect()
}
```

Results are in request order regardless of which folder completed first.

### Sub-batch size

- Albums ≤ 20 tracks: write all sequentially in one batch (no sub-grouping needed).
- Albums > 20 tracks: write in groups of 20, sequential within the folder. This bounds memory and provides natural checkpoint granularity for large classical/jazz box sets.

The constant `SUBBATCH_SIZE = 20` goes in `infra/tag_io.rs` (the planned tag I/O module) or directly in `commands/mutations.rs`.

### No duplicate reads, no duplicate writes

- Partition key is the **parent directory** from `Path::new(&track.path).parent()` — track paths from the renderer are already absolute, so no `fs::canonicalize` syscall is needed.
- Each track path maps to exactly one folder partition.
- Each folder is dispatched to exactly one worker.
- No two workers touch the same file.

### Read-write separation

- Reads and writes are **never interleaved** — reads complete before writes begin (already the case; `batch_write_with_readback` does write then readback, but readback is per-file within the same phase).
- The advisor flagged that reads after writes are fine since they happen within the same folder lock.

## Files to modify

| File | Change |
|------|--------|
| `frontend/src-tauri/src/state/write_queue.rs` | Replace single `Mutex<()>` with `Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>`; add `run_for_folder()` method; keep `active` counter for quit guard |
| `frontend/src-tauri/src/commands/mutations.rs` | Refactor `batch_write_queued` to partition by folder → per-folder `run_for_folder` → per-folder sub-batch writes; add `group_by_folder` helper; add `SUBBATCH_SIZE` constant |
| `frontend/src-tauri/src/commands/tracks.rs` | Add `read_albums(album_paths: &[PathBuf]) -> Vec<AlbumDetail>` for parallel album reads; expose via Tauri command if needed |
| `frontend/src-tauri/src/infra/tag_io.rs` | (Optional) Place the sub-batch constant and any shared I/O utilities here since this module is the planned tag I/O home |
| `frontend/src-tauri/src/commands/library.rs` | (Optional) Wire `read_albums` into album refresh/scan flows |

## Reuse

- **Existing `write_track_dispatch`** — unchanged; called per-track from within per-folder workers
- **Existing `read_track_metadata`** — unchanged; called per-track during readback
- **Existing `read_album`** — unchanged; called per-folder from parallel `read_albums`
- **Existing `WriteQueue::is_active`** — unchanged; quit guard still works
- **Existing `validated_track_extension`** — unchanged
- **Existing `ApiError`** — unchanged
- **Existing `tracks_batch_write` Tauri command** — unchanged signature; the refactoring is entirely on the Rust side

## Steps

- [ ] **Refactor `WriteQueue`**: Replace single `Mutex<()>` with `Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>`. Add `run_for_folder(folder: &Path, operation: F) -> T` that acquires only the folder-scoped lock. Keep `is_active()` and quit guard unchanged.
- [ ] **Add `group_by_folder` helper** in `commands/mutations.rs`: takes `Vec<TrackUpdate>`, returns `HashMap<PathBuf, Vec<TrackUpdate>>` grouped by `Path::parent()`.
- [ ] **Refactor `batch_write_queued`**: Use `group_by_folder` → partition → `tokio::spawn` per folder → `run_for_folder` → `SUBBATCH_SIZE` chunks → sequential dispatch within each chunk.
- [ ] **Refactor `batch_write_with_readback`**: Keep the same signature; after batch write, collect readbacks in original request order (maintains current API contract).
- [ ] **Add `read_albums` parallel reader** in `commands/tracks.rs`: spawn blocking per folder, merge results.
- [ ] **Verify per-folder isolation**: Test that writes to two different album folders complete concurrently (check wall-clock time vs sequential baseline).
- [ ] **Verify no duplicate file writes**: Add a test that dispatches two `TrackUpdate` entries pointing to the same file path — should be rejected or serialized correctly.
- [ ] **Verify large-album sub-batching**: Add a test with >20 tracks in one folder — confirms sub-batch boundary.

## Non-goals (explicitly out of scope)

- ❌ Changing the `WriteQueue` quit-guard semantics (`is_active` still counts in-flight operations)
- ❌ Adding progress/cancellation events to batch writes (Electron doesn't expose them either)
- ❌ Changing the Lofty library or tag-writing implementations
- ❌ Parallelizing within a single album folder (files in the same folder share a mutex)
- ❌ Changing the renderer's `writeTracks` API shape — the refactoring is purely backend
- ❌ Refactoring `batch_write_extra_tags_queued` in this change (same sequential bottleneck exists there; deferred to a follow-up to keep this change focused on the main batch-write path)

## Verification

1. **`cargo test`** in `frontend/src-tauri` — existing mutation tests (52) + new folder-partition tests pass
2. **`cargo clippy`** — no new warnings
3. **Manual batch write**: Write 200 tracks across 10 albums — should complete significantly faster than current sequential time
4. **Manual large-album write**: Write 60 tracks in one album (e.g., a classical box set) — sub-batching at 20 should keep memory bounded
5. **Quit guard**: Start a write, trigger quit — the guard should block until all folder workers complete (same as current behavior)
6. **Order preservation**: `batch_write_with_readback` returns results in the same order as the input `Vec<TrackUpdate>`, regardless of folder completion order
