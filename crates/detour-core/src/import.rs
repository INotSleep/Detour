//! Moving over from OpenSSH: hosts from `~/.ssh/config` (with their keys and
//! jump hosts) and pinned server keys from `~/.ssh/known_hosts`.
//!
//! The config is read as `ssh` reads it: for each host, every block whose
//! patterns match is applied in order and the first value of an option wins,
//! so `Host *` defaults at the end fill in what the host itself did not say.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use russh::keys::PublicKey;
use russh::keys::ssh_key::HashAlg;
use serde::Serialize;

use crate::model::{KnownHost, now_ms};

/// One `Host` or `Match` block: its patterns (none for `Match`, which is not followed) and options.
struct Block {
    patterns: Vec<String>,
    options: Vec<(String, String)>,
}

/// What `~/.ssh/config` says about one host.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ConfigHost {
    pub alias: String,
    pub address: String,
    pub port: u16,
    pub user: String,
    /// `IdentityFile`, expanded; `None` means ssh-agent.
    pub key_file: Option<String>,
    /// Hops of `ProxyJump`, first to last, as written.
    pub jumps: Vec<String>,
    /// Set when the host relies on something Detour does not do (`ProxyCommand`).
    pub unsupported: Option<String>,
}

/// A line of `known_hosts`, one per name it lists.
#[derive(Debug, Clone)]
pub struct KnownEntry {
    pub endpoint: String,
    pub key: String,
    pub fingerprint: String,
}

#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownHostsFile {
    #[serde(skip)]
    pub entries: Vec<KnownEntry>,
    /// Lines with hashed names: the server is not known, so they can't be used.
    pub hashed: usize,
}

pub fn ssh_dir() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|d| d.home_dir().join(".ssh"))
}

fn local_user() -> String {
    std::env::var("USER").or_else(|_| std::env::var("USERNAME")).unwrap_or_else(|_| "root".into())
}

/// `keyword value` or `keyword=value`; quotes around the value are dropped.
fn split_line(line: &str) -> Option<(String, String)> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let end = line.find(|c: char| c.is_whitespace() || c == '=').unwrap_or(line.len());
    let key = line[..end].to_lowercase();
    let rest = line[end..].trim_start_matches(|c: char| c.is_whitespace() || c == '=').trim();
    Some((key, rest.trim_matches('"').to_string()))
}

/// Words of a `Host` line, honouring quotes.
fn words(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    for c in s.chars() {
        match c {
            '"' => quoted = !quoted,
            c if c.is_whitespace() && !quoted => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            c => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn expand_home(path: &str, home: &Path) -> PathBuf {
    match path.strip_prefix("~/").or_else(|| path.strip_prefix("~\\")) {
        // Joined part by part, so the path uses this system's separators.
        Some(rest) => rest.split(['/', '\\']).filter(|p| !p.is_empty()).fold(home.to_path_buf(), |p, part| p.join(part)),
        None if path == "~" => home.to_path_buf(),
        None => PathBuf::from(path),
    }
}

/// `*` and `?` as ssh matches host patterns.
fn glob(pattern: &str, text: &str) -> bool {
    fn go(p: &[char], t: &[char]) -> bool {
        match (p.first(), t.first()) {
            (None, None) => true,
            (Some('*'), _) => go(&p[1..], t) || (!t.is_empty() && go(p, &t[1..])),
            (Some('?'), Some(_)) => go(&p[1..], &t[1..]),
            (Some(a), Some(b)) => a.eq_ignore_ascii_case(b) && go(&p[1..], &t[1..]),
            _ => false,
        }
    }
    go(&pattern.chars().collect::<Vec<_>>(), &text.chars().collect::<Vec<_>>())
}

/// A host matches a block when one pattern matches and no negated one does.
fn matches(patterns: &[String], host: &str) -> bool {
    let mut hit = false;
    for p in patterns {
        if let Some(neg) = p.strip_prefix('!') {
            if glob(neg, host) {
                return false;
            }
        } else if glob(p, host) {
            hit = true;
        }
    }
    hit
}

/// Files an `Include` names: relative ones are in `~/.ssh`; `*` and `?` in the file name are expanded.
fn included(arg: &str, ssh: &Path, home: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for word in words(arg) {
        let path = expand_home(&word, home);
        let path = if path.is_absolute() { path } else { ssh.join(path) };
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
        if name.contains(['*', '?']) {
            let Some(dir) = path.parent() else { continue };
            let mut found: Vec<PathBuf> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.file_name().is_some_and(|n| glob(&name, &n.to_string_lossy()))).collect();
            found.sort();
            out.extend(found);
        } else {
            out.push(path);
        }
    }
    out
}

fn read_blocks(path: &Path, ssh: &Path, home: &Path, depth: usize, blocks: &mut Vec<Block>) {
    let Ok(text) = std::fs::read_to_string(path) else { return };
    for line in text.lines() {
        let Some((key, value)) = split_line(line) else { continue };
        match key.as_str() {
            "host" => blocks.push(Block { patterns: words(&value), options: Vec::new() }),
            // Match conditions depend on how ssh is run: its options are not taken.
            "match" => blocks.push(Block { patterns: Vec::new(), options: Vec::new() }),
            "include" if depth < 8 => {
                for file in included(&value, ssh, home) {
                    read_blocks(&file, ssh, home, depth + 1, blocks);
                }
            }
            _ => {
                // Options before any Host line apply to every host.
                if blocks.is_empty() {
                    blocks.push(Block { patterns: vec!["*".into()], options: Vec::new() });
                }
                blocks.last_mut().unwrap().options.push((key, value));
            }
        }
    }
}

/// The hosts a config names (patterns are defaults, not hosts), with every option resolved.
pub fn read_config(path: &Path, home: &Path) -> Vec<ConfigHost> {
    let ssh = path.parent().unwrap_or(home).to_path_buf();
    let mut blocks = Vec::new();
    read_blocks(path, &ssh, home, 0, &mut blocks);
    let mut aliases: Vec<String> = Vec::new();
    for b in &blocks {
        for p in &b.patterns {
            if !p.starts_with('!') && !p.contains(['*', '?']) && !aliases.contains(p) {
                aliases.push(p.clone());
            }
        }
    }
    aliases
        .into_iter()
        .map(|alias| {
            let mut opts: BTreeMap<String, String> = BTreeMap::new();
            for b in blocks.iter().filter(|b| !b.patterns.is_empty() && matches(&b.patterns, &alias)) {
                for (k, v) in &b.options {
                    opts.entry(k.clone()).or_insert_with(|| v.clone());
                }
            }
            let address = opts.get("hostname").map(|h| h.replace("%h", &alias)).unwrap_or_else(|| alias.clone());
            let user = opts.get("user").cloned().unwrap_or_else(local_user);
            let key_file = opts.get("identityfile").filter(|f| !f.eq_ignore_ascii_case("none")).map(|f| {
                let f = f.replace("%d", &home.display().to_string()).replace("%u", &local_user()).replace("%h", &address).replace("%r", &user);
                expand_home(&f, home).display().to_string()
            });
            let jumps = opts.get("proxyjump").filter(|j| !j.eq_ignore_ascii_case("none")).map(|j| j.split(',').map(|h| h.trim().trim_start_matches("ssh://").to_string()).filter(|h| !h.is_empty()).collect()).unwrap_or_default();
            ConfigHost {
                port: opts.get("port").and_then(|p| p.parse().ok()).unwrap_or(22),
                unsupported: opts.get("proxycommand").filter(|c| !c.eq_ignore_ascii_case("none")).map(|_| "proxyCommand".to_string()),
                alias,
                address,
                user,
                key_file,
                jumps,
            }
        })
        .collect()
}

/// `[user@]host[:port]` of a `ProxyJump` hop.
pub fn parse_hop(hop: &str, default_user: &str) -> (String, String, u16) {
    let (user, rest) = match hop.rsplit_once('@') {
        Some((u, r)) => (u.to_string(), r),
        None => (default_user.to_string(), hop),
    };
    let (host, port) = match rest.strip_prefix('[').and_then(|r| r.split_once(']')) {
        Some((h, p)) => (h.to_string(), p.trim_start_matches(':').parse().unwrap_or(22)),
        None => match rest.rsplit_once(':') {
            Some((h, p)) if !h.contains(':') => (h.to_string(), p.parse().unwrap_or(22)),
            _ => (rest.to_string(), 22),
        },
    };
    (user, host, port)
}

/// Server keys from a `known_hosts` file. Hashed names, wildcards and `@revoked`
/// or `@cert-authority` lines are left out.
pub fn read_known_hosts(path: &Path) -> KnownHostsFile {
    let mut out = KnownHostsFile::default();
    let Ok(text) = std::fs::read_to_string(path) else { return out };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('@') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let (Some(names), Some(kind), Some(data)) = (parts.next(), parts.next(), parts.next()) else { continue };
        if names.starts_with("|1|") {
            out.hashed += 1;
            continue;
        }
        let key_line = format!("{kind} {data}");
        let Ok(key) = PublicKey::from_openssh(&key_line) else { continue };
        let fingerprint = key.fingerprint(HashAlg::Sha256).to_string();
        for name in names.split(',').filter(|n| !n.starts_with('!') && !n.contains(['*', '?'])) {
            let endpoint = match name.strip_prefix('[').and_then(|r| r.split_once("]:")) {
                Some((h, p)) => format!("{h}:{p}"),
                None => format!("{name}:22"),
            };
            out.entries.push(KnownEntry { endpoint, key: key_line.clone(), fingerprint: fingerprint.clone() });
        }
    }
    out
}

impl KnownEntry {
    pub fn to_known(&self) -> KnownHost {
        KnownHost { endpoint: self.endpoint.clone(), key: self.key.clone(), fingerprint: self.fingerprint.clone(), added_at: now_ms() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hops() {
        assert_eq!(parse_hop("bastion", "me"), ("me".into(), "bastion".into(), 22));
        assert_eq!(parse_hop("ops@10.0.0.1:2200", "me"), ("ops".into(), "10.0.0.1".into(), 2200));
        assert_eq!(parse_hop("[fe80::1]:22", "me"), ("me".into(), "fe80::1".into(), 22));
    }

    #[test]
    fn patterns() {
        assert!(glob("*.prod", "db.prod") && !glob("*.prod", "db.dev") && glob("web-?", "web-1"));
        assert!(!matches(&["*".into(), "!db".into()], "db") && matches(&["*".into(), "!db".into()], "web"));
    }
}
