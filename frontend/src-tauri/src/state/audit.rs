//! Shared cancellation state for audit runners.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub struct AuditState {
    current: Mutex<Option<Arc<AtomicBool>>>,
}

impl AuditState {
    pub fn start(&self) -> Option<Arc<AtomicBool>> {
        let mut current = self.current.lock().ok()?;
        let token = Arc::new(AtomicBool::new(false));
        *current = Some(Arc::clone(&token));
        Some(token)
    }

    pub fn cancel(&self) {
        let Some(token) = self
            .current
            .lock()
            .ok()
            .and_then(|mut current| current.take())
        else {
            return;
        };
        token.store(true, Ordering::Release);
    }

    pub fn finish(&self, token: &Arc<AtomicBool>) {
        let Ok(mut current) = self.current.lock() else {
            return;
        };
        if current
            .as_ref()
            .is_some_and(|active| Arc::ptr_eq(active, token))
        {
            *current = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cancel_aborts_and_clears_only_the_current_run() {
        let state = AuditState::default();
        state.cancel();
        let first = state.start().unwrap();
        assert!(!first.load(Ordering::Acquire));
        state.cancel();
        assert!(first.load(Ordering::Acquire));
        let second = state.start().unwrap();
        state.finish(&first);
        state.cancel();
        assert!(second.load(Ordering::Acquire));
    }
}
