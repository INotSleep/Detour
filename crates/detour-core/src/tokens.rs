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
    /// Change hosts, snippets and tunnels of the personal vault, create and import keys;
    /// in a team, change the hosts the token reaches.
    #[serde(rename = "vault:edit")]
    VaultEdit,
    /// Accept the key of a server no one has connected to yet (never a changed one).
    #[serde(rename = "hostkeys:new")]
    HostKeysNew,
}

/// Hosts a token reaches by what they are in, so hosts added there later are in it too.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Reach {
    /// Every host of every vault.
    All,
    /// Every host of the personal vault or of a team.
    Vault { vault_id: String },
    /// The hosts of a shared group, by its name.
    Group { vault_id: String, name: String },
    /// The hosts of a group of this person's own grouping of a team's hosts.
    MyGroup { vault_id: String, id: String },
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
    /// Hosts reached by what they are in, besides `hosts`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub reach: Vec<Reach>,
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

    /// Names this host itself (not by what it is in: see `Store::token_reaches`).
    pub fn names(&self, vault_id: &str, host_id: &str) -> bool {
        self.hosts.iter().any(|h| h.vault_id == vault_id && h.host_id == host_id)
    }

    /// Reaches every host of this vault: it may add hosts there.
    pub fn reaches_vault(&self, vault_id: &str) -> bool {
        self.reach.iter().any(|r| matches!(r, Reach::All) || matches!(r, Reach::Vault { vault_id: v } if v == vault_id))
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
    pub fn agent_token_add(&mut self, agent: &str, hosts: BTreeSet<HostRef>, reach: Vec<Reach>, scopes: BTreeSet<Scope>, expires_at: Option<Millis>) -> Result<(AgentToken, String)> {
        let secret = new_secret();
        let token = AgentToken { id: crate::new_id(), agent: agent.to_string(), hash: hash(&secret), hosts, reach, scopes, created_at: now_ms(), expires_at };
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

    /// Changes what a token reaches and allows, and until when; its secret stays.
    pub fn agent_token_set(&mut self, id: &str, hosts: BTreeSet<HostRef>, reach: Vec<Reach>, scopes: BTreeSet<Scope>, expires_at: Option<Millis>) -> Result<AgentToken> {
        let t = self.vault_mut()?.agents.iter_mut().find(|t| t.id == id).ok_or_else(|| Error::NotFound { what: "token".into() })?;
        (t.hosts, t.reach, t.scopes, t.expires_at) = (hosts, reach, scopes, expires_at);
        let t = t.clone();
        self.save()?;
        Ok(t)
    }

    /// Does the token reach this host: by naming it, or by what the host is in now.
    pub fn token_reaches(&self, t: &AgentToken, vault_id: &str, host_id: &str) -> bool {
        if t.names(vault_id, host_id) {
            return true;
        }
        if t.reach.is_empty() {
            return false;
        }
        let Some(group) = self.host_group(vault_id, host_id) else { return false };
        t.reach.iter().any(|r| match r {
            Reach::All => true,
            Reach::Vault { vault_id: v } => v == vault_id,
            Reach::Group { vault_id: v, name } => v == vault_id && !group.is_empty() && *name == group,
            Reach::MyGroup { vault_id: v, id } => {
                v == vault_id
                    && self.vault().is_ok_and(|d| d.layouts.get(v).is_some_and(|l| l.groups.iter().any(|g| g.id == *id && g.hosts.iter().any(|h| h == host_id))))
            }
        })
    }

    /// The group of a host there is (`""` for none); `None` when there is no such host.
    fn host_group(&self, vault_id: &str, host_id: &str) -> Option<String> {
        if vault_id == crate::store::PERSONAL {
            return self.vault().ok()?.hosts.iter().find(|h| h.id == host_id).map(|h| h.group.clone());
        }
        self.team(vault_id).ok()?.view().data.hosts.into_iter().find(|h| h.id == host_id).map(|h| h.group)
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
