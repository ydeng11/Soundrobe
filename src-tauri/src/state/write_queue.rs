//! Process-wide serialization and activity tracking for media writes.
//!
//! The write queue supports two locking modes:
//!   - `run()`: global lock (serialises all operations) — used by single-file
//!     writes, renames, extra-tag edits, and delete operations.
//!   - `run_for_folder()`: per-folder lock — used by batch write so tracks in
//!     *different* album folders can be written concurrently.

use std::collections::HashMap;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::{Mutex, RwLock};

/// All media mutations pass through one queue so two UI actions cannot race on
/// the same file and lifecycle code can block quit while work waits or runs.
///
/// Cloning is cheap — the inner state is shared via `Arc`. This allows passing
/// the queue to spawned tasks for concurrent per-folder batch writes.
#[derive(Clone, Default)]
pub struct WriteQueue(Arc<WriteQueueInner>);

#[derive(Default)]
struct WriteQueueInner {
    /// Shared operations may proceed under a read guard; assistant metadata
    /// batches use the write guard to keep preflight, writes, and readback atomic.
    coordination: RwLock<()>,
    /// Global serialisation gate for single-file and extra-tag operations.
    gate: Mutex<()>,
    /// Per-folder gates keyed by parent directory.
    /// Each folder gets its own `Mutex<()>` — writes to different
    /// album folders can proceed concurrently.
    folder_gates: Mutex<HashMap<PathBuf, Arc<Mutex<()>>>>,
    active: AtomicUsize,
}

impl WriteQueue {
    pub fn is_active(&self) -> bool {
        self.0.active.load(Ordering::Acquire) > 0
    }

    /// Acquires the global lock. Every caller waits for all other `run` and
    /// `run_for_folder` callers that happen to acquire the *global* gate.
    /// Per-folder operations that use `run_for_folder` do **not** block on
    /// this global gate — they only wait for their folder-specific lock.
    pub async fn run<F, T>(&self, operation: F) -> T
    where
        F: Future<Output = T>,
    {
        let activity = ActivityGuard::new(&self.0.active);
        let _coordination = self.0.coordination.read().await;
        let _gate = self.0.gate.lock().await;
        let output = operation.await;
        drop(activity);
        output
    }

    /// Acquires a lock scoped to a single album folder.
    ///
    /// Different folders can run their operations concurrently; two writes to
    /// the **same** folder are serialised (same as `run()` on the global gate).
    ///
    /// The `active` counter is still incremented so the quit guard sees
    /// in-flight work regardless of which locking mode is used.
    pub async fn run_for_folder<F, T>(&self, folder: &Path, operation: F) -> T
    where
        F: Future<Output = T>,
    {
        let activity = ActivityGuard::new(&self.0.active);
        let _coordination = self.0.coordination.read().await;
        let folder_lock = {
            let mut gates = self.0.folder_gates.lock().await;
            gates
                .entry(folder.to_path_buf())
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };
        let _guard = folder_lock.lock().await;
        let output = operation.await;
        drop(activity);
        output
    }

    /// Runs a multi-step mutation while excluding every other queued write.
    /// Callers must use the underlying dispatch functions inside this guard,
    /// because calling `run` or `run_for_folder` recursively would deadlock.
    pub async fn run_exclusive<F, T>(&self, operation: F) -> T
    where
        F: Future<Output = T>,
    {
        let activity = ActivityGuard::new(&self.0.active);
        let _coordination = self.0.coordination.write().await;
        let output = operation.await;
        drop(activity);
        output
    }

    /// Claim exclusive coordination only when no write is already active.
    /// Used by application replacement, which must reject instead of waiting
    /// behind disk work the user may still expect to complete in this process.
    pub async fn try_run_exclusive<F, T>(&self, operation: F) -> Option<T>
    where
        F: Future<Output = T>,
    {
        if self.is_active() {
            return None;
        }
        let coordination = self.0.coordination.try_write().ok()?;
        if self.is_active() {
            return None;
        }
        let activity = ActivityGuard::new(&self.0.active);
        let output = operation.await;
        drop(activity);
        drop(coordination);
        Some(output)
    }
}

struct ActivityGuard<'a> {
    active: &'a AtomicUsize,
}

impl<'a> ActivityGuard<'a> {
    fn new(active: &'a AtomicUsize) -> Self {
        active.fetch_add(1, Ordering::AcqRel);
        Self { active }
    }
}

impl Drop for ActivityGuard<'_> {
    fn drop(&mut self) {
        self.active.fetch_sub(1, Ordering::AcqRel);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;
    use std::sync::Arc;
    use tokio::sync::{Barrier, Notify};

    /// Waiting operations count as active and execution stays serialized. This
    /// is what the quit guard needs: quitting with queued work is still unsafe.
    #[tokio::test]
    async fn serializes_operations_and_tracks_waiters() {
        let queue = WriteQueue::default();
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Notify::new());

        let first_queue = queue.clone();
        let first_entered = Arc::clone(&entered);
        let first_release = Arc::clone(&release);
        let first = tokio::spawn(async move {
            first_queue
                .run(async move {
                    first_entered.wait().await;
                    first_release.notified().await;
                })
                .await;
        });
        entered.wait().await;
        assert!(queue.is_active());

        let second_queue = queue.clone();
        let second_started = Arc::new(AtomicBool::new(false));
        let second_started_task = Arc::clone(&second_started);
        let second = tokio::spawn(async move {
            second_queue
                .run(async move {
                    second_started_task.store(true, Ordering::Release);
                })
                .await;
        });
        tokio::task::yield_now().await;
        assert!(queue.is_active());
        assert!(!second_started.load(Ordering::Acquire));

        release.notify_one();
        first.await.unwrap();
        second.await.unwrap();
        assert!(!queue.is_active());
    }

    /// Two writes to the same folder are serialised (second waits for first).
    #[tokio::test]
    async fn run_for_folder_serialises_same_folder() {
        let queue = WriteQueue::default();
        let folder = Path::new("/some/album");
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Notify::new());

        let first_queue = queue.clone();
        let first_entered = Arc::clone(&entered);
        let first_release = Arc::clone(&release);
        let first_folder = folder.to_path_buf();
        let first = tokio::spawn(async move {
            first_queue
                .run_for_folder(&first_folder, async move {
                    first_entered.wait().await;
                    first_release.notified().await;
                })
                .await;
        });
        entered.wait().await;

        let second_queue = queue.clone();
        let second_started = Arc::new(AtomicBool::new(false));
        let second_started_task = Arc::clone(&second_started);
        let second_folder = folder.to_path_buf();
        let second = tokio::spawn(async move {
            second_queue
                .run_for_folder(&second_folder, async move {
                    second_started_task.store(true, Ordering::Release);
                })
                .await;
        });
        tokio::task::yield_now().await;
        // Second should NOT be started yet because same-folder lock is held.
        assert!(!second_started.load(Ordering::Acquire));

        release.notify_one();
        first.await.unwrap();
        second.await.unwrap();
        assert!(!queue.is_active());
    }

    /// Two writes to *different* folders proceed concurrently (no serialisation).
    #[tokio::test]
    async fn run_for_folder_allows_concurrent_different_folders() {
        let queue = WriteQueue::default();
        let folder_a = Path::new("/album/a");
        let folder_b = Path::new("/album/b");

        let a_entered = Arc::new(Barrier::new(2));
        let a_release = Arc::new(Notify::new());
        let b_entered = Arc::new(Barrier::new(2));
        let b_release = Arc::new(Notify::new());

        // Start operation A in folder_a
        let q_a = queue.clone();
        let f_a = folder_a.to_path_buf();
        let ae = Arc::clone(&a_entered);
        let ar = Arc::clone(&a_release);
        let a = tokio::spawn(async move {
            q_a.run_for_folder(&f_a, async move {
                ae.wait().await;
                ar.notified().await;
            })
            .await;
        });

        // Start operation B in folder_b
        let q_b = queue.clone();
        let f_b = folder_b.to_path_buf();
        let be = Arc::clone(&b_entered);
        let br = Arc::clone(&b_release);
        let b = tokio::spawn(async move {
            q_b.run_for_folder(&f_b, async move {
                be.wait().await;
                br.notified().await;
            })
            .await;
        });

        // Wait for both to enter (they should both be able to start)
        a_entered.wait().await;
        b_entered.wait().await;
        assert!(queue.is_active());

        // Release both
        a_release.notify_one();
        b_release.notify_one();
        a.await.unwrap();
        b.await.unwrap();
        assert!(!queue.is_active());
    }

    /// `run_for_folder` is tracked via `is_active` (quit guard needs this).
    #[tokio::test]
    async fn run_for_folder_tracks_active() {
        let queue = WriteQueue::default();
        let folder = Path::new("/some/album");
        // Two-phase barrier: phase 1 = child entered the operation,
        // phase 2 = main thread has checked is_active, child can exit.
        let barrier = Arc::new(Barrier::new(2));

        let q = queue.clone();
        let f = folder.to_path_buf();
        let b = Arc::clone(&barrier);
        let handle = tokio::spawn(async move {
            q.run_for_folder(&f, async move {
                // Signal: I'm inside the operation (active should be true)
                b.wait().await;
                // Wait for main thread to check is_active
                b.wait().await;
            })
            .await;
        });
        // Wait for child to enter the operation
        barrier.wait().await;
        assert!(queue.is_active());
        // Signal child it can exit
        barrier.wait().await;

        handle.await.unwrap();
        assert!(!queue.is_active());
    }

    /// Same folder key reuses the same mutex (serialisation verified above).
    /// This test checks that the HashMap entry persists across calls.
    #[tokio::test]
    async fn run_for_folder_reuses_mutex() {
        let queue = WriteQueue::default();
        let folder = Path::new("/reuse/album");

        // First call creates the entry
        queue.run_for_folder(folder, async {}).await;

        // Second call uses the same entry — we just need it not to panic.
        queue.run_for_folder(folder, async {}).await;

        // Verify: the HashMap should have exactly one entry
        let gates = queue.0.folder_gates.lock().await;
        assert_eq!(gates.len(), 1);
        assert!(gates.contains_key(folder));
    }

    /// `run` (global) and `run_for_folder` do NOT block each other
    /// (different lock domains). This is intentional so single-file writes
    /// don't block folder-scoped batches and vice versa.
    #[tokio::test]
    async fn global_and_folder_are_independent() {
        let queue = WriteQueue::default();
        let folder = Path::new("/independent/album");

        let global_entered = Arc::new(Barrier::new(2));
        let global_release = Arc::new(Notify::new());

        // Start a global-lock operation that holds the gate
        let q = queue.clone();
        let ge = Arc::clone(&global_entered);
        let gr = Arc::clone(&global_release);
        let global = tokio::spawn(async move {
            q.run(async move {
                ge.wait().await;
                gr.notified().await;
            })
            .await;
        });
        global_entered.wait().await;

        // A folder-scoped operation should be able to start concurrently
        let folder_started = Arc::new(AtomicBool::new(false));
        let q2 = queue.clone();
        let f = folder.to_path_buf();
        let started = Arc::clone(&folder_started);
        let folder_op = tokio::spawn(async move {
            q2.run_for_folder(&f, async move {
                started.store(true, Ordering::Release);
            })
            .await;
        });
        tokio::task::yield_now().await;
        // Folder op should have started (not blocked by global gate)
        assert!(folder_started.load(Ordering::Acquire));

        global_release.notify_one();
        global.await.unwrap();
        folder_op.await.unwrap();
        assert!(!queue.is_active());
    }

    #[tokio::test]
    async fn exclusive_operation_blocks_global_and_folder_writes() {
        let queue = WriteQueue::default();
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Notify::new());
        let exclusive_queue = queue.clone();
        let exclusive_entered = Arc::clone(&entered);
        let exclusive_release = Arc::clone(&release);
        let exclusive = tokio::spawn(async move {
            exclusive_queue
                .run_exclusive(async move {
                    exclusive_entered.wait().await;
                    exclusive_release.notified().await;
                })
                .await;
        });
        entered.wait().await;

        let global_started = Arc::new(AtomicBool::new(false));
        let global_flag = Arc::clone(&global_started);
        let global_queue = queue.clone();
        let global = tokio::spawn(async move {
            global_queue
                .run(async move {
                    global_flag.store(true, Ordering::Release);
                })
                .await;
        });
        let folder_started = Arc::new(AtomicBool::new(false));
        let folder_flag = Arc::clone(&folder_started);
        let folder_queue = queue.clone();
        let folder = tokio::spawn(async move {
            folder_queue
                .run_for_folder(Path::new("/album"), async move {
                    folder_flag.store(true, Ordering::Release);
                })
                .await;
        });
        tokio::task::yield_now().await;
        assert!(!global_started.load(Ordering::Acquire));
        assert!(!folder_started.load(Ordering::Acquire));

        release.notify_one();
        exclusive.await.unwrap();
        global.await.unwrap();
        folder.await.unwrap();
        assert!(global_started.load(Ordering::Acquire));
        assert!(folder_started.load(Ordering::Acquire));
    }

    /// Intent: updater installation must fail immediately when a media write
    /// is already active; waiting and then replacing the application could
    /// make the user believe the protected operation was safely coordinated.
    #[tokio::test]
    async fn try_exclusive_rejects_an_already_active_write() {
        let queue = WriteQueue::default();
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Notify::new());
        let write_queue = queue.clone();
        let write_entered = Arc::clone(&entered);
        let write_release = Arc::clone(&release);
        let write = tokio::spawn(async move {
            write_queue
                .run(async move {
                    write_entered.wait().await;
                    write_release.notified().await;
                })
                .await;
        });
        entered.wait().await;

        let result = queue.try_run_exclusive(async { "installed" }).await;
        assert_eq!(result, None);

        release.notify_one();
        write.await.unwrap();
    }

    /// Intent: once updater installation claims coordination, it is itself a
    /// protected disk operation and later writes remain blocked until it ends.
    #[tokio::test]
    async fn try_exclusive_tracks_activity_and_blocks_new_writes() {
        let queue = WriteQueue::default();
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Notify::new());
        let install_queue = queue.clone();
        let install_entered = Arc::clone(&entered);
        let install_release = Arc::clone(&release);
        let install = tokio::spawn(async move {
            install_queue
                .try_run_exclusive(async move {
                    install_entered.wait().await;
                    install_release.notified().await;
                })
                .await
        });
        entered.wait().await;
        assert!(queue.is_active());

        let write_started = Arc::new(AtomicBool::new(false));
        let write_flag = Arc::clone(&write_started);
        let write_queue = queue.clone();
        let write = tokio::spawn(async move {
            write_queue
                .run(async move { write_flag.store(true, Ordering::Release) })
                .await;
        });
        tokio::task::yield_now().await;
        assert!(!write_started.load(Ordering::Acquire));

        release.notify_one();
        assert_eq!(install.await.unwrap(), Some(()));
        write.await.unwrap();
        assert!(write_started.load(Ordering::Acquire));
        assert!(!queue.is_active());
    }
}
