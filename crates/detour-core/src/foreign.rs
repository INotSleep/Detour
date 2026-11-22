//! Hosts saved in other SSH clients: PuTTY and KiTTY sessions, MobaXterm bookmarks,
//! SecureCRT sessions, a CSV list (Termius, Royal TS, Remote Desktop Manager) and an
//! OpenSSH config file. Each is read into the same `ForeignHost`, and the store brings the
//! chosen ones in (see `Store::foreign_import`).
//!
//! Only what these clients keep in the open comes along. MobaXterm and SecureCRT encrypt
//! saved passwords with their own keys, so hosts from them come without passwords.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::model::TunnelKind;
use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Client {
    Putty,
    Kitty,
    Mobaxterm,
    Securecrt,
    /// A CSV list with a header row: Termius, Royal TS, Remote Desktop Manager export it.
    Csv,
    /// An OpenSSH config file anywhere (Termius exports one).
    SshConfig,
}

impl Client {
    pub fn name(self) -> &'static str {
        match self {
            Client::Putty => "PuTTY",
            Client::Kitty => "KiTTY",
            Client::Mobaxterm => "MobaXterm",
            Client::Securecrt => "SecureCRT",
            Client::Csv => "CSV",
            Client::SshConfig => "SSH config",
        }
    }
}

/// A host on the way: another host of the same list (`label`), or one written out.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hop {
    /// The `key` of another host of the list, when the hop is one.
    pub key: Option<String>,
    pub address: String,
    pub port: u16,
    pub user: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForeignTunnel {
    pub kind: TunnelKind,
    pub bind_port: u16,
    pub target: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ForeignProxy {
    Socks5 { address: String, port: u16, user: String },
    Http { address: String, port: u16, user: String },
}

/// One host as another client keeps it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ForeignHost {
    /// Unique within the list: what the UI chooses by.
    pub key: String,
    pub label: String,
    pub address: String,
    pub port: u16,
    pub user: String,
    /// The folder it is in there; empty for none.
    pub group: String,
    /// A private key file (OpenSSH or PuTTY .ppk).
    pub key_file: Option<String>,
    #[serde(skip)]
    pub password: Option<String>,
    pub has_password: bool,
    pub jump: Option<Hop>,
    pub proxy: Option<ForeignProxy>,
    pub tunnels: Vec<ForeignTunnel>,
}

impl ForeignHost {
    fn new(key: String, label: String, address: String, port: u16, user: String, group: String) -> Self {
        ForeignHost { key, label, address, port, user, group, key_file: None, password: None, has_password: false, jump: None, proxy: None, tunnels: vec![] }
    }
}

/// What was read, and from where.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Found {
    /// A file, a folder or a registry key, to show.
    pub source: String,
    pub hosts: Vec<ForeignHost>,
}

fn not_found(what: impl Into<String>) -> Error {
    Error::NotFound { what: what.into() }
}

fn home() -> PathBuf {
    directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf()).unwrap_or_default()
}

/// Text in UTF-8, UTF-16 (with a byte order mark, as .reg files are) or Windows-1252.
pub fn decode_text(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        let units: Vec<u16> = rest.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        return String::from_utf16_lossy(&units);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let units: Vec<u16> = rest.chunks_exact(2).map(|c| u16::from_be_bytes([c[0], c[1]])).collect();
        return String::from_utf16_lossy(&units);
    }
    let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        // Windows-1252 is Latin-1 but for 0x80-0x9F, which hold no letters of host names.
        Err(_) => bytes.iter().map(|&b| b as char).collect(),
    }
}

fn read_text(path: &Path) -> Result<String> {
    let bytes = std::fs::read(path).map_err(|_| not_found(path.display().to_string()))?;
    Ok(decode_text(&bytes))
}

/// `%20` and the like, as PuTTY writes session names.
fn url_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or(""), 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `user@host` in one field, as PuTTY allows.
fn split_user(host: &str, user: &str) -> (String, String) {
    match host.rsplit_once('@') {
        Some((u, h)) if user.is_empty() => (u.to_string(), h.to_string()),
        Some((_, h)) => (user.to_string(), h.to_string()),
        None => (user.to_string(), host.to_string()),
    }
}

/// Where the reading starts when the person points at nothing.
pub fn default_source(client: Client) -> Option<PathBuf> {
    let h = home();
    let candidates: Vec<PathBuf> = match client {
        Client::Putty => vec![h.join(".putty").join("sessions")],
        Client::Kitty => vec![],
        Client::Mobaxterm => {
            let mut v = vec![];
            if let Some(d) = directories::BaseDirs::new() {
                v.push(d.config_dir().join("MobaXterm").join("MobaXterm.ini"));
            }
            v.push(h.join("Documents").join("MobaXterm").join("MobaXterm.ini"));
            v
        }
        Client::Securecrt => {
            let mut v = vec![];
            if let Some(d) = directories::BaseDirs::new() {
                v.push(d.config_dir().join("VanDyke").join("Config").join("Sessions"));
            }
            v.push(h.join("Library/Application Support/VanDyke/SecureCRT/Config/Sessions"));
            v.push(h.join(".vandyke/SecureCRT/Config/Sessions"));
            v
        }
        Client::Csv | Client::SshConfig => vec![],
    };
    candidates.into_iter().find(|p| p.exists())
}

/// Reads a client's hosts from `path`, or from where the client keeps them.
pub fn read(client: Client, path: Option<&Path>) -> Result<Found> {
    match client {
        Client::Putty | Client::Kitty => read_putty(client, path),
        Client::Mobaxterm => {
            let p = path.map(Path::to_path_buf).or_else(|| default_source(client)).ok_or_else(|| not_found("MobaXterm.ini"))?;
            Ok(Found { source: p.display().to_string(), hosts: parse_mobaxterm(&read_text(&p)?) })
        }
        Client::Securecrt => {
            let p = path.map(Path::to_path_buf).or_else(|| default_source(client)).ok_or_else(|| not_found("SecureCRT Sessions"))?;
            let hosts = if p.is_dir() { read_securecrt_dir(&p) } else { parse_securecrt_xml(&read_text(&p)?) };
            Ok(Found { source: p.display().to_string(), hosts })
        }
        Client::Csv => {
            let p = path.ok_or_else(|| not_found("CSV"))?;
            Ok(Found { source: p.display().to_string(), hosts: parse_csv(&read_text(p)?)? })
        }
        Client::SshConfig => {
            let p = path.ok_or_else(|| not_found("config"))?;
            Ok(Found { source: p.display().to_string(), hosts: from_ssh_config(p) })
        }
    }
}

/* ---------- PuTTY and KiTTY ---------- */

const PUTTY_KEY: &str = r"Software\SimonTatham\PuTTY\Sessions";
const KITTY_KEY: &str = r"Software\9bis.com\KiTTY\Sessions";

fn read_putty(client: Client, path: Option<&Path>) -> Result<Found> {
    if let Some(p) = path {
        let sessions = if p.is_dir() { read_session_dir(p) } else { parse_reg(&read_text(p)?) };
        return Ok(Found { source: p.display().to_string(), hosts: putty_hosts(sessions) });
    }
    #[cfg(windows)]
    {
        let key = if client == Client::Kitty { KITTY_KEY } else { PUTTY_KEY };
        if let Some(text) = export_registry(key) {
            return Ok(Found { source: format!(r"HKEY_CURRENT_USER\{key}"), hosts: putty_hosts(parse_reg(&text)) });
        }
    }
    let _ = KITTY_KEY;
    let p = default_source(client).ok_or_else(|| not_found(format!("{} sessions", client.name())))?;
    Ok(Found { source: p.display().to_string(), hosts: putty_hosts(read_session_dir(&p)) })
}

/// The registry key as `reg export` writes it; None when it isn't there.
#[cfg(windows)]
fn export_registry(key: &str) -> Option<String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let file = std::env::temp_dir().join(format!("detour-import-{}.reg", crate::new_id()));
    let ok = std::process::Command::new("reg")
        .args(["export", &format!(r"HKCU\{key}"), &file.display().to_string(), "/y"])
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    let text = ok.then(|| std::fs::read(&file).ok()).flatten().map(|b| decode_text(&b));
    let _ = std::fs::remove_file(&file);
    text
}

type Sessions = Vec<(String, HashMap<String, String>)>;

/// A .reg file: one section per session, its values as text (dwords as numbers).
pub fn parse_reg(text: &str) -> Sessions {
    let mut out: Sessions = vec![];
    // Long values may go on over lines ending in a backslash.
    let joined = text.replace("\\\r\n  ", "").replace("\\\n  ", "");
    for line in joined.lines() {
        let line = line.trim();
        if let Some(section) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            let lower = section.to_ascii_lowercase();
            let name = ["\\sessions\\"].iter().find_map(|m| lower.find(m).map(|i| &section[i + m.len()..]));
            if let Some(name) = name.filter(|n| !n.contains('\\')) {
                out.push((url_decode(name), HashMap::new()));
            }
            continue;
        }
        let Some((_, values)) = out.last_mut() else { continue };
        let Some(rest) = line.strip_prefix('"') else { continue };
        let Some((name, value)) = rest.split_once("\"=") else { continue };
        let value = if let Some(v) = value.strip_prefix("dword:") {
            u32::from_str_radix(v.trim(), 16).map(|n| n.to_string()).unwrap_or_default()
        } else if let Some(v) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
            v.replace("\\\\", "\u{0}").replace("\\\"", "\"").replace('\u{0}', "\\")
        } else {
            continue;
        };
        values.insert(name.to_string(), value);
    }
    out
}

/// A folder of session files: PuTTY on Linux and macOS (`Key=value`), portable KiTTY (`Key\value\`).
fn read_session_dir(dir: &Path) -> Sessions {
    let Ok(entries) = std::fs::read_dir(dir) else { return vec![] };
    let mut out = vec![];
    for e in entries.flatten() {
        if !e.path().is_file() {
            continue;
        }
        let Ok(text) = read_text(&e.path()) else { continue };
        let mut values = HashMap::new();
        for line in text.lines() {
            // KiTTY writes `Key\value\`, PuTTY `Key=value`.
            let parsed = match line.strip_suffix('\\').and_then(|l| l.split_once('\\')) {
                Some(kv) if !kv.0.contains('=') => Some(kv),
                _ => line.split_once('='),
            };
            if let Some((k, v)) = parsed {
                values.insert(k.to_string(), v.to_string());
            }
        }
        out.push((url_decode(&e.file_name().to_string_lossy()), values));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// `L8080=localhost:80,R9000=localhost:9000,D1080` (each may start with 4 or 6 for the IP version).
pub fn parse_forwards(s: &str) -> Vec<ForeignTunnel> {
    s.split(',')
        .filter_map(|f| {
            let f = f.trim().trim_start_matches(['4', '6']);
            let (kind, rest) = match f.chars().next()? {
                'L' => (TunnelKind::Local, &f[1..]),
                'R' => (TunnelKind::Remote, &f[1..]),
                'D' => (TunnelKind::Dynamic, &f[1..]),
                _ => return None,
            };
            let (bind, target) = rest.split_once('=').map(|(b, t)| (b, Some(t.trim().to_string()))).unwrap_or((rest, None));
            // A bind address may come first: 127.0.0.1:8080.
            let port = bind.rsplit(':').next()?.trim().parse().ok()?;
            Some(ForeignTunnel { kind, bind_port: port, target: target.filter(|t| !t.is_empty()) })
        })
        .collect()
}

fn putty_hosts(sessions: Sessions) -> Vec<ForeignHost> {
    let mut out = vec![];
    for (name, v) in sessions {
        let get = |k: &str| v.get(k).map(|s| s.trim().to_string()).unwrap_or_default();
        let protocol = get("Protocol");
        if name == "Default Settings" || !(protocol.is_empty() || protocol == "ssh") {
            continue;
        }
        let (user, address) = split_user(&get("HostName"), &get("UserName"));
        if address.is_empty() {
            continue;
        }
        let port = get("PortNumber").parse().unwrap_or(22);
        let mut h = ForeignHost::new(name.clone(), name.clone(), address, port, user, String::new());
        h.key_file = Some(get("PublicKeyFile")).filter(|f| !f.is_empty());
        h.tunnels = parse_forwards(&get("PortForwardings"));
        let (ph, pp, pu) = (get("ProxyHost"), get("ProxyPort").parse().unwrap_or(0), get("ProxyUsername"));
        if !ph.is_empty() && pp > 0 {
            match get("ProxyMethod").as_str() {
                "2" => h.proxy = Some(ForeignProxy::Socks5 { address: ph, port: pp, user: pu }),
                "3" => h.proxy = Some(ForeignProxy::Http { address: ph, port: pp, user: pu }),
                // An SSH proxy (PuTTY 0.77 and later): the host goes through another.
                "6" => h.jump = Some(Hop { key: None, address: ph, port: pp, user: pu }),
                _ => {}
            }
        }
        out.push(h);
    }
    out
}

/* ---------- MobaXterm ---------- */

fn moba_unescape(s: &str) -> String {
    s.replace("__PTVIRG__", ";").replace("__DBLQUO__", "\"").replace("__PIPE__", "|").replace("__PERCENT__", "%").replace("__DIEZE__", "#")
}

fn moba_path(s: &str) -> String {
    let s = moba_unescape(s);
    let profile = std::env::var("USERPROFILE").unwrap_or_else(|_| home().display().to_string());
    s.replace("_ProfileDir_", &profile).replace("_CurrentDrive_", "C")
}

/// MobaXterm.ini or a .mxtsessions export: SSH bookmarks (`#109#`), their folders as groups.
pub fn parse_mobaxterm(text: &str) -> Vec<ForeignHost> {
    let mut out = vec![];
    let mut in_bookmarks = false;
    let mut group = String::new();
    for line in text.lines() {
        let line = line.trim_end();
        if let Some(section) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            in_bookmarks = section.to_ascii_lowercase().starts_with("bookmarks");
            group.clear();
            continue;
        }
        if !in_bookmarks {
            continue;
        }
        let Some((name, value)) = line.split_once('=') else { continue };
        if name == "SubRep" {
            group = value.replace('\\', " / ");
            continue;
        }
        let Some(i) = value.find("#109#") else { continue };
        let first = value[i + 5..].split('#').next().unwrap_or("");
        let f: Vec<&str> = first.split('%').collect();
        let at = |n: usize| f.get(n).map(|s| s.trim()).unwrap_or("");
        let address = at(1).to_string();
        if address.is_empty() {
            continue;
        }
        let label = name.trim().trim_end_matches('\\').trim().to_string();
        let key = format!("{group}/{label}");
        let mut h = ForeignHost::new(key, label, address, at(2).parse().unwrap_or(22), at(3).to_string(), group.clone());
        h.key_file = Some(moba_path(at(14))).filter(|p| !p.is_empty());
        let gw = moba_unescape(at(8));
        if let Some(gh) = gw.split('|').next().filter(|g| !g.is_empty()) {
            let port = moba_unescape(at(9)).split('|').next().and_then(|p| p.parse().ok()).unwrap_or(22);
            let user = moba_unescape(at(10)).split('|').next().unwrap_or("").to_string();
            h.jump = Some(Hop { key: None, address: gh.to_string(), port, user });
        }
        out.push(h);
    }
    out
}

/* ---------- SecureCRT ---------- */

fn crt_host(key: String, label: String, group: String, values: &HashMap<String, String>) -> Option<ForeignHost> {
    let get = |k: &str| values.get(k).map(|s| s.trim().to_string()).unwrap_or_default();
    let protocol = get("Protocol Name");
    if !(protocol.is_empty() || protocol.starts_with("SSH")) {
        return None;
    }
    let address = get("Hostname");
    if address.is_empty() {
        return None;
    }
    let port = get("[SSH2] Port").parse().ok().filter(|&p: &u16| p > 0).unwrap_or(22);
    let mut h = ForeignHost::new(key, label, address, port, get("Username"), group);
    // "C:\keys\id_rsa::rawkey": the part before :: is the file.
    let identity = get("Identity Filename V2");
    let identity = identity.split("::").next().unwrap_or("").to_string();
    h.key_file = Some(identity).filter(|f| !f.is_empty());
    Some(h)
}

/// One session .ini: `S:"Hostname"=…`, `D:"[SSH2] Port"=00000016` (hex).
pub fn parse_securecrt_ini(text: &str) -> HashMap<String, String> {
    let mut values = HashMap::new();
    for line in text.lines() {
        let Some((kind, rest)) = line.split_once(":\"") else { continue };
        let Some((name, value)) = rest.split_once("\"=") else { continue };
        let value = match kind {
            "D" => u32::from_str_radix(value.trim(), 16).map(|n| n.to_string()).unwrap_or_default(),
            "S" => value.to_string(),
            _ => continue,
        };
        values.insert(name.to_string(), value);
    }
    values
}

fn read_securecrt_dir(root: &Path) -> Vec<ForeignHost> {
    let mut out = vec![];
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            let name = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            let Some(label) = name.strip_suffix(".ini") else { continue };
            if label.starts_with("__") || label == "Default" {
                continue;
            }
            let rel = p.parent().and_then(|d| d.strip_prefix(root).ok()).map(|d| d.components().map(|c| c.as_os_str().to_string_lossy().to_string()).collect::<Vec<_>>().join(" / ")).unwrap_or_default();
            let Ok(text) = read_text(&p) else { continue };
            if let Some(h) = crt_host(format!("{rel}/{label}"), label.to_string(), rel, &parse_securecrt_ini(&text)) {
                out.push(h);
            }
        }
    }
    out.sort_by(|a, b| (&a.group, &a.label).cmp(&(&b.group, &b.label)));
    out
}

fn xml_unescape(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// SecureCRT's XML export: nested `<key name="…">`, values in `<string>` and `<dword>`.
pub fn parse_securecrt_xml(text: &str) -> Vec<ForeignHost> {
    let mut out = vec![];
    // Open keys, each with its values.
    let mut stack: Vec<(String, HashMap<String, String>)> = vec![];
    let mut rest = text;
    while let Some(i) = rest.find('<') {
        rest = &rest[i..];
        let Some(end) = rest.find('>') else { break };
        let tag = &rest[1..end];
        let attr = |t: &str| t.split_once("name=\"").and_then(|(_, r)| r.split_once('"')).map(|(n, _)| xml_unescape(n));
        if tag.starts_with("key ") {
            let name = attr(tag).unwrap_or_default();
            if tag.ends_with('/') {
                rest = &rest[end + 1..];
                continue;
            }
            stack.push((name, HashMap::new()));
        } else if tag == "/key" {
            if let Some((name, values)) = stack.pop()
                && values.contains_key("Hostname")
            {
                // The keys above it, after "Sessions", are its folders.
                let path: Vec<&str> = stack.iter().map(|(n, _)| n.as_str()).collect();
                let folders = path.iter().position(|n| *n == "Sessions").map(|i| path[i + 1..].join(" / ")).unwrap_or_default();
                if let Some(h) = crt_host(format!("{folders}/{name}"), name, folders, &values) {
                    out.push(h);
                }
            }
        } else if tag.starts_with("string ") || tag.starts_with("dword ") {
            let close = if tag.starts_with("string") { "</string>" } else { "</dword>" };
            if let (Some(name), Some(stop)) = (attr(tag), rest.find(close)) {
                let value = xml_unescape(&rest[end + 1..stop]);
                if let Some((_, values)) = stack.last_mut() {
                    values.insert(name, value);
                }
                rest = &rest[stop + close.len()..];
                continue;
            }
        }
        rest = &rest[end + 1..];
    }
    out
}

/* ---------- CSV ---------- */

/// Rows of a CSV text: quoted fields, `,` or `;` (whichever the header uses more).
fn csv_rows(text: &str) -> Vec<Vec<String>> {
    let first = text.lines().next().unwrap_or("");
    let sep = if first.matches(';').count() > first.matches(',').count() { ';' } else if first.contains('\t') && !first.contains(',') { '\t' } else { ',' };
    let mut rows = vec![];
    let mut row = vec![];
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            c if c == sep && !quoted => row.push(std::mem::take(&mut field)),
            '\n' if !quoted => {
                row.push(std::mem::take(&mut field).trim_end_matches('\r').to_string());
                rows.push(std::mem::take(&mut row));
            }
            c => field.push(c),
        }
    }
    if !field.is_empty() || !row.is_empty() {
        row.push(field);
        rows.push(row);
    }
    rows.retain(|r| r.iter().any(|f| !f.trim().is_empty()));
    rows
}

/// A CSV list: the header names the columns, in the words of whichever app wrote it.
pub fn parse_csv(text: &str) -> Result<Vec<ForeignHost>> {
    let rows = csv_rows(text);
    let Some(header) = rows.first() else { return Ok(vec![]) };
    let norm: Vec<String> = header.iter().map(|h| h.to_lowercase().chars().filter(|c| c.is_alphanumeric()).collect()).collect();
    let col = |names: &[&str]| names.iter().find_map(|n| norm.iter().position(|h| h == n));
    let address = col(&["hostnameip", "hostname", "host", "address", "ip", "ipaddress", "computername", "server", "uri", "url"]).ok_or(Error::NoAddressColumn)?;
    let label = col(&["label", "name", "title", "displayname", "entryname", "alias"]);
    let port = col(&["port"]);
    let user = col(&["username", "user", "login"]);
    let password = col(&["password"]);
    let group = col(&["groups", "group", "folder", "path", "parent", "parentfolder"]);
    let protocol = col(&["protocol", "type", "connectiontype"]);
    let mut out = vec![];
    for (i, r) in rows.iter().enumerate().skip(1) {
        let at = |c: Option<usize>| c.and_then(|c| r.get(c)).map(|s| s.trim().to_string()).unwrap_or_default();
        let p = at(protocol).to_lowercase();
        if !(p.is_empty() || p.contains("ssh")) {
            continue;
        }
        // An address may be a URL (ssh://user@host:port).
        let raw = at(Some(address));
        let raw = raw.strip_prefix("ssh://").unwrap_or(&raw).trim_end_matches('/').to_string();
        let (u, hostport) = split_user(&raw, &at(user));
        let (addr, inline_port) = match hostport.rsplit_once(':') {
            Some((h, p)) if !h.contains(':') && p.parse::<u16>().is_ok() => (h.to_string(), p.parse().ok()),
            _ => (hostport.clone(), None),
        };
        if addr.is_empty() {
            continue;
        }
        let port = at(port).parse().ok().or(inline_port).unwrap_or(22);
        let name = Some(at(label)).filter(|l| !l.is_empty()).unwrap_or_else(|| addr.clone());
        let mut h = ForeignHost::new(format!("{i}"), name, addr, port, u, at(group).replace(['\\', '|'], " / "));
        h.password = Some(at(password)).filter(|p| !p.is_empty());
        h.has_password = h.password.is_some();
        out.push(h);
    }
    Ok(out)
}

/* ---------- an OpenSSH config file ---------- */

fn from_ssh_config(path: &Path) -> Vec<ForeignHost> {
    let config = crate::import::read_config(path, &home());
    config
        .iter()
        .map(|c| {
            let mut h = ForeignHost::new(c.alias.clone(), c.alias.clone(), c.address.clone(), c.port, c.user.clone(), String::new());
            h.key_file = c.key_file.clone();
            // The last hop is the one the host is reached through; earlier ones chain through the list.
            if let Some(hop) = c.jumps.last() {
                h.jump = Some(match config.iter().find(|x| &x.alias == hop) {
                    Some(x) => Hop { key: Some(x.alias.clone()), address: x.address.clone(), port: x.port, user: x.user.clone() },
                    None => {
                        let (user, address, port) = crate::import::parse_hop(hop, &c.user);
                        Hop { key: None, address, port, user }
                    }
                });
            }
            h
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_putty_sessions_from_a_reg_file() {
        let reg = "Windows Registry Editor Version 5.00\r\n\r\n[HKEY_CURRENT_USER\\Software\\SimonTatham\\PuTTY\\Sessions]\r\n\r\n[HKEY_CURRENT_USER\\Software\\SimonTatham\\PuTTY\\Sessions\\Default%20Settings]\r\n\"HostName\"=\"\"\r\n\r\n[HKEY_CURRENT_USER\\Software\\SimonTatham\\PuTTY\\Sessions\\web%20box]\r\n\"HostName\"=\"admin@10.0.0.5\"\r\n\"PortNumber\"=dword:00000856\r\n\"Protocol\"=\"ssh\"\r\n\"PublicKeyFile\"=\"C:\\\\keys\\\\web.ppk\"\r\n\"PortForwardings\"=\"L8080=localhost:80,4D1080\"\r\n\"ProxyMethod\"=dword:00000002\r\n\"ProxyHost\"=\"proxy.lan\"\r\n\"ProxyPort\"=dword:00000438\r\n\"ProxyUsername\"=\"\"\r\n\r\n[HKEY_CURRENT_USER\\Software\\SimonTatham\\PuTTY\\Sessions\\serial]\r\n\"Protocol\"=\"serial\"\r\n";
        let hosts = putty_hosts(parse_reg(reg));
        assert_eq!(hosts.len(), 1);
        let h = &hosts[0];
        assert_eq!((h.label.as_str(), h.address.as_str(), h.port, h.user.as_str()), ("web box", "10.0.0.5", 2134, "admin"));
        assert_eq!(h.key_file.as_deref(), Some("C:\\keys\\web.ppk"));
        assert_eq!(h.tunnels.len(), 2);
        assert_eq!(h.tunnels[0].bind_port, 8080);
        assert_eq!(h.tunnels[0].target.as_deref(), Some("localhost:80"));
        assert!(matches!(h.tunnels[1].kind, TunnelKind::Dynamic));
        assert!(matches!(&h.proxy, Some(ForeignProxy::Socks5 { address, port: 1080, .. }) if address == "proxy.lan"));
    }

    #[test]
    fn reads_mobaxterm_bookmarks_with_folders_and_gateways() {
        let ini = "[Misc]\r\nx=1\r\n[Bookmarks]\r\nSubRep=\r\nImgNum=42\r\nbox= #109#0%10.0.0.9%22%root%%-1%-1%%jump.lan%2222%ops%0%0%0%_ProfileDir_\\.ssh\\id_rsa%%-1%0%0%0%%1080%%0%0%1#MobaFont%10#0# #-1\r\n[Bookmarks_1]\r\nSubRep=Prod\\DB\r\nImgNum=41\r\ndb1=#109#0%db1.lan%22%pg%%-1%-1%%%%%0%0%0%%%-1%0%0%0%%1080%%0%0%1#MobaFont#0# #-1\r\nrdp=#91#4%win%3389%\r\n";
        let hosts = parse_mobaxterm(ini);
        assert_eq!(hosts.len(), 2);
        assert_eq!(hosts[0].label, "box");
        assert_eq!(hosts[0].user, "root");
        let jump = hosts[0].jump.as_ref().unwrap();
        assert_eq!((jump.address.as_str(), jump.port, jump.user.as_str()), ("jump.lan", 2222, "ops"));
        assert!(hosts[0].key_file.as_ref().unwrap().ends_with(".ssh\\id_rsa"));
        assert_eq!((hosts[1].group.as_str(), hosts[1].address.as_str()), ("Prod / DB", "db1.lan"));
    }

    #[test]
    fn reads_securecrt_sessions() {
        let ini = "S:\"Protocol Name\"=SSH2\r\nS:\"Hostname\"=crt.lan\r\nS:\"Username\"=bob\r\nD:\"[SSH2] Port\"=00000016\r\nS:\"Identity Filename V2\"=C:\\k\\id::rawkey\r\n";
        let h = crt_host("a".into(), "a".into(), String::new(), &parse_securecrt_ini(ini)).unwrap();
        assert_eq!((h.address.as_str(), h.port, h.user.as_str(), h.key_file.as_deref()), ("crt.lan", 22, "bob", Some("C:\\k\\id")));
        let xml = r#"<?xml version="1.0"?><VanDyke><key name="Sessions"><key name="Work"><key name="api"><string name="Hostname">api.lan</string><dword name="[SSH2] Port">2200</dword><string name="Username">deploy</string><string name="Protocol Name">SSH2</string></key></key><key name="Default"><string name="Protocol Name">SSH2</string></key></key></VanDyke>"#;
        let hosts = parse_securecrt_xml(xml);
        assert_eq!(hosts.len(), 1);
        assert_eq!((hosts[0].group.as_str(), hosts[0].label.as_str(), hosts[0].port), ("Work", "api", 2200));
    }

    #[test]
    fn reads_csv_in_the_words_of_several_apps() {
        let termius = "Groups,Label,Tags,Hostname/IP,Protocol,Port,Username,Password\nProd,web,,10.0.0.1,ssh,22,root,s3cret\nProd,\"rdp, box\",,10.0.0.2,rdp,3389,,\n,,,ssh://me@h.lan:2222,,,,\n";
        let hosts = parse_csv(termius).unwrap();
        assert_eq!(hosts.len(), 2);
        assert_eq!((hosts[0].label.as_str(), hosts[0].group.as_str(), hosts[0].password.as_deref()), ("web", "Prod", Some("s3cret")));
        assert_eq!((hosts[1].address.as_str(), hosts[1].port, hosts[1].user.as_str(), hosts[1].label.as_str()), ("h.lan", 2222, "me", "h.lan"));
        let royal = "Name;URI;Port;Username;Folder\nbox;box.lan;;admin;A\\B\n";
        let hosts = parse_csv(royal).unwrap();
        assert_eq!((hosts[0].address.as_str(), hosts[0].port, hosts[0].group.as_str()), ("box.lan", 22, "A / B"));
        assert!(parse_csv("Name,Notes\nx,y\n").is_err());
    }
}
