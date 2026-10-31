//! Edits from several clients at once (the desktop app, the command line, agents).
//!
//! A client that sends a whole vault also sends what it last saw of it (`base`). Only
//! what it changed since then is applied to what the vault holds now, so a host another
//! client added meanwhile is kept, not overwritten by a stale copy.

use serde::Serialize;
use serde_json::Value;

use crate::model::{HostView, KeyView, Snippet, Tunnel, VaultView};
use crate::store::VaultUpdate;

impl From<VaultView> for VaultUpdate {
    fn from(v: VaultView) -> Self {
        Self { hosts: v.hosts, keys: v.keys, snippets: v.snippets, tunnels: v.tunnels }
    }
}

trait Record: Serialize + Clone {
    fn id(&self) -> &str;
    /// Carries something the base can't show (a typed password).
    fn touched(&self) -> bool {
        false
    }
}

impl Record for HostView {
    fn id(&self) -> &str {
        &self.id
    }
    fn touched(&self) -> bool {
        self.new_password.is_some() || self.new_proxy_password.is_some()
    }
}

impl Record for KeyView {
    fn id(&self) -> &str {
        &self.id
    }
}

impl Record for Snippet {
    fn id(&self) -> &str {
        &self.id
    }
}

impl Record for Tunnel {
    fn id(&self) -> &str {
        &self.id
    }
}

fn json<T: Serialize>(t: &T) -> Value {
    serde_json::to_value(t).unwrap_or(Value::Null)
}

/// The client's order; records it changed or added are its own, the rest are as they are now.
/// Records others added since `base` follow; records others deleted stay deleted unless changed here.
fn records<T: Record>(base: &[T], next: Vec<T>, current: Vec<T>) -> Vec<T> {
    let mut out: Vec<T> = Vec::new();
    for n in next {
        match base.iter().find(|b| b.id() == n.id()) {
            Some(b) if !n.touched() && json(b) == json(&n) => {
                if let Some(c) = current.iter().find(|c| c.id() == n.id()) {
                    out.push(c.clone());
                }
            }
            _ => out.push(n),
        }
    }
    for c in current {
        if !base.iter().any(|b| b.id() == c.id()) && !out.iter().any(|o| o.id() == c.id()) {
            out.push(c);
        }
    }
    out
}

/// What to save when a client changed `base` into `next` while the vault became `current`.
pub fn merge(base: &VaultUpdate, next: VaultUpdate, current: VaultUpdate) -> VaultUpdate {
    VaultUpdate {
        hosts: records(&base.hosts, next.hosts, current.hosts),
        keys: records(&base.keys, next.keys, current.keys),
        snippets: records(&base.snippets, next.snippets, current.snippets),
        tunnels: records(&base.tunnels, next.tunnels, current.tunnels),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snip(id: &str, command: &str) -> Snippet {
        Snippet { id: id.into(), name: id.into(), command: command.into() }
    }

    fn of(snippets: Vec<Snippet>) -> VaultUpdate {
        VaultUpdate { hosts: vec![], keys: vec![], snippets, tunnels: vec![] }
    }

    #[test]
    fn keeps_what_others_did_meanwhile() {
        let base = of(vec![snip("a", "ls"), snip("b", "df"), snip("c", "top")]);
        // This client edits a and deletes b.
        let next = of(vec![snip("a", "ls -la"), snip("c", "top")]);
        // Meanwhile another client edited c, deleted nothing, and added d.
        let current = of(vec![snip("a", "ls"), snip("b", "df"), snip("c", "htop"), snip("d", "uptime")]);
        let merged = merge(&base, next, current).snippets;
        let got: Vec<(&str, &str)> = merged.iter().map(|s| (s.id.as_str(), s.command.as_str())).collect();
        assert_eq!(got, [("a", "ls -la"), ("c", "htop"), ("d", "uptime")]);
    }

    #[test]
    fn a_record_deleted_elsewhere_stays_deleted_unless_edited_here() {
        let base = of(vec![snip("a", "ls"), snip("b", "df")]);
        let next = of(vec![snip("a", "ls"), snip("b", "df -h")]);
        let current = of(vec![]);
        let merged = merge(&base, next, current).snippets;
        assert_eq!(merged.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), ["b"]);
    }
}
