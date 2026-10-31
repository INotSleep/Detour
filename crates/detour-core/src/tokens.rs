//! Tokens of AI agents: what the person allowed an agent to do, on which hosts, until when.
//!
//! The token itself goes to the agent once; the vault keeps only its SHA-256, so
//! reading the vault (or a backup of it) gives no one the agent's access.
//! Tokens stay on this device: they are not part of what syncs.

use std::collections::BTreeSet;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD as B64;
use rand::RngExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::model::{Millis, now_ms};
use crate::store::Store;
use crate::{Error, Result};

/// What an agent may do. Everything it may do on a host needs that host in its token as well.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Scope {
    /// Open terminal sessions and read them; each command or keystroke it sends waits for the person.
    #[serde(rename = "connect")]
    Connect,
    /// Run commands and type into its sessions without asking.
    #[serde(rename = "exec")]
    Exec,
    #[serde(rename = "files:read")]
    FilesRead,
    #[serde(rename = "files:write")]
    FilesWrite,
    #[serde(rename = "tunnels")]
    Tunnels,
    /// Change hosts, snippets and tunnels of the personal vault, create and import keys.
    #[serde(rename = "vault:edit")]
    VaultEdit,
}

/// A host of some vault.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostRef {
    pub vault_id: String,
    pub host_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentToken {
    pub id: String,
    /// The agent's name, as it gave it (Claude Code, Codex...).
    pub agent: String,
    /// SHA-256 of the token, hex.
    pub hash: String,
    pub hosts: BTreeSet<HostRef>,
    pub scopes: BTreeSet<Scope>,
    pub created_at: Millis,
    /// `None`: until revoked.
    pub expires_at: Option<Millis>,
}

impl AgentToken {
    pub fn is_expired(&self) -> bool {
        self.expires_at.is_some_and(|t| t <= now_ms())
    }

    pub fn may(&self, scope: Scope) -> bool {
        self.scopes.contains(&scope)
    }

    pub fn reaches(&self, vault_id: &str, host_id: &str) -> bool {
        self.hosts.iter().any(|h| h.vault_id == vault_id && h.host_id == host_id)
    }
}

fn hash(token: &str) -> String {
    Sha256::digest(token.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

/// A token as the agent gets it: `dtr_` and 256 random bits.
fn new_secret() -> String {
    let bytes: [u8; 32] = rand::rng().random();
    format!("dtr_{}", B64.encode(bytes))
}

impl Store {
    pub fn agent_tokens(&self) -> Result<Vec<AgentToken>> {
        Ok(self.vault()?.agents.clone())
    }

    /// Saves a new token and returns it with the one copy of its secret.
    pub fn agent_token_add(&mut self, agent: &str, hosts: BTreeSet<HostRef>, scopes: BTreeSet<Scope>, expires_at: Option<Millis>) -> Result<(AgentToken, String)> {
        let secret = new_secret();
        let token = AgentToken { id: crate::new_id(), agent: agent.to_string(), hash: hash(&secret), hosts, scopes, created_at: now_ms(), expires_at };
        let agents = &mut self.vault_mut()?.agents;
        agents.retain(|t| !t.is_expired());
        agents.push(token.clone());
        self.save()?;
        Ok((token, secret))
    }

    /// The token with this secret, if it is one and hasn't expired.
    pub fn agent_token_find(&self, secret: &str) -> Result<Option<AgentToken>> {
        let h = hash(secret.trim());
        Ok(self.vault()?.agents.iter().find(|t| t.hash == h && !t.is_expired()).cloned())
    }

    pub fn agent_token_scope_add(&mut self, id: &str, scope: Scope) -> Result<()> {
        let t = self.vault_mut()?.agents.iter_mut().find(|t| t.id == id).ok_or_else(|| Error::NotFound { what: "token".into() })?;
        t.scopes.insert(scope);
        self.save()
    }

    /// Forgets a token; `false` when there was none.
    pub fn agent_token_remove(&mut self, id: &str) -> Result<bool> {
        let agents = &mut self.vault_mut()?.agents;
        let before = agents.len();
        agents.retain(|t| t.id != id);
        let removed = agents.len() != before;
        if removed {
            self.save()?;
        }
        Ok(removed)
    }
}
