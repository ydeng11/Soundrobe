//! Pending updater state. Checking never downloads; installation consumes the
//! checked update only after it has exclusive write coordination.

use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::{Mutex, MutexGuard};

struct Inner<T> {
    pending: Option<T>,
}

/// Generic only to make the state transitions independently testable without
/// constructing a Tauri updater resource.
pub struct PendingUpdateState<T> {
    inner: Mutex<Inner<T>>,
    check_gate: Mutex<()>,
    installing: AtomicBool,
}

impl<T> Default for PendingUpdateState<T> {
    fn default() -> Self {
        Self {
            inner: Mutex::new(Inner { pending: None }),
            check_gate: Mutex::new(()),
            installing: AtomicBool::new(false),
        }
    }
}

impl<T: Clone> PendingUpdateState<T> {
    pub async fn lock_check(&self) -> MutexGuard<'_, ()> {
        self.check_gate.lock().await
    }

    pub async fn replace(&self, pending: Option<T>) -> Result<(), &'static str> {
        let mut inner = self.inner.lock().await;
        if self.is_installing() {
            return Err("an update installation is already in progress");
        }
        inner.pending = pending;
        Ok(())
    }

    pub async fn begin_install(&self) -> Result<T, &'static str> {
        let inner = self.inner.lock().await;
        if self.is_installing() {
            return Err("an update installation is already in progress");
        }
        let pending = inner
            .pending
            .clone()
            .ok_or("there is no pending update to install")?;
        self.installing.store(true, Ordering::Release);
        Ok(pending)
    }

    pub async fn finish_install(&self, succeeded: bool) {
        let mut inner = self.inner.lock().await;
        if succeeded {
            inner.pending = None;
        }
        self.installing.store(false, Ordering::Release);
    }

    pub fn is_installing(&self) -> bool {
        self.installing.load(Ordering::Acquire)
    }
}

pub type UpdaterState = PendingUpdateState<tauri_plugin_updater::Update>;

#[cfg(test)]
mod tests {
    use super::PendingUpdateState;

    /// Intent: a failed download or signature/install error leaves the checked
    /// update available so the renderer's Retry action does not need a new
    /// network check first.
    #[tokio::test]
    async fn failed_install_retains_pending_update_for_retry() {
        let state = PendingUpdateState::default();
        state.replace(Some("v2")).await.unwrap();
        assert!(!state.is_installing());
        assert_eq!(state.begin_install().await.unwrap(), "v2");
        assert!(state.is_installing());
        state.finish_install(false).await;
        assert!(!state.is_installing());
        assert_eq!(state.begin_install().await.unwrap(), "v2");
    }

    /// Intent: only one install may own pending state, and success clears it so
    /// a stale package cannot be installed again.
    #[tokio::test]
    async fn install_is_single_owner_and_success_consumes_pending_update() {
        let state = PendingUpdateState::default();
        state.replace(Some("v2")).await.unwrap();
        assert_eq!(state.begin_install().await.unwrap(), "v2");
        assert_eq!(
            state.begin_install().await.unwrap_err(),
            "an update installation is already in progress"
        );
        state.finish_install(true).await;
        assert_eq!(
            state.begin_install().await.unwrap_err(),
            "there is no pending update to install"
        );
    }
}
