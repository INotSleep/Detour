//! The commands: each asks the core and prints what it said, as a table or with `--json`.

use std::io::{IsTerminal, Write};

use detour_ipc::{Client, StreamItem};
use serde_json::{Value, json};

use crate::conn::{Fail, Res, confirm, fail, secret};
use crate::i18n::{error_text, t};

pub const PERSONAL: &str = "personal";

pub struct Ctx {
    pub c: Client,
    /// `personal` or a team's id.
    pub vault: String,
    pub json: bool,
}

/* ---------- output ---------- */

pub fn table(headers: &[&str], rows: Vec<Vec<String>>) {
    let width = |i: usize| rows.iter().map(|r| r[i].chars().count()).chain([headers[i].chars().count()]).max().unwrap_or(0);
    let widths: Vec<usize> = (0..headers.len()).map(width).collect();
    let line = |cells: Vec<&str>| {
        let mut out = String::new();
        for (i, c) in cells.iter().enumerate() {
            if i + 1 == cells.len() {
                out.push_str(c);
            } else {
                out.push_str(&format!("{c:<w$}  ", w = widths[i]));
            }
        }
        println!("{}", out.trim_end());
    };
    line(headers.to_vec());
    for r in &rows {
        line(r.iter().map(String::as_str).collect());
    }
}

fn print_json(v: &Value) {
    println!("{}", serde_json::to_string_pretty(v).unwrap_or_default());
}

fn s(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn size(n: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut u = 0;
    while v >= 1024.0 && u + 1 < units.len() {
        v /= 1024.0;
        u += 1;
    }
    if u == 0 { format!("{n} B") } else { format!("{v:.1} {}", units[u]) }
}

fn when(ms: &Value) -> String {
    let Some(ms) = ms.as_i64() else { return String::new() };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
    let min = (now - ms) / 60_000;
    match min {
        m if m < 1 => "now".into(),
        m if m < 60 => format!("{m}m"),
        m if m < 60 * 24 => format!("{}h", m / 60),
        m => format!("{}d", m / 60 / 24),
    }
}

/* ---------- vaults and records ---------- */

/// The vault a `--vault` name means: the personal one, or a team by name or id.
pub async fn vault_id(c: &Client, name: Option<&str>) -> Res<String> {
    let Some(name) = name.filter(|n| !n.eq_ignore_ascii_case(PERSONAL)) else { return Ok(PERSONAL.into()) };
    let teams = c.call("teams_list", json!({})).await?;
    teams
        .as_array()
        .into_iter()
        .flatten()
        .find(|e| s(&e["id"]) == name || e["view"]["name"].as_str().is_some_and(|n| n.eq_ignore_ascii_case(name)))
        .map(|e| s(&e["id"]))
        .ok_or_else(|| Fail(format!("{}: {name}", t().unknown_vault)))
}

/// Hosts, keys, snippets and tunnels of the vault, as the app shows them.
async fn data(ctx: &Ctx) -> Res<Value> {
    if ctx.vault == PERSONAL {
        return Ok(ctx.c.call("vault_get", json!({})).await?);
    }
    let teams = ctx.c.call("teams_list", json!({})).await?;
    let team = teams.as_array().into_iter().flatten().find(|e| s(&e["id"]) == ctx.vault).ok_or_else(|| Fail(t().unknown_vault.into()))?;
    Ok(team["view"]["data"].clone())
}

fn update_of(d: &Value) -> Value {
    json!({ "hosts": d["hosts"], "keys": d["keys"], "snippets": d["snippets"], "tunnels": d["tunnels"] })
}

/// Saves an edit made on `base`: only what changed goes in, edits elsewhere stay.
async fn put(ctx: &Ctx, base: &Value, next: &Value) -> Res<()> {
    let (base, next) = (update_of(base), update_of(next));
    if ctx.vault == PERSONAL {
        ctx.c.call("vault_put", json!({ "update": next, "base": base })).await?;
    } else {
        ctx.c.call("team_put", json!({ "id": ctx.vault, "update": next, "base": base })).await?;
    }
    Ok(())
}

/// A record by id or name (hosts also by address); several matches are an error that lists them.
fn find<'a>(list: &'a Value, name: &str, field: &str, unknown: &str) -> Res<&'a Value> {
    let all: Vec<&Value> = list.as_array().into_iter().flatten().collect();
    if let Some(v) = all.iter().find(|v| s(&v["id"]) == name) {
        return Ok(v);
    }
    let hits: Vec<&&Value> = all.iter().filter(|v| v[field].as_str().is_some_and(|l| l.eq_ignore_ascii_case(name)) || (field == "label" && v["address"].as_str() == Some(name))).collect();
    match hits.as_slice() {
        [one] => Ok(one),
        [] => fail(format!("{unknown}: {name}")),
        many => fail(format!("{}\n{}", t().which_host, many.iter().map(|v| format!("  {}  ({})", s(&v[field]), s(&v["id"]))).collect::<Vec<_>>().join("\n"))),
    }
}

pub async fn host(ctx: &Ctx, name: &str) -> Res<Value> {
    let d = data(ctx).await?;
    Ok(find(&d["hosts"], name, "label", t().unknown_host)?.clone())
}

/* ---------- logins that ask ---------- */

/// Repeats a call that logs in to a host, answering what it asks on the terminal:
/// a new or changed server key, a password or passphrase.
pub async fn with_login<F, Fut>(ctx: &Ctx, host: &Value, mut call: F) -> Res<Value>
where
    F: FnMut(Option<String>, Option<String>) -> Fut,
    Fut: std::future::Future<Output = Result<Value, Value>>,
{
    let (mut secret_now, mut trust): (Option<String>, Option<String>) = (None, None);
    let mut attempts = 3;
    loop {
        match call(secret_now.clone(), trust.clone()).await {
            Ok(v) => return Ok(v),
            Err(e) => match e["kind"].as_str().unwrap_or("") {
                "hostKeyUnknown" => {
                    let fp = s(&e["fingerprint"]);
                    eprintln!("{} {fp}", t().new_host_key);
                    if !confirm(t().trust_it) {
                        return fail(t().cancelled);
                    }
                    trust = Some(trust.map_or(fp.clone(), |t| format!("{t},{fp}")));
                }
                "hostKeyChanged" => {
                    eprintln!("{} {}, {}", t().changed_host_key, s(&e["expected"]), s(&e["actual"]));
                    if !confirm(t().replace_it) {
                        return fail(t().cancelled);
                    }
                    let via = e["via"].as_str().map(str::to_string).unwrap_or_else(|| s(&host["id"]));
                    ctx.c.call("host_forget_key", json!({ "vaultId": ctx.vault, "hostId": via })).await?;
                    let fp = s(&e["actual"]);
                    trust = Some(trust.map_or(fp.clone(), |t| format!("{t},{fp}")));
                }
                "needSecret" => secret_now = Some(secret(&s(&e["prompt"]))?),
                "authFailed" if !e["secret"].is_null() && attempts > 1 => {
                    attempts -= 1;
                    eprintln!("{}", t().permission_denied);
                    secret_now = None;
                }
                _ => return Err(e.into()),
            },
        }
    }
}

/* ---------- the core ---------- */

pub async fn status(c: &Client, json_out: bool) -> Res<()> {
    let st = c.call("vault_status", json!({})).await?;
    let info = c.call("hello", json!({ "client": "cli" })).await?;
    if json_out {
        print_json(&json!({ "core": info, "vault": st }));
        return Ok(());
    }
    let state = if st["initialized"] == false { t().status_new } else if st["unlocked"] == true { t().status_unlocked } else { t().status_locked };
    println!("{} {} (pid {}): {state}", t().status_running, s(&info["coreVersion"]), s(&info["pid"]));
    Ok(())
}

/* ---------- hosts ---------- */

pub async fn ls(ctx: &Ctx, query: Option<&str>) -> Res<()> {
    let d = data(ctx).await?;
    let q = query.map(str::to_lowercase);
    let hosts: Vec<&Value> = d["hosts"].as_array().into_iter().flatten().filter(|h| q.as_ref().is_none_or(|q| [&h["label"], &h["address"], &h["user"], &h["group"]].iter().any(|v| s(v).to_lowercase().contains(q.as_str())))).collect();
    if ctx.json {
        print_json(&json!(hosts));
        return Ok(());
    }
    if hosts.is_empty() {
        eprintln!("{}", t().no_hosts);
        return Ok(());
    }
    let rows = hosts
        .iter()
        .map(|h| {
            let port = if h["port"] == 22 { String::new() } else { format!(":{}", s(&h["port"])) };
            vec![s(&h["label"]), format!("{}@{}{port}", s(&h["user"]), s(&h["address"])), s(&h["group"]), when(&h["lastConnected"])]
        })
        .collect();
    table(&t().headers_hosts, rows);
    Ok(())
}

/// How a host logs in and is reached, as given on the command line.
#[derive(clap::Args, Default)]
pub struct HostOpts {
    #[arg(long, short)]
    pub address: Option<String>,
    #[arg(long, short)]
    pub port: Option<u16>,
    #[arg(long, short)]
    pub user: Option<String>,
    #[arg(long, short, conflicts_with_all = ["password", "agent"])]
    pub key: Option<String>,
    #[arg(long, conflicts_with = "agent")]
    pub password: bool,
    #[arg(long)]
    pub agent: bool,
    #[arg(long, short)]
    pub group: Option<String>,
    #[arg(long, value_name = "HOST")]
    pub via: Option<String>,
    #[arg(long)]
    pub forward_agent: Option<bool>,
    #[arg(long)]
    pub tags: Option<String>,
}

fn apply(d: &Value, h: &mut Value, o: HostOpts) -> Res<()> {
    if let Some(a) = o.address {
        h["address"] = json!(a);
    }
    if let Some(p) = o.port {
        h["port"] = json!(p);
    }
    if let Some(u) = o.user {
        h["user"] = json!(u);
    }
    if let Some(k) = o.key {
        let key = find(&d["keys"], &k, "name", t().unknown_key)?;
        h["auth"] = json!({ "type": "key", "keyId": key["id"] });
    }
    if o.password {
        h["auth"] = json!({ "type": "password" });
        let pw = secret(t().host_password)?;
        if !pw.is_empty() {
            h["newPassword"] = json!(pw);
        }
    }
    if o.agent {
        h["auth"] = json!({ "type": "agent" });
    }
    if let Some(g) = o.group {
        h["group"] = json!(g);
    }
    if let Some(v) = o.via {
        let via = find(&d["hosts"], &v, "label", t().unknown_host)?;
        h["proxy"] = json!({ "type": "jump", "hostId": via["id"] });
    }
    if let Some(f) = o.forward_agent {
        h["forwardAgent"] = json!(f);
    }
    if let Some(tags) = o.tags {
        h["tags"] = json!(tags.split(',').map(str::trim).filter(|t| !t.is_empty()).collect::<Vec<_>>());
    }
    Ok(())
}

pub async fn host_add(ctx: &Ctx, name: &str, o: HostOpts) -> Res<()> {
    let base = data(ctx).await?;
    let Some(address) = o.address.clone() else { return fail("--address") };
    let mut h = json!({
        "id": detour_core::new_id(), "group": "", "label": name, "address": address, "port": 22, "user": "root",
        "auth": { "type": "agent" }, "tags": [], "hasPassword": false, "forwardAgent": false,
    });
    // The first key of the vault, as the app suggests, unless told otherwise.
    if let Some(k) = base["keys"].as_array().and_then(|k| k.first())
        && o.key.is_none()
        && !o.password
        && !o.agent
    {
        h["auth"] = json!({ "type": "key", "keyId": k["id"] });
    }
    apply(&base, &mut h, o)?;
    let mut next = base.clone();
    next["hosts"].as_array_mut().unwrap().push(h.clone());
    put(ctx, &base, &next).await?;
    eprintln!("{}: {name}", t().host_added);
    Ok(())
}

pub async fn host_edit(ctx: &Ctx, name: &str, label: Option<String>, o: HostOpts) -> Res<()> {
    let base = data(ctx).await?;
    let id = s(&find(&base["hosts"], name, "label", t().unknown_host)?["id"]);
    let mut next = base.clone();
    let h = next["hosts"].as_array_mut().unwrap().iter_mut().find(|h| s(&h["id"]) == id).unwrap();
    if let Some(l) = label {
        h["label"] = json!(l);
    }
    let mut edited = h.clone();
    apply(&base, &mut edited, o)?;
    *h = edited;
    put(ctx, &base, &next).await?;
    eprintln!("{}: {name}", t().host_saved);
    Ok(())
}

pub async fn host_rm(ctx: &Ctx, name: &str) -> Res<()> {
    let base = data(ctx).await?;
    let id = s(&find(&base["hosts"], name, "label", t().unknown_host)?["id"]);
    let mut next = base.clone();
    next["hosts"].as_array_mut().unwrap().retain(|h| s(&h["id"]) != id);
    put(ctx, &base, &next).await?;
    eprintln!("{}: {name}", t().host_removed);
    Ok(())
}

pub async fn host_import(ctx: &Ctx, aliases: Vec<String>, known_hosts: bool) -> Res<()> {
    let preview = ctx.c.call("ssh_import_preview", json!({})).await?;
    let fresh: Vec<String> = preview["hosts"].as_array().into_iter().flatten().filter(|h| h["exists"] == false).map(|h| s(&h["alias"])).collect();
    let pick: Vec<String> = if aliases.is_empty() { fresh } else { aliases };
    if pick.is_empty() && !known_hosts {
        eprintln!("{}", t().nothing_to_import);
        return Ok(());
    }
    let r = ctx.c.call("ssh_import", json!({ "aliases": pick, "knownHosts": known_hosts })).await?;
    if ctx.json {
        print_json(&r);
        return Ok(());
    }
    eprintln!("{}: {} hosts, {} keys, {} known hosts", t().imported, s(&r["hosts"]), s(&r["keys"]), s(&r["known"]));
    Ok(())
}

pub async fn host_export(ctx: &Ctx, names: Vec<String>, out: Option<std::path::PathBuf>, keys: Option<std::path::PathBuf>) -> Res<()> {
    let d = data(ctx).await?;
    let ids: Option<Vec<String>> = if names.is_empty() { None } else { Some(names.iter().map(|n| find(&d["hosts"], n, "label", t().unknown_host).map(|h| s(&h["id"]))).collect::<Res<_>>()?) };
    let key_dir = keys.map(|k| std::path::absolute(&k).unwrap_or(k).display().to_string());
    if let Some(dir) = &key_dir {
        std::fs::create_dir_all(dir).map_err(|e| Fail(e.to_string()))?;
    }
    let r = ctx.c.call("hosts_export", json!({ "vaultId": ctx.vault, "hostIds": ids, "keyDir": key_dir })).await?;
    let config = s(&r["config"]);
    match out {
        Some(path) => {
            std::fs::write(&path, config).map_err(|e| Fail(e.to_string()))?;
            eprintln!("{} {}", t().saved_to, path.display());
        }
        None => print!("{config}"),
    }
    Ok(())
}

/* ---------- running commands ---------- */

pub async fn exec(ctx: &Ctx, name: &str, command: &str) -> Res<i32> {
    let h = host(ctx, name).await?;
    let (out_id, mut out) = ctx.c.stream();
    let (err_id, mut err) = ctx.c.stream();
    let pump = tokio::spawn(async move {
        let (mut so, mut se) = (std::io::stdout(), std::io::stderr());
        loop {
            tokio::select! {
                Some(StreamItem::Data(b)) = out.recv() => { let _ = so.write_all(&b); let _ = so.flush(); }
                Some(StreamItem::Data(b)) = err.recv() => { let _ = se.write_all(&b); let _ = se.flush(); }
                else => break,
            }
        }
    });
    let (c, vault, host_id) = (ctx.c.clone(), ctx.vault.clone(), s(&h["id"]));
    let r = with_login(ctx, &h, |secret, trust| {
        let (c, vault, host_id, command) = (c.clone(), vault.clone(), host_id.clone(), command.to_string());
        async move { c.call("host_exec", json!({ "vaultId": vault, "hostId": host_id, "command": command, "secret": secret, "trustFingerprint": trust, "stream": out_id, "errStream": err_id })).await }
    })
    .await;
    // What is still on its way is printed before exiting.
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    ctx.c.drop_stream(out_id);
    ctx.c.drop_stream(err_id);
    let _ = pump.await;
    Ok(r?["exitStatus"].as_i64().map(|c| c as i32).unwrap_or(255))
}

/* ---------- files ---------- */

/// `host:path` on a server, or a path on this computer.
pub fn split_remote(arg: &str) -> Option<(&str, &str)> {
    // `C:\x` is a Windows path, not host `C`.
    let (host, path) = arg.split_once(':')?;
    if host.len() == 1 && cfg!(windows) || host.is_empty() || host.contains(['/', '\\']) {
        return None;
    }
    Some((host, path))
}

async fn files_open(ctx: &Ctx, name: &str) -> Res<(String, String)> {
    let h = host(ctx, name).await?;
    let (c, vault, host_id) = (ctx.c.clone(), ctx.vault.clone(), s(&h["id"]));
    let r = with_login(ctx, &h, |secret, trust| {
        let (c, vault, host_id) = (c.clone(), vault.clone(), host_id.clone());
        async move { c.call("files_open", json!({ "vaultId": vault, "hostId": host_id, "secret": secret, "trustFingerprint": trust })).await }
    })
    .await?;
    Ok((s(&r["id"]), s(&r["home"])))
}

/// A path on the server: relative ones start at the login's home folder.
fn remote_path(home: &str, path: &str) -> String {
    if path.starts_with('/') {
        path.to_string()
    } else if path.is_empty() || path == "~" {
        home.to_string()
    } else {
        format!("{}/{}", home.trim_end_matches('/'), path.trim_start_matches("~/"))
    }
}

/// Prints a transfer's progress on one line while it goes.
fn progress_printer(c: &Client) -> (u64, tokio::task::JoinHandle<()>) {
    let (id, mut rx) = c.stream();
    let show = std::io::stderr().is_terminal();
    let task = tokio::spawn(async move {
        while let Some(item) = rx.recv().await {
            if let (true, StreamItem::Json(p)) = (show, item) {
                let (done, total) = (p["done"].as_u64().unwrap_or(0), p["total"].as_u64().unwrap_or(0));
                let pct = (done * 100).checked_div(total).unwrap_or(0);
                eprint!("\r{pct:>3}%  {} / {}   ", size(done), size(total));
            }
        }
        if show {
            eprintln!();
        }
    });
    (id, task)
}

pub async fn cp(ctx: &Ctx, from: &str, to: &str) -> Res<()> {
    let transfer = detour_core::new_id();
    match (split_remote(from), split_remote(to)) {
        (None, Some((name, path))) => {
            let (id, home) = files_open(ctx, name).await?;
            let local = std::path::absolute(from).map_err(|e| Fail(e.to_string()))?;
            let mut remote = remote_path(&home, path);
            if path.is_empty() || path.ends_with('/') {
                remote = format!("{}/{}", remote.trim_end_matches('/'), local.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default());
            }
            let (stream, task) = progress_printer(&ctx.c);
            let r = ctx.c.call("files_upload", json!({ "id": id, "local": local, "remote": remote, "replace": true, "transfer": transfer, "stream": stream })).await;
            ctx.c.drop_stream(stream);
            let _ = task.await;
            let _ = ctx.c.call("files_close", json!({ "id": id })).await;
            r?;
        }
        (Some((name, path)), None) => {
            let (id, home) = files_open(ctx, name).await?;
            let remote = remote_path(&home, path);
            let mut local = std::path::absolute(to).map_err(|e| Fail(e.to_string()))?;
            let (stream, task) = progress_printer(&ctx.c);
            let r = if local.is_dir() {
                ctx.c.call("files_download_into", json!({ "id": id, "remote": remote, "localDir": local, "transfer": transfer, "stream": stream })).await.inspect(|v| local = std::path::PathBuf::from(s(v)))
            } else {
                ctx.c.call("files_download", json!({ "id": id, "remote": remote, "local": local, "transfer": transfer, "stream": stream })).await
            };
            ctx.c.drop_stream(stream);
            let _ = task.await;
            let _ = ctx.c.call("files_close", json!({ "id": id })).await;
            r?;
        }
        _ => return fail("detour cp <file> <host>:<path>  |  detour cp <host>:<path> <file>"),
    }
    Ok(())
}

pub enum FileOp {
    Ls,
    Mkdir,
    Rm,
    Mv(String),
    Chmod(u32),
}

pub async fn files(ctx: &Ctx, target: &str, op: FileOp) -> Res<()> {
    let Some((name, path)) = split_remote(target) else { return fail("<host>:<path>") };
    let (id, home) = files_open(ctx, name).await?;
    let path = remote_path(&home, path);
    let r: Result<Value, Value> = match op {
        FileOp::Ls => {
            let list = ctx.c.call("files_list", json!({ "id": id, "path": path })).await?;
            if ctx.json {
                print_json(&list);
            } else {
                let rows = list
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|e| {
                        let dir = e["kind"] == "dir";
                        let name = if dir { format!("{}/", s(&e["name"])) } else { s(&e["name"]) };
                        vec![name, if dir { t().dir.to_string() } else { size(e["size"].as_u64().unwrap_or(0)) }, when(&e["modified"])]
                    })
                    .collect();
                table(&t().headers_files, rows);
            }
            Ok(Value::Null)
        }
        FileOp::Mkdir => ctx.c.call("files_mkdir", json!({ "id": id, "path": path })).await,
        FileOp::Rm => {
            let (stream, task) = progress_printer(&ctx.c);
            let r = ctx.c.call("files_remove", json!({ "id": id, "path": path, "transfer": detour_core::new_id(), "stream": stream })).await;
            ctx.c.drop_stream(stream);
            let _ = task.await;
            r
        }
        FileOp::Mv(to) => ctx.c.call("files_rename", json!({ "id": id, "from": path, "to": remote_path(&home, &to) })).await,
        FileOp::Chmod(mode) => ctx.c.call("files_chmod", json!({ "id": id, "path": path, "mode": mode })).await,
    };
    let _ = ctx.c.call("files_close", json!({ "id": id })).await;
    r?;
    Ok(())
}

/* ---------- tunnels ---------- */

pub async fn tunnels(ctx: &Ctx) -> Res<()> {
    let d = data(ctx).await?;
    let status = ctx.c.call("tunnels_status", json!({})).await?;
    let state = |id: &str| status.as_array().into_iter().flatten().find(|x| s(&x["key"]) == format!("{}/{id}", ctx.vault)).map(|x| s(&x["state"]));
    if ctx.json {
        let list: Vec<Value> = d["tunnels"].as_array().into_iter().flatten().map(|tu| json!({ "tunnel": tu, "state": state(&s(&tu["id"])) })).collect();
        print_json(&json!(list));
        return Ok(());
    }
    let rows = d["tunnels"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|tu| vec![s(&tu["name"]), s(&tu["kind"]), s(&tu["bindPort"]), state(&s(&tu["id"])).map(|s| if s == "on" { t().on.to_string() } else { s }).unwrap_or_else(|| t().off.to_string())])
        .collect();
    table(&t().headers_tunnels, rows);
    Ok(())
}

pub async fn tunnel_up(ctx: &Ctx, name: &str) -> Res<()> {
    let d = data(ctx).await?;
    let tu = find(&d["tunnels"], name, "name", t().unknown_tunnel)?.clone();
    let via = find(&d["hosts"], &s(&tu["viaHostId"]), "label", t().unknown_host)?.clone();
    let (c, vault, id) = (ctx.c.clone(), ctx.vault.clone(), s(&tu["id"]));
    with_login(ctx, &via, |secret, trust| {
        let (c, vault, id) = (c.clone(), vault.clone(), id.clone());
        async move { c.call("tunnel_start", json!({ "vaultId": vault, "tunnelId": id, "secret": secret, "trustFingerprint": trust })).await }
    })
    .await?;
    eprintln!("{}: {name} ({})", t().tunnel_up, s(&tu["bindPort"]));
    Ok(())
}

pub async fn tunnel_down(ctx: &Ctx, name: &str) -> Res<()> {
    let d = data(ctx).await?;
    let tu = find(&d["tunnels"], name, "name", t().unknown_tunnel)?;
    ctx.c.call("tunnel_stop", json!({ "vaultId": ctx.vault, "tunnelId": tu["id"] })).await?;
    eprintln!("{}: {name}", t().tunnel_down);
    Ok(())
}

/* ---------- keys ---------- */

pub async fn keys(ctx: &Ctx) -> Res<()> {
    let d = data(ctx).await?;
    if ctx.json {
        print_json(&d["keys"]);
        return Ok(());
    }
    let rows = d["keys"].as_array().into_iter().flatten().map(|k| vec![s(&k["name"]), format!("{}{}", s(&k["algorithm"]), k["bits"].as_u64().map(|b| format!(" {b}")).unwrap_or_default()), s(&k["fingerprint"])]).collect();
    table(&t().headers_keys, rows);
    Ok(())
}

pub async fn key_gen(ctx: &Ctx, name: &str, algorithm: &str, with_passphrase: bool) -> Res<()> {
    let passphrase = if with_passphrase { Some(secret(t().passphrase)?) } else { None };
    let k = ctx.c.call("key_generate", json!({ "vaultId": ctx.vault, "name": name, "algorithm": algorithm, "passphrase": passphrase, "remember": true })).await?;
    if ctx.json {
        print_json(&k);
        return Ok(());
    }
    println!("{}", s(&k["publicKey"]));
    eprintln!("{}: {name}", t().key_saved);
    Ok(())
}

pub async fn key_import(ctx: &Ctx, file: &std::path::Path, name: Option<String>) -> Res<()> {
    let text = std::fs::read_to_string(file).map_err(|e| Fail(format!("{}: {e}", file.display())))?;
    let name = name.unwrap_or_else(|| file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| "key".into()));
    let mut passphrase: Option<String> = None;
    loop {
        match ctx.c.call("key_import", json!({ "vaultId": ctx.vault, "name": name, "text": text, "passphrase": passphrase, "remember": true })).await {
            Ok(_) => break,
            Err(e) if e["kind"] == "passphraseRequired" && passphrase.is_none() => passphrase = Some(secret(t().passphrase)?),
            Err(e) => return Err(e.into()),
        }
    }
    eprintln!("{}: {name}", t().key_saved);
    Ok(())
}

pub async fn key_export(ctx: &Ctx, name: &str, out: Option<std::path::PathBuf>) -> Res<()> {
    let d = data(ctx).await?;
    let k = find(&d["keys"], name, "name", t().unknown_key)?;
    let text = s(&ctx.c.call("key_export", json!({ "vaultId": ctx.vault, "keyId": k["id"], "passphrase": null })).await?);
    match out {
        Some(dir_or_file) => {
            let (dir, file) = if dir_or_file.is_dir() { (dir_or_file.clone(), detour_core::export::key_file_name(&s(&k["name"]))) } else { (dir_or_file.parent().map(|p| p.to_path_buf()).unwrap_or_default(), dir_or_file.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()) };
            let path = detour_core::keys::save_file(&dir, &file, &text).map_err(|e| Fail(e.to_string()))?;
            eprintln!("{} {}", t().saved_to, path.display());
        }
        None => print!("{text}"),
    }
    Ok(())
}

pub async fn key_rm(ctx: &Ctx, name: &str) -> Res<()> {
    let base = data(ctx).await?;
    let id = s(&find(&base["keys"], name, "name", t().unknown_key)?["id"]);
    let mut next = base.clone();
    next["keys"].as_array_mut().unwrap().retain(|k| s(&k["id"]) != id);
    put(ctx, &base, &next).await?;
    eprintln!("{}: {name}", t().key_removed);
    Ok(())
}

/* ---------- snippets ---------- */

pub async fn snippets(ctx: &Ctx) -> Res<()> {
    let d = data(ctx).await?;
    if ctx.json {
        print_json(&d["snippets"]);
        return Ok(());
    }
    table(&t().headers_snippets, d["snippets"].as_array().into_iter().flatten().map(|x| vec![s(&x["name"]), s(&x["command"])]).collect());
    Ok(())
}

pub async fn snippet_add(ctx: &Ctx, name: &str, command: &str) -> Res<()> {
    let base = data(ctx).await?;
    let mut next = base.clone();
    next["snippets"].as_array_mut().unwrap().push(json!({ "id": detour_core::new_id(), "name": name, "command": command }));
    put(ctx, &base, &next).await?;
    eprintln!("{}: {name}", t().snippet_saved);
    Ok(())
}

pub async fn snippet_rm(ctx: &Ctx, name: &str) -> Res<()> {
    let base = data(ctx).await?;
    let id = s(&find(&base["snippets"], name, "name", t().unknown_snippet)?["id"]);
    let mut next = base.clone();
    next["snippets"].as_array_mut().unwrap().retain(|x| s(&x["id"]) != id);
    put(ctx, &base, &next).await?;
    eprintln!("{}: {name}", t().snippet_removed);
    Ok(())
}

pub async fn snippet_run(ctx: &Ctx, name: &str, host: &str) -> Res<i32> {
    let d = data(ctx).await?;
    let command = s(&find(&d["snippets"], name, "name", t().unknown_snippet)?["command"]);
    exec(ctx, host, &command).await
}

/* ---------- sync ---------- */

pub async fn sync(ctx: &Ctx, now: bool) -> Res<()> {
    let st = if now { ctx.c.call("sync_now", json!({})).await?["status"].clone() } else { ctx.c.call("sync_status", json!({})).await? };
    if ctx.json {
        print_json(&st);
        return Ok(());
    }
    if st["enabled"] == false {
        println!("{}", t().sync_off);
        return Ok(());
    }
    match st["error"].is_null() {
        true => println!("{}: {} ({})", t().synced, s(&st["location"]), when(&st["syncedAt"])),
        false => println!("{}: {}", s(&st["location"]), error_text(&st["error"])),
    }
    Ok(())
}

/* ---------- teams ---------- */

pub async fn teams(ctx: &Ctx) -> Res<()> {
    let list = ctx.c.call("teams_list", json!({})).await?;
    if ctx.json {
        print_json(&list);
        return Ok(());
    }
    let rows = list.as_array().into_iter().flatten().map(|e| {
        let v = &e["view"];
        if v.is_null() { vec![s(&e["id"]), String::new(), s(&e["error"])] } else { vec![s(&v["name"]), s(&v["myRole"]), v["data"]["hosts"].as_array().map_or(0, |h| h.len()).to_string()] }
    });
    table(&t().headers_teams, rows.collect());
    Ok(())
}

pub async fn team_sync(ctx: &Ctx) -> Res<()> {
    ctx.c.call("team_sync", json!({ "id": ctx.vault })).await?;
    eprintln!("{}", t().synced);
    Ok(())
}

async fn team_view(ctx: &Ctx) -> Res<Value> {
    Ok(ctx.c.call("team_sync", json!({ "id": ctx.vault })).await?)
}

pub async fn members(ctx: &Ctx) -> Res<()> {
    let v = team_view(ctx).await?;
    if ctx.json {
        print_json(&v["members"]);
        return Ok(());
    }
    let rows = v["members"].as_array().into_iter().flatten().map(|m| vec![format!("{}{}", s(&m["name"]), if m["isYou"] == true { format!(" {}", t().you) } else { String::new() }), s(&m["role"]), s(&m["id"])]).collect();
    table(&t().headers_members, rows);
    Ok(())
}

async fn member_id(ctx: &Ctx, name: &str) -> Res<String> {
    let v = team_view(ctx).await?;
    Ok(s(&find(&v["members"], name, "name", t().unknown_member)?["id"]))
}

pub async fn member_add(ctx: &Ctx, detour_id: &str, name: &str, role: &str, access: &str) -> Res<()> {
    ctx.c.call("team_member_add", json!({ "id": ctx.vault, "detourId": detour_id, "name": name, "role": role, "access": access })).await?;
    eprintln!("{}", t().done);
    Ok(())
}

pub async fn member_rm(ctx: &Ctx, name: &str) -> Res<()> {
    let member = member_id(ctx, name).await?;
    ctx.c.call("team_member_remove", json!({ "id": ctx.vault, "member": member })).await?;
    eprintln!("{}", t().done);
    Ok(())
}

pub async fn member_role(ctx: &Ctx, name: &str, role: &str) -> Res<()> {
    let member = member_id(ctx, name).await?;
    ctx.c.call("team_member_update", json!({ "id": ctx.vault, "member": member, "role": role })).await?;
    eprintln!("{}", t().done);
    Ok(())
}

pub async fn access(ctx: &Ctx, record: &str, name: &str, level: &str) -> Res<()> {
    let d = data(ctx).await?;
    // A host, key, snippet or tunnel by its name.
    let id = ["hosts", "keys", "snippets", "tunnels"]
        .iter()
        .find_map(|coll| find(&d[*coll], record, if *coll == "hosts" { "label" } else { "name" }, "").ok().map(|r| s(&r["id"])))
        .ok_or_else(|| Fail(format!("{}: {record}", t().unknown_host)))?;
    let member = member_id(ctx, name).await?;
    ctx.c.call("team_access_set", json!({ "id": ctx.vault, "changes": [{ "record": id, "member": member, "level": level }] })).await?;
    eprintln!("{}", t().done);
    Ok(())
}

pub async fn team_leave(ctx: &Ctx) -> Res<()> {
    ctx.c.call("team_leave", json!({ "id": ctx.vault })).await?;
    eprintln!("{}", t().done);
    Ok(())
}

pub async fn my_id(c: &Client, json_out: bool) -> Res<()> {
    let id = c.call("my_id", json!({})).await?;
    if json_out {
        print_json(&id);
        return Ok(());
    }
    println!("{}", s(&id["detourId"]));
    eprintln!("{}", s(&id["fingerprint"]));
    Ok(())
}
