//! Process-wide serialization and activity tracking for media writes.

use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::Mutex;

/// All media mutations pass through one queue so two UI actions cannot race on
/// the same file and lifecycle code can block quit while work waits or runs.
#[derive(Default)]
pub struct WriteQueue {
    gate: Mutex<()>,
    active: AtomicUsize,
}

impl WriteQueue {
    pub fn is_active(&self) -> bool {
        self.active.load(Ordering::Acquire) > 0
    }

    pub async fn run<F, T>(&self, operation: F) -> T
    where
        F: Future<Output = T>,
    {
        let activity = ActivityGuard::new(&self.active);
        let _gate = self.gate.lock().await;
        let output = operation.await;
        drop(activity);
        output
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
        let queue = Arc::new(WriteQueue::default());
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Notify::new());

        let first_queue = Arc::clone(&queue);
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

        let second_queue = Arc::clone(&queue);
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
}
