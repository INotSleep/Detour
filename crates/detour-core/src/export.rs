//! Hosts back out to OpenSSH: `~/.ssh/config` entries and `ssh` commands.
//! The same as `src/lib/sshConfig.ts`, which the app's demo uses.

use std::collections::HashMap;

use crate::model::{Auth, HostView, KeyView, Proxy};

/// A name ssh takes after `Host`: no spaces, commas or patterns.
fn alias(label: &str) -> String {
    let mut out = String::new();
    let mut gap = false;
    for c in label.trim().chars() {
        if c.is_whitespace() || ",*?!\"".contains(c) {
            gap = true;
        } else {
            if gap {
                out.push('-');
                gap = false;
            }
            out.push(c);
        }
    }
    if gap {
        out.push('-');
    }
    if out.is_empty() { "host".into() } else { out }
}

/// A file name for a key, the same on every system.
pub fn key_file_name(name: &str) -> String {
    let mut out = String::new();
    let mut gap = false;
    for c in name.trim().chars() {
        if c.is_whitespace() || "\\/:*?\"<>|".contains(c) {
            gap = true;
        } else {
            if gap {
                out.push('-');
                gap = false;
            }
            out.push(c);
        }
    }
    if gap {
        out.push('-');
    }
    if out.is_empty() { "key".into() } else { out }
}

fn arg(s: &str) -> String {
    if s.contains(char::is_whitespace) { format!("\"{s}\"") } else { s.to_string() }
}

/// Who logs in: a team host may have this person's own login instead of the shared one.
fn login(h: &HostView) -> (&str, &Auth) {
    match &h.own {
        Some(o) => (&o.user, &o.auth),
        None => (&h.user, &h.auth),
    }
}

/// The keys these hosts log in with.
pub fn keys_of<'k>(hosts: &[HostView], keys: &'k [KeyView]) -> Vec<&'k KeyView> {
    let ids: Vec<&str> = hosts.iter().filter_map(|h| if let (_, Auth::Key { key_id }) = login(h) { Some(key_id.as_str()) } else { None }).collect();
    keys.iter().filter(|k| ids.contains(&k.id.as_str())).collect()
}

/// The hosts with the jump hosts they go through, which their entries name.
pub fn with_jumps(hosts: &[HostView], all: &[HostView]) -> Vec<HostView> {
    let mut out: Vec<HostView> = hosts.to_vec();
    let mut i = 0;
    while i < out.len() {
        if let Some(Proxy::Jump { host_id }) = &out[i].proxy
            && let Some(j) = all.iter().find(|h| &h.id == host_id)
            && !out.iter().any(|h| h.id == j.id)
        {
            out.push(j.clone());
        }
        i += 1;
    }
    out
}

/// The hosts as `~/.ssh/config` entries. A key file is where `key_paths` says (by key id),
/// otherwise `~/.ssh/<key name>`. Passwords can't go into an ssh config.
pub fn ssh_config(hosts: &[HostView], all: &[HostView], keys: &[KeyView], key_paths: &HashMap<String, String>) -> String {
    let mut list = with_jumps(hosts, all);
    // Hosts without a group first, then group by group, so a group's comment heads only its hosts.
    list.sort_by(|a, b| match (a.group.is_empty(), b.group.is_empty()) {
        _ if a.group == b.group => std::cmp::Ordering::Equal,
        (true, _) => std::cmp::Ordering::Less,
        (_, true) => std::cmp::Ordering::Greater,
        _ => a.group.cmp(&b.group),
    });
    let mut names: HashMap<String, String> = HashMap::new();
    let mut taken: Vec<String> = Vec::new();
    for h in &list {
        let base = alias(&h.label);
        let mut name = base.clone();
        let mut n = 2;
        while taken.contains(&name.to_lowercase()) {
            name = format!("{base}-{n}");
            n += 1;
        }
        taken.push(name.to_lowercase());
        names.insert(h.id.clone(), name);
    }

    let mut blocks: Vec<String> = Vec::new();
    let mut group: Option<&str> = None;
    for h in &list {
        let mut lines: Vec<String> = Vec::new();
        if !h.group.is_empty() && group != Some(h.group.as_str()) {
            lines.push(format!("# {}", h.group));
        }
        group = Some(h.group.as_str());
        let (user, auth) = login(h);
        lines.push(format!("Host {}", names[&h.id]));
        lines.push(format!("  HostName {}", h.address));
        lines.push(format!("  User {user}"));
        if h.port != 22 {
            lines.push(format!("  Port {}", h.port));
        }
        if let Auth::Key { key_id } = auth
            && let Some(k) = keys.iter().find(|k| &k.id == key_id)
        {
            let path = key_paths.get(&k.id).cloned().unwrap_or_else(|| format!("~/.ssh/{}", key_file_name(&k.name)));
            lines.push(format!("  IdentityFile {}", arg(&path)));
            lines.push("  IdentitiesOnly yes".into());
        }
        match &h.proxy {
            Some(Proxy::Jump { host_id }) => {
                if let Some(n) = names.get(host_id) {
                    lines.push(format!("  ProxyJump {n}"));
                }
            }
            Some(Proxy::Socks5 { address, port, .. }) => lines.push(format!("  ProxyCommand nc -X 5 -x {address}:{port} %h %p")),
            Some(Proxy::Http { address, port, user }) => {
                let p = if user.is_empty() { String::new() } else { format!(" -P {user}") };
                lines.push(format!("  ProxyCommand nc -X connect -x {address}:{port}{p} %h %p"));
            }
            None => {}
        }
        if h.forward_agent {
            lines.push("  ForwardAgent yes".into());
        }
        blocks.push(lines.join("\n"));
    }
    blocks.join("\n\n") + "\n"
}

/// One host as an `ssh` command.
pub fn ssh_command(h: &HostView, all: &[HostView]) -> String {
    let mut parts: Vec<String> = vec!["ssh".into()];
    if h.forward_agent {
        parts.push("-A".into());
    }
    // -J a,b: the first hop first.
    let mut hops: Vec<String> = Vec::new();
    let mut at = h;
    for _ in 0..8 {
        let Some(Proxy::Jump { host_id }) = &at.proxy else { break };
        let Some(j) = all.iter().find(|x| &x.id == host_id) else { break };
        let port = if j.port != 22 { format!(":{}", j.port) } else { String::new() };
        hops.insert(0, format!("{}@{}{port}", login(j).0, j.address));
        at = j;
    }
    if !hops.is_empty() {
        parts.push("-J".into());
        parts.push(hops.join(","));
    }
    if h.port != 22 {
        parts.push("-p".into());
        parts.push(h.port.to_string());
    }
    parts.push(format!("{}@{}", login(h).0, h.address));
    parts.join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host(id: &str, label: &str, group: &str, auth: Auth, proxy: Option<Proxy>) -> HostView {
        serde_json::from_value(serde_json::json!({ "id": id, "group": group, "label": label, "address": format!("10.0.0.{}", id.len()), "port": 22, "user": "u", "auth": auth, "proxy": proxy })).unwrap()
    }

    #[test]
    fn matches_the_apps_export() {
        let key: KeyView = serde_json::from_value(serde_json::json!({ "id": "k", "name": "Work key", "algorithm": "ed25519", "fingerprint": "", "publicKey": "", "hasPassphrase": false, "createdAt": 0 })).unwrap();
        let b = host("b", "bastion", "Edge", Auth::Agent, None);
        let mut d = host("dd", "db internal", "Data", Auth::Key { key_id: "k".into() }, Some(Proxy::Jump { host_id: "b".into() }));
        d.forward_agent = true;
        d.port = 2222;
        let w = host("www", "web", "", Auth::Password, Some(Proxy::Http { address: "proxy".into(), port: 3128, user: "me".into() }));
        let all = vec![b.clone(), d.clone(), w.clone()];
        let text = ssh_config(&[d.clone(), w], &all, &[key], &HashMap::new());
        assert_eq!(
            text,
            "Host web\n  HostName 10.0.0.3\n  User u\n  ProxyCommand nc -X connect -x proxy:3128 -P me %h %p\n\n# Data\nHost db-internal\n  HostName 10.0.0.2\n  User u\n  Port 2222\n  IdentityFile ~/.ssh/Work-key\n  IdentitiesOnly yes\n  ProxyJump bastion\n  ForwardAgent yes\n\n# Edge\nHost bastion\n  HostName 10.0.0.1\n  User u\n"
        );
        assert_eq!(ssh_command(&d, &all), "ssh -A -J u@10.0.0.1 -p 2222 u@10.0.0.2");
    }
}
