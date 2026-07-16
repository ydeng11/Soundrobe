//! Infrastructure for the Tauri shell.
//!
//! Per the plan: filesystem, SQLite ([`rusqlite`]), HTTP ([`reqwest`] with
//! Rust TLS), logging, artwork ([`image`]), encoding
//! ([`chardetng`]/[`encoding_rs`]), and audio tag I/O ([`lofty`]).
//! Blocking filesystem/audio/SQLite work is dispatched through bounded Rust
//! blocking tasks and never holds a SQLite lock across a network request.
//!
//! Not yet ported — populated per slice.

pub mod aliases;
pub mod artwork;
pub mod encoding;
pub mod fs;
pub mod http;
pub mod logging;
pub mod openrouter;
pub mod sqlite;
pub mod tag_io;
