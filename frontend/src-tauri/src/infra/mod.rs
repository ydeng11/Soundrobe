//! Infrastructure for the Tauri shell.
//!
//! Per the plan: filesystem, SQLite ([`rusqlite`]), HTTP ([`reqwest`] with
//! Rust TLS), logging, artwork ([`image`]), encoding
//! ([`chardetng`]/[`encoding_rs`]), and audio tag I/O ([`lofty`]).
//! Blocking filesystem/audio/SQLite work is dispatched through bounded Rust
//! blocking tasks and never holds a SQLite lock across a network request.
//!
//! Not yet ported — populated per slice.

/// Returns true when a key is non-empty and not a redacted placeholder.
/// The renderer's `getConfig()` returns masked values starting with "****"
/// (e.g. `"****b7"`) so they MUST be rejected — the real key always lives
/// in `ConfigState` (env/config file).
pub fn is_not_redacted(key: &str) -> bool {
    !key.is_empty() && !key.starts_with("****")
}

pub mod aliases;
pub mod artwork;
pub mod encoding;
pub mod fs;
pub mod http;
pub mod logging;
pub mod openrouter;
pub mod sqlite;
pub mod tag_io;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_redacted_key() {
        assert!(!is_not_redacted("****b7"));
    }

    #[test]
    fn rejects_empty_key() {
        assert!(!is_not_redacted(""));
    }

    #[test]
    fn accepts_real_key() {
        assert!(is_not_redacted("sk-or-v1-abc123"));
    }

    #[test]
    fn rejects_short_masked_key() {
        assert!(!is_not_redacted("****k"));
    }
}
