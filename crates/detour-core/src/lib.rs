//! Detour core.
//!
//! Everything that is not UI lives here: the domain model, the encrypted
//! vault, SSH keys and SSH sessions (team sync comes later). This crate must
//! never depend on Tauri or any other UI toolkit, so the frontend can be
//! replaced without touching it.

pub mod identity;
pub mod keys;
pub mod model;
pub mod ssh;
pub mod store;
pub mod sync;
pub mod team;
pub mod tunnel;

use serde::Serialize;

/// Version of the core crate, shown in the UI's About section.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Which secret a login is missing or got wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SecretKind {
    Password,
    Passphrase,
}

/// Errors are part of the UI contract: they serialize as `{ "kind": ..., ...fields }`
/// so a frontend can react to each case (prompt for a secret, confirm a host key, ...).
#[derive(Debug, thiserror::Error, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Error {
    #[error("vault is locked")]
    Locked,
    #[error("vault is already set up")]
    AlreadyInitialized,
    #[error("vault is not set up yet")]
    NotInitialized,
    #[error("wrong master password")]
    WrongMasterPassword,
    #[error("{what} not found")]
    NotFound { what: String },
    #[error("invalid key: {reason}")]
    InvalidKey { reason: String },
    #[error("this key is protected with a passphrase")]
    PassphraseRequired,
    #[error("wrong key passphrase")]
    WrongPassphrase,
    #[error("{prompt}")]
    NeedSecret { secret: SecretKind, prompt: String },
    #[error("authentication failed")]
    AuthFailed { secret: Option<SecretKind> },
    #[error("unknown host key {fingerprint}")]
    HostKeyUnknown { fingerprint: String },
    #[error("host key changed: expected {expected}, got {actual}")]
    HostKeyChanged { expected: String, actual: String },
    #[error("{endpoint} is not responding")]
    Unreachable { endpoint: String },
    #[error("no ssh-agent is running")]
    AgentUnavailable,
    #[error("{message}")]
    Connect { message: String },
    #[error("invalid {reason}")]
    InvalidId { reason: String },
    #[error("not allowed")]
    Forbidden,
    #[error("this folder has no team vault")]
    NotATeamVault,
    #[error("the folder is not empty")]
    FolderNotEmpty,
    #[error("{path} is not available")]
    Unavailable { path: String },
    #[error("the team vault's creator key changed")]
    RootChanged,
    #[error("already a member")]
    AlreadyMember,
    /// A local port for a tunnel is taken by another program.
    #[error("port {port} is in use")]
    PortBusy { port: u16 },
    /// The server does not let us listen on its port (`ssh -R`).
    #[error("server refused to forward port {port}")]
    ForwardRefused { port: u16 },
    /// The Git repository or its server said no, or sent something unusable.
    #[error("git: {message}")]
    Git { message: String },
    #[error("{message}")]
    Io { message: String },
    #[error("{message}")]
    Internal { message: String },
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Io { message: e.to_string() }
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::Internal { message: format!("vault data: {e}") }
    }
}

pub type Result<T> = std::result::Result<T, Error>;

pub(crate) fn new_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}
