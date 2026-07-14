//! Structured command-error mapping for the renderer facade.
//!
//! Electron handlers throw `new Error(message)`; Tauri 2 rejects `invoke`
//! with the serialized `Err` value. The [`Serialize`](impl Serialize-for-ApiError)
//! impl serializes each error to its [`Display`](std::fmt::Display) string so
//! the Tauri renderer adapter can construct the equivalent `new Error(message)`
//! and preserve the renderer's existing `try/catch` behavior.

use serde::{Serialize, Serializer};

/// All Tauri command failures. New variants are added per slice; each one
/// keeps the Electron-facing message stable while exposing a structured kind
/// for the adapter where parity demands it.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// A slice that is not yet ported. Surfaced loudly (never silently skipped)
    /// so the renderer can distinguish "unimplemented runtime" from real failure.
    #[error("not implemented: {0}")]
    NotImplemented(&'static str),

    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("tauri error: {0}")]
    Tauri(#[from] tauri::Error),

    #[error("audio metadata error: {0}")]
    Lofty(#[from] lofty::error::LoftyError),

    /// A candidate media write failed validation before replacing the source.
    #[error("media safety validation failed: {0}")]
    MediaSafety(String),

    #[error("background write task failed: {0}")]
    WriteTask(String),

    /// A second modal context menu would overwrite the first request's labels
    /// and resolve the wrong renderer promise, so reject it loudly.
    #[error("track context menu already active")]
    ContextMenuAlreadyActive,

    /// A poisoned native-menu state cannot safely route click events.
    #[error("track context menu state unavailable")]
    ContextMenuStatePoisoned,
}

/// Serialize as a plain message string; the adapter wraps it in `new Error(...)`.
impl Serialize for ApiError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::ApiError;

    /// Intent: the rejected value is a string message, matching Electron's
    /// `new Error(...).message`, so the adapter can reproduce the same message.
    #[test]
    fn serializes_to_display_string() {
        let err = ApiError::NotImplemented("library:scan parity");
        let json = serde_json::to_value(&err).unwrap();
        assert_eq!(
            json,
            serde_json::Value::String(String::from("not implemented: library:scan parity"))
        );
    }
}
