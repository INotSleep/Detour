//! Detour core.
//!
//! Everything that is not UI lives here: the domain model, SSH sessions,
//! encrypted vaults and serverless team sync. This crate must never depend on
//! Tauri or any other UI toolkit, so the frontend can be replaced without
//! touching it.

pub mod model;

/// Version of the core crate, shown in the UI's About/status bar.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not implemented yet: {0}")]
    NotImplemented(&'static str),
}

pub type Result<T> = std::result::Result<T, Error>;
