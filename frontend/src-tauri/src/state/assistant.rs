//! Assistant service configuration shared by runtime/tools.

use serde::{Deserialize, Serialize};
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
