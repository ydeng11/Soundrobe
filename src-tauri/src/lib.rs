//! Soundrobe shared Rust runtime.

#[cfg(all(feature = "desktop", feature = "server"))]
compile_error!("desktop and server features are mutually exclusive");

pub mod state;

#[cfg(any(feature = "desktop", feature = "server"))]
pub mod commands;
#[cfg(feature = "desktop")]
pub mod infra;
mod error;

#[cfg(feature = "desktop")]
mod desktop;
#[cfg(feature = "desktop")]
pub use desktop::{init_logging, init_logging_in, run};

#[cfg(feature = "server")]
pub mod server;
