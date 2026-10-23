//! Domain model shared by every frontend.
//!
//! Field names are serialized in camelCase to match the TypeScript types in
//! `src/lib/types.ts`.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Vault {
    pub id: String,
    pub name: String,
    pub kind: VaultKind,
    pub sync: SyncBackend,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum VaultKind {
    /// Lives only on this device (optionally synced between the user's own devices).
    Personal,
    /// Shared with a team, end-to-end encrypted to every member's key.
    Team,
}

/// Where a vault's ciphertext is stored. Detour runs no server of its own:
/// teams bring their own dumb storage.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum SyncBackend {
    Local,
    Git { remote: String, branch: String },
    Folder { path: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Host {
    pub id: String,
    pub vault_id: String,
    pub group: String,
    pub label: String,
    pub address: String,
    pub port: u16,
    pub user: String,
    pub auth: Auth,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum Auth {
    Key { key_id: String },
    Password,
    Agent,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Member {
    pub id: String,
    pub name: String,
    pub role: Role,
    /// Fingerprint of the member's public key; vault data is encrypted to it.
    pub fingerprint: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Role {
    Owner,
    Admin,
    Member,
    ReadOnly,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sync_backend_serializes_with_type_tag() {
        let git = SyncBackend::Git { remote: "git@example.com:team/vault.git".into(), branch: "main".into() };
        let json = serde_json::to_string(&git).unwrap();
        assert_eq!(json, r#"{"type":"git","remote":"git@example.com:team/vault.git","branch":"main"}"#);
    }
}
