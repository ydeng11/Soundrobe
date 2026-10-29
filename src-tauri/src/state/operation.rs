use std::{
    fmt,
    sync::{Arc, Mutex},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationKind {
    Assistant,
    Audit,
    AutoTag,
    Organizer,
    Lyrics,
    Mutation,
}

impl OperationKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Assistant => "assistant",
            Self::Audit => "audit",
            Self::AutoTag => "auto-tag",
            Self::Organizer => "organizer",
            Self::Lyrics => "lyrics",
            Self::Mutation => "mutation",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OperationBusy {
    pub active: OperationKind,
}

impl fmt::Display for OperationBusy {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "another {} operation is already running",
            self.active.label()
        )
    }
}

impl std::error::Error for OperationBusy {}

#[derive(Debug, Clone, Default)]
pub struct OperationCoordinator {
    active: Arc<Mutex<CoordinatorState>>,
}

#[derive(Debug)]
pub struct OperationGuard {
    coordinator: OperationCoordinator,
    token: OperationToken,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OperationToken(u64);

#[derive(Debug, Default)]
struct CoordinatorState {
    next_token: u64,
    active: Option<(OperationToken, OperationKind)>,
}

impl OperationCoordinator {
    pub fn try_start(&self, kind: OperationKind) -> Result<OperationToken, OperationBusy> {
        let mut state = self
            .active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some((_, active_kind)) = state.active {
            return Err(OperationBusy {
                active: active_kind,
            });
        }
        state.next_token = state.next_token.wrapping_add(1).max(1);
        let token = OperationToken(state.next_token);
        state.active = Some((token, kind));
        Ok(token)
    }

    pub fn finish(&self, token: OperationToken) -> bool {
        let mut state = self
            .active
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.active.map(|(active, _)| active) == Some(token) {
            state.active = None;
            true
        } else {
            false
        }
    }

    pub fn try_acquire(&self, kind: OperationKind) -> Result<OperationGuard, OperationBusy> {
        let token = self.try_start(kind)?;
        Ok(OperationGuard {
            coordinator: self.clone(),
            token,
        })
    }
}

impl Drop for OperationGuard {
    fn drop(&mut self) {
        self.coordinator.finish(self.token);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn conflicting_operations_return_a_clear_busy_error() {
        let coordinator = OperationCoordinator::default();
        let guard = coordinator.try_acquire(OperationKind::AutoTag).unwrap();
        let error = coordinator
            .try_acquire(OperationKind::Assistant)
            .unwrap_err();

        assert!(error.to_string().contains("auto-tag"));
        drop(guard);
        assert!(coordinator.try_acquire(OperationKind::Mutation).is_ok());
    }

    #[tokio::test]
    async fn one_coordinator_serializes_all_mutating_operation_kinds() {
        let coordinator = OperationCoordinator::default();
        let kinds = [
            OperationKind::Assistant,
            OperationKind::Audit,
            OperationKind::AutoTag,
            OperationKind::Organizer,
            OperationKind::Lyrics,
            OperationKind::Mutation,
        ];
        let guard = coordinator.try_acquire(kinds[0]).unwrap();

        for kind in kinds.into_iter().skip(1) {
            assert!(coordinator.try_acquire(kind).is_err());
        }
        drop(guard);
        assert!(coordinator.try_acquire(OperationKind::Lyrics).is_ok());
    }

    #[test]
    fn explicit_tokens_hold_long_running_operations_until_finished() {
        let coordinator = OperationCoordinator::default();
        let token = coordinator.try_start(OperationKind::AutoTag).unwrap();

        assert!(coordinator.try_start(OperationKind::Mutation).is_err());
        assert!(coordinator.finish(token));
        assert!(!coordinator.finish(token));
        assert!(coordinator.try_start(OperationKind::Mutation).is_ok());
    }
}
