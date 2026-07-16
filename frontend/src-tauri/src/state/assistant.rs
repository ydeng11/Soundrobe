//! Assistant service configuration shared by runtime/tools.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantServicesConfig {
    pub api_key: String,
    pub model: Option<String>,
    pub discogs_token: Option<String>,
    pub lyrics_host: Option<String>,
    pub library_path: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantServicesSnapshot {
    pub api_key: String,
    pub model: String,
    pub discogs_token: Option<String>,
    pub lyrics_host: Option<String>,
    pub library_path: Option<String>,
    pub initialized: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantAction {
    pub tag_kind: Option<String>,
    pub track_path: Option<String>,
    pub field: Option<String>,
    pub old_value: Option<String>,
    pub new_value: Option<String>,
    pub operation: Option<String>,
    pub destination_path: Option<String>,
    pub source_path: Option<String>,
    pub skip_reason: Option<String>,
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantActionBatch {
    pub id: String,
    pub created_at: String,
    pub session_id: String,
    pub kind: String,
    pub title: String,
    pub summary: String,
    pub risk_level: String,
    pub actions: Vec<AssistantAction>,
    pub reversible: bool,
    pub status: String,
}

#[derive(Default)]
struct AssistantRuntimeInner {
    active: bool,
    cancelled: bool,
    batches: Vec<AssistantActionBatch>,
    batch_errors: HashMap<String, String>,
}

#[derive(Default)]
pub struct AssistantRuntimeState {
    inner: Mutex<AssistantRuntimeInner>,
}

impl AssistantRuntimeState {
    pub fn is_active(&self) -> bool {
        self.inner.lock().is_ok_and(|state| state.active)
    }

    pub fn initialize(&self) -> bool {
        let Ok(mut state) = self.inner.lock() else {
            return false;
        };
        state.active = true;
        state.cancelled = false;
        true
    }

    pub fn cancel(&self) -> bool {
        let Ok(mut state) = self.inner.lock() else {
            return false;
        };
        if !state.active {
            return false;
        }
        state.cancelled = true;
        true
    }

    pub fn reset(&self) -> bool {
        let Ok(mut state) = self.inner.lock() else {
            return false;
        };
        if !state.active {
            return false;
        }
        state.cancelled = false;
        true
    }

    pub fn reject_batch(&self, batch_id: &str) -> Option<String> {
        let mut state = self.inner.lock().ok()?;
        if !state.active {
            return None;
        }
        let batch = state
            .batches
            .iter_mut()
            .find(|batch| batch.id == batch_id)?;
        batch.status = "rejected".to_string();
        Some(batch.title.clone())
    }

    pub fn get_batch(&self, batch_id: &str) -> Option<AssistantActionBatch> {
        self.inner
            .lock()
            .ok()?
            .batches
            .iter()
            .find(|batch| batch.id == batch_id)
            .cloned()
    }

    pub fn mark_batch_applied(&self, batch_id: &str) -> Option<String> {
        self.set_batch_status(batch_id, "applied")
    }

    pub fn mark_batch_failed(&self, batch_id: &str, error: &str) -> Option<String> {
        let mut state = self.inner.lock().ok()?;
        let batch = state
            .batches
            .iter_mut()
            .find(|batch| batch.id == batch_id)?;
        batch.status = "failed".into();
        let title = batch.title.clone();
        state.batch_errors.insert(batch_id.into(), error.into());
        Some(title)
    }

    pub fn batch_error(&self, batch_id: &str) -> Option<String> {
        self.inner.lock().ok()?.batch_errors.get(batch_id).cloned()
    }

    fn set_batch_status(&self, batch_id: &str, status: &str) -> Option<String> {
        let mut state = self.inner.lock().ok()?;
        let batch = state
            .batches
            .iter_mut()
            .find(|batch| batch.id == batch_id)?;
        batch.status = status.into();
        Some(batch.title.clone())
    }

    pub fn pending_batches(&self) -> Vec<AssistantActionBatch> {
        self.inner
            .lock()
            .ok()
            .filter(|state| state.active)
            .map(|state| {
                state
                    .batches
                    .iter()
                    .filter(|batch| batch.status == "pending")
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn add_batch(&self, batch: AssistantActionBatch) -> bool {
        let Ok(mut state) = self.inner.lock() else {
            return false;
        };
        if !state.active {
            return false;
        }
        state.batches.push(batch);
        true
    }

    #[cfg(test)]
    fn is_cancelled(&self) -> bool {
        self.inner.lock().is_ok_and(|state| state.cancelled)
    }
}

#[derive(Default)]
pub struct AssistantServicesState {
    inner: Mutex<AssistantServicesSnapshot>,
}

impl AssistantServicesState {
    pub fn initialize(&self, config: AssistantServicesConfig) -> bool {
        let Ok(mut state) = self.inner.lock() else {
            return false;
        };
        // Electron retains the previous real key/model when an empty/redacted
        // renderer value is supplied, while recreating services/credentials.
        if !config.api_key.is_empty() {
            state.api_key = config.api_key;
        }
        if let Some(model) = config.model.filter(|model| !model.is_empty()) {
            state.model = model;
        }
        state.discogs_token = config.discogs_token.filter(|value| !value.is_empty());
        state.lyrics_host = config.lyrics_host.filter(|value| !value.is_empty());
        state.library_path = config.library_path.filter(|value| !value.is_empty());
        state.initialized = true;
        true
    }

    pub fn snapshot(&self) -> Option<AssistantServicesSnapshot> {
        self.inner.lock().ok().map(|state| state.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn batch(id: &str) -> AssistantActionBatch {
        AssistantActionBatch {
            id: id.to_string(),
            created_at: "2026-01-01T00:00:00.000Z".to_string(),
            session_id: "session-1-abcdefg".to_string(),
            kind: "tag-update".to_string(),
            title: "Update".to_string(),
            summary: "One update".to_string(),
            risk_level: "low".to_string(),
            actions: Vec::new(),
            reversible: true,
            status: "pending".to_string(),
        }
    }

    #[test]
    fn runtime_cancel_reset_and_pending_batch_transitions_are_gated_by_init() {
        let state = AssistantRuntimeState::default();
        assert!(!state.add_batch(batch("before")));
        assert!(state.pending_batches().is_empty());
        assert!(!state.cancel());
        assert!(!state.is_cancelled());
        assert!(state.initialize());
        assert!(state.add_batch(batch("one")));
        assert_eq!(state.pending_batches().len(), 1);
        assert_eq!(
            state.get_batch("one").map(|batch| batch.status),
            Some("pending".into())
        );
        assert_eq!(state.mark_batch_applied("one").as_deref(), Some("Update"));
        assert_eq!(
            state.get_batch("one").map(|batch| batch.status),
            Some("applied".into())
        );
        assert_eq!(
            state.mark_batch_failed("one", "late failure").as_deref(),
            Some("Update")
        );
        assert_eq!(
            state.get_batch("one").map(|batch| batch.status),
            Some("failed".into())
        );
        assert_eq!(state.batch_error("one").as_deref(), Some("late failure"));
        assert!(state.add_batch(batch("two")));
        assert_eq!(state.reject_batch("two").as_deref(), Some("Update"));
        assert!(state.pending_batches().is_empty());
        assert!(state.cancel());
        assert!(state.is_cancelled());
        assert!(state.reset());
        assert!(!state.is_cancelled());
    }

    #[test]
    fn initializes_credentials_paths_and_replaces_service_options() {
        let state = AssistantServicesState::default();
        assert!(state.initialize(AssistantServicesConfig {
            api_key: "secret".to_string(),
            model: Some("model-a".to_string()),
            discogs_token: Some("discogs".to_string()),
            lyrics_host: Some("lyrics.example".to_string()),
            library_path: Some("/music".to_string()),
        }));
        assert_eq!(
            state.snapshot().unwrap(),
            AssistantServicesSnapshot {
                api_key: "secret".to_string(),
                model: "model-a".to_string(),
                discogs_token: Some("discogs".to_string()),
                lyrics_host: Some("lyrics.example".to_string()),
                library_path: Some("/music".to_string()),
                initialized: true,
            }
        );
        assert!(state.initialize(AssistantServicesConfig {
            api_key: String::new(),
            model: Some(String::new()),
            discogs_token: None,
            lyrics_host: None,
            library_path: None,
        }));
        let snapshot = state.snapshot().unwrap();
        assert_eq!(snapshot.api_key, "secret");
        assert_eq!(snapshot.model, "model-a");
        assert_eq!(snapshot.discogs_token, None);
        assert_eq!(snapshot.library_path, None);
    }
}
