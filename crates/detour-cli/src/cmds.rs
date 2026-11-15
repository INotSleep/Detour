//! The commands: each asks the core and prints what it said, as a table or with `--json`.

use std::io::{IsTerminal, Read, Write};

use detour_ipc::{Client, StreamItem};
use serde_json::{Value, json};

use crate::conn::{Fail, Res, confirm, fail, secret};
use crate::i18n::t;

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
    let mut u = json!({ "hosts": d["hosts"], "keys": d["keys"], "snippets": d["snippets"], "tunnels": d["tunnels"] });
    // A core that doesn't know proxies or groups sends none; then they aren't touched.
    for k in ["proxies", "groups"] {
        if d.get(k).is_some_and(Value::is_array) {
            u[k] = d[k].clone();
        }
    }
    u
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

/* ---------- groups and their order ---------- */

fn group_of(h: &Value) -> String {
    let g = s(&h["group"]);
    if g == "Без группы" || g == "No group" { String::new() } else { g }
}

/// Hosts by their order in a group; those without one after, as they come.
fn sorted_hosts(mut hosts: Vec<&Value>) -> Vec<&Value> {
    hosts.sort_by(|a, b| a["order"].as_f64().unwrap_or(f64::INFINITY).total_cmp(&b["order"].as_f64().unwrap_or(f64::INFINITY)));
    hosts
}

/// The vault's groups in the app's order: those with a record by their order, then groups
/// only hosts name, by name, then hosts without a group (named "").
fn ordered_groups(d: &Value, with_empty: bool) -> Vec<(String, Vec<&Value>)> {
    let hosts: Vec<&Value> = d["hosts"].as_array().into_iter().flatten().collect();
    let mut records: Vec<&Value> = d["groups"].as_array().into_iter().flatten().collect();
    records.sort_by(|a, b| a["order"].as_f64().unwrap_or(0.0).total_cmp(&b["order"].as_f64().unwrap_or(0.0)));
    let of = |name: &str| sorted_hosts(hosts.iter().copied().filter(|h| group_of(h) == name).collect());
    let mut out: Vec<(String, Vec<&Value>)> = records.iter().map(|g| (s(&g["name"]), of(&s(&g["name"])))).filter(|(_, h)| with_empty || !h.is_empty()).collect();
    let mut rest: Vec<String> = hosts.iter().map(|h| group_of(h)).filter(|g| !g.is_empty() && !records.iter().any(|r| s(&r["name"]) == *g)).collect();
    rest.sort();
    rest.dedup();
    for g in rest {
        let h = of(&g);
        out.push((g, h));
    }
    if hosts.iter().any(|h| group_of(h).is_empty()) {
        out.push((String::new(), of("")));
    }
    out
}

fn between(a: Option<f64>, b: Option<f64>) -> f64 {
    match (a, b) {
        (None, None) => 1.0,
        (None, Some(b)) => b - 1.0,
        (Some(a), None) => a + 1.0,
        (Some(a), Some(b)) => (a + b) / 2.0,
    }
}

/// A group named by the person: its name as the vault has it.
fn find_group(d: &Value, name: &str) -> Res<String> {
    ordered_groups(d, true).into_iter().map(|(g, _)| g).find(|g| !g.is_empty() && g.eq_ignore_ascii_case(name)).ok_or_else(|| Fail(format!("{}: {name}", t().unknown_group)))
}

/// Gives every named group a record, so each has a place to keep.
fn with_records(next: &mut Value) {
    let names: Vec<String> = ordered_groups(next, true).into_iter().map(|(g, _)| g).filter(|g| !g.is_empty()).collect();
    if !next["groups"].is_array() {
        next["groups"] = json!([]);
    }
    let last = next["groups"].as_array().unwrap().iter().filter_map(|g| g["order"].as_f64()).fold(0.0, f64::max);
    let mut n = 0.0;
    for name in names {
        let list = next["groups"].as_array_mut().unwrap();
        if !list.iter().any(|g| s(&g["name"]) == name) {
            n += 1.0;
            list.push(json!({ "id": detour_core::new_id(), "name": name, "order": last + n }));
        }
    }
}

/// Groups in order with their hosts, and the proxy and login each one gives its hosts:
/// this person's own settings of it, else (the proxy) the team's.
pub async fn groups(ctx: &Ctx, mine: bool) -> Res<()> {
    let (d, layout) = if mine { mine_of(ctx).await? } else { (data(ctx).await?, Value::Null) };
    let list: Vec<(String, Option<String>, Vec<&Value>)> = if mine { ordered_mine(&d, &layout) } else { ordered_groups(&d, true).into_iter().map(|(g, h)| (g, None, h)).collect() };
    let prefs = ctx.c.call("group_prefs", json!({})).await.unwrap_or(json!([]));
    let proxies = all_proxies(ctx, &d).await?;
    let proxy_name = |vault: &str, id: &str| proxies.iter().find(|(v, p)| v == vault && s(&p["id"]) == id).map(|(_, p)| s(&p["name"])).unwrap_or_else(|| id.to_string());
    let keys = ctx.c.call("vault_get", json!({})).await.map(|v| v["keys"].clone()).unwrap_or(json!([]));
    let rows: Vec<(String, usize, Vec<String>, String, String)> = list
        .iter()
        .map(|(g, id, hosts)| {
            let key = match id {
                Some(id) => format!("{}/mine:{id}", ctx.vault),
                None => format!("{}/team:{g}", ctx.vault),
            };
            let p = prefs.as_array().into_iter().flatten().find(|p| s(&p["key"]) == key);
            let shared = d["groups"].as_array().into_iter().flatten().find(|r| id.is_none() && s(&r["name"]) == *g).map(|r| s(&r["proxyId"])).filter(|x| !x.is_empty());
            let proxy = match p.map(|p| &p["proxy"]).filter(|x| x.is_object()) {
                Some(r) => proxy_name(&s(&r["vault"]), &s(&r["id"])),
                None => shared.map(|x| proxy_name(&ctx.vault, &x)).unwrap_or_default(),
            };
            let login = p.map(|p| login_text(p, &keys)).unwrap_or_default();
            (g.clone(), hosts.len(), hosts.iter().map(|h| s(&h["label"])).collect(), proxy, login)
        })
        .collect();
    if ctx.json {
        let out: Vec<Value> = rows.iter().map(|(g, _, hosts, proxy, login)| json!({ "group": g, "hosts": hosts, "proxy": proxy, "login": login })).collect();
        print_json(&json!(out));
        return Ok(());
    }
    let rows = rows.into_iter().map(|(g, n, _, proxy, login)| vec![if g.is_empty() { t().no_group.to_string() } else { g }, n.to_string(), proxy, login]).collect();
    table(&t().headers_groups, rows);
    Ok(())
}

/// Proxies a group may use: this vault's, then this person's own (for a team), each with its vault.
async fn all_proxies(ctx: &Ctx, d: &Value) -> Res<Vec<(String, Value)>> {
    let mut out: Vec<(String, Value)> = d["proxies"].as_array().into_iter().flatten().map(|p| (ctx.vault.clone(), p.clone())).collect();
    if ctx.vault != PERSONAL {
        let personal = ctx.c.call("vault_get", json!({})).await?;
        out.extend(personal["proxies"].as_array().into_iter().flatten().map(|p| (PERSONAL.to_string(), p.clone())));
    }
    Ok(out)
}

/// "user, key" as a group's login gives it.
fn login_text(p: &Value, keys: &Value) -> String {
    let auth = match s(&p["auth"]["type"]).as_str() {
        "key" => keys.as_array().into_iter().flatten().find(|k| k["id"] == p["auth"]["keyId"]).map(|k| s(&k["name"])).unwrap_or_else(|| "key".into()),
        "password" => t().password_word.to_string(),
        "agent" => "ssh-agent".into(),
        _ => String::new(),
    };
    [s(&p["user"]), auth].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(", ")
}

/* ---------- this person's own grouping of a team's hosts (--mine) ---------- */

/// The team's data and this person's grouping of its hosts; at first, a copy of the shared one.
async fn mine_of(ctx: &Ctx) -> Res<(Value, Value)> {
    if ctx.vault == PERSONAL {
        return fail(t().mine_team_only);
    }
    let teams = ctx.c.call("teams_list", json!({})).await?;
    let team = teams.as_array().into_iter().flatten().find(|e| s(&e["id"]) == ctx.vault).ok_or_else(|| Fail(t().unknown_vault.into()))?;
    let d = team["view"]["data"].clone();
    let layout = match &team["view"]["layout"] {
        l @ Value::Object(_) => l.clone(),
        _ => json!({ "groups": ordered_groups(&d, true).into_iter().filter(|(g, _)| !g.is_empty()).map(|(g, hosts)| json!({ "id": detour_core::new_id(), "name": g, "hosts": hosts.iter().map(|h| h["id"].clone()).collect::<Vec<_>>() })).collect::<Vec<_>>() }),
    };
    Ok((d, layout))
}

/// Groups of mine in order (with their ids), then the hosts in none of them under their shared groups.
fn ordered_mine<'a>(d: &'a Value, layout: &Value) -> Vec<(String, Option<String>, Vec<&'a Value>)> {
    let hosts: Vec<&Value> = d["hosts"].as_array().into_iter().flatten().collect();
    let mut placed = std::collections::HashSet::new();
    let mut out = vec![];
    for g in layout["groups"].as_array().into_iter().flatten() {
        let list: Vec<&Value> = g["hosts"].as_array().into_iter().flatten().filter_map(|id| hosts.iter().copied().find(|h| h["id"] == *id)).collect();
        placed.extend(list.iter().map(|h| s(&h["id"])));
        out.push((s(&g["name"]), Some(s(&g["id"])), list));
    }
    let mut rest = d.clone();
    if let Some(list) = rest["hosts"].as_array_mut() {
        list.retain(|h| !placed.contains(&s(&h["id"])));
    }
    let rest_names: Vec<(String, Vec<String>)> = ordered_groups(&rest, false).into_iter().map(|(g, h)| (g, h.iter().map(|h| s(&h["id"])).collect())).collect();
    for (g, ids) in rest_names {
        out.push((g, None, ids.iter().filter_map(|id| hosts.iter().copied().find(|h| s(&h["id"]) == *id)).collect()));
    }
    out
}

/// A group of mine by name or id: its place in the layout.
fn find_mine(layout: &Value, name: &str) -> Option<usize> {
    layout["groups"].as_array()?.iter().position(|g| s(&g["id"]) == name || s(&g["name"]).eq_ignore_ascii_case(name))
}

/// Shared groups shown below mine become mine, so they can be ordered among them.
fn materialize(d: &Value, layout: &mut Value) {
    let extra: Vec<Value> = ordered_mine(d, layout)
        .into_iter()
        .filter(|(g, id, _)| id.is_none() && !g.is_empty())
        .map(|(g, _, hosts)| json!({ "id": detour_core::new_id(), "name": g, "hosts": hosts.iter().map(|h| h["id"].clone()).collect::<Vec<_>>() }))
        .collect();
    layout["groups"].as_array_mut().unwrap().extend(extra);
}

async fn layout_put(ctx: &Ctx, layout: &Value) -> Res<()> {
    ctx.c.call("layout_set", json!({ "id": ctx.vault, "layout": layout })).await?;
    Ok(())
}

pub async fn mine_group_add(ctx: &Ctx, name: &str) -> Res<()> {
    let (_, mut layout) = mine_of(ctx).await?;
    if find_mine(&layout, name).is_some() {
        return fail(format!("{}: {name}", t().group_exists));
    }
    layout["groups"].as_array_mut().unwrap().push(json!({ "id": detour_core::new_id(), "name": name, "hosts": [] }));
    layout_put(ctx, &layout).await?;
    eprintln!("{}: {name}", t().group_added);
    Ok(())
}

pub async fn mine_group_rename(ctx: &Ctx, name: &str, to: &str) -> Res<()> {
    let (d, mut layout) = mine_of(ctx).await?;
    if find_mine(&layout, name).is_none() {
        materialize(&d, &mut layout);
    }
    let i = find_mine(&layout, name).ok_or_else(|| Fail(format!("{}: {name}", t().unknown_group)))?;
    layout["groups"][i]["name"] = json!(to);
    layout_put(ctx, &layout).await?;
    eprintln!("{}: {to}", t().group_saved);
    Ok(())
}

pub async fn mine_group_rm(ctx: &Ctx, name: &str) -> Res<()> {
    let (_, mut layout) = mine_of(ctx).await?;
    let i = find_mine(&layout, name).ok_or_else(|| Fail(format!("{}: {name}", t().unknown_group)))?;
    let removed = layout["groups"].as_array_mut().unwrap().remove(i);
    layout_put(ctx, &layout).await?;
    eprintln!("{}: {}", t().mine_group_removed, s(&removed["name"]));
    Ok(())
}

pub async fn mine_group_move(ctx: &Ctx, name: &str, before: Option<&str>) -> Res<()> {
    let (d, mut layout) = mine_of(ctx).await?;
    materialize(&d, &mut layout);
    let i = find_mine(&layout, name).ok_or_else(|| Fail(format!("{}: {name}", t().unknown_group)))?;
    let moving = layout["groups"].as_array_mut().unwrap().remove(i);
    let at = match before {
        Some(b) => find_mine(&layout, b).ok_or_else(|| Fail(format!("{}: {b}", t().unknown_group)))?,
        None => layout["groups"].as_array().unwrap().len(),
    };
    layout["groups"].as_array_mut().unwrap().insert(at, moving.clone());
    layout_put(ctx, &layout).await?;
    eprintln!("{}: {}", t().group_saved, s(&moving["name"]));
    Ok(())
}

/// Moves a host into a group of mine (a new one if there is none of that name; `none`: out of
/// mine, under its shared group), before another host or last.
pub async fn mine_host_move(ctx: &Ctx, name: &str, group: Option<&str>, before: Option<&str>) -> Res<()> {
    let (d, mut layout) = mine_of(ctx).await?;
    let host = find(&d["hosts"], name, "label", t().unknown_host)?.clone();
    let id = host["id"].clone();
    let before_id = before.map(|b| find(&d["hosts"], b, "label", t().unknown_host).map(|h| h["id"].clone())).transpose()?;
    let shown = ordered_mine(&d, &layout);
    let current = shown.iter().find(|(_, _, hosts)| hosts.iter().any(|h| h["id"] == id)).map(|(g, _, _)| g.clone()).unwrap_or_default();
    let target = match group {
        Some(g) if g.is_empty() || g.eq_ignore_ascii_case("none") => None,
        Some(g) => Some(g.to_string()),
        None => Some(current).filter(|g| !g.is_empty()),
    };
    for g in layout["groups"].as_array_mut().unwrap() {
        if let Some(list) = g["hosts"].as_array_mut() {
            list.retain(|h| *h != id);
        }
    }
    if let Some(target) = target {
        let i = match find_mine(&layout, &target) {
            Some(i) => i,
            None => {
                // A shared group shown below becomes mine with its hosts; else a new group.
                let hosts: Vec<Value> = shown.iter().find(|(g, gid, _)| gid.is_none() && g.eq_ignore_ascii_case(&target)).map(|(_, _, hs)| hs.iter().map(|h| h["id"].clone()).filter(|h| *h != id).collect()).unwrap_or_default();
                let name = shown.iter().find(|(g, _, _)| g.eq_ignore_ascii_case(&target)).map(|(g, _, _)| g.clone()).unwrap_or(target);
                layout["groups"].as_array_mut().unwrap().push(json!({ "id": detour_core::new_id(), "name": name, "hosts": hosts }));
                layout["groups"].as_array().unwrap().len() - 1
            }
        };
        let list = layout["groups"][i]["hosts"].as_array_mut().unwrap();
        let at = before_id.and_then(|b| list.iter().position(|h| *h == b)).unwrap_or(list.len());
        list.insert(at, id);
    }
    layout_put(ctx, &layout).await?;
    eprintln!("{}: {name}", t().host_saved);
    Ok(())
}

/* ---------- a group's settings ---------- */

/// A group's proxy and login, as given on the command line.
#[derive(clap::Args, Default)]
pub struct GroupOpts {
    #[arg(long)]
    pub mine: bool,
    #[arg(long, value_name = "PROXY")]
    pub proxy: Option<String>,
    #[arg(long, value_name = "PROXY", conflicts_with = "mine")]
    pub team_proxy: Option<String>,
    #[arg(long, short)]
    pub user: Option<String>,
    #[arg(long, short, conflicts_with_all = ["password", "agent", "no_login"])]
    pub key: Option<String>,
    #[arg(long, conflicts_with_all = ["agent", "no_login"])]
    pub password: bool,
    #[arg(long, conflicts_with = "no_login")]
    pub agent: bool,
    #[arg(long, conflicts_with = "user")]
    pub no_login: bool,
}

/// Sets this person's own proxy and login of a group, and (for those who manage the vault) the
/// proxy of the group for everyone.
pub async fn group_set(ctx: &Ctx, name: &str, o: GroupOpts) -> Res<()> {
    let (d, layout) = if o.mine { mine_of(ctx).await? } else { (data(ctx).await?, Value::Null) };
    let (key, group) = if o.mine {
        let i = find_mine(&layout, name).ok_or_else(|| Fail(format!("{}: {name}", t().unknown_group)))?;
        (format!("{}/mine:{}", ctx.vault, s(&layout["groups"][i]["id"])), s(&layout["groups"][i]["name"]))
    } else {
        let g = find_group(&d, name)?;
        (format!("{}/team:{g}", ctx.vault), g)
    };
    let proxies = all_proxies(ctx, &d).await?;
    let find_proxy = |n: &str, own_vault_only: bool| -> Res<Option<(String, String)>> {
        if n.eq_ignore_ascii_case("none") || n.is_empty() {
            return Ok(None);
        }
        proxies
            .iter()
            .filter(|(v, _)| !own_vault_only || *v == ctx.vault)
            .find(|(_, p)| s(&p["id"]) == n || s(&p["name"]).eq_ignore_ascii_case(n))
            .map(|(v, p)| Some((v.clone(), s(&p["id"]))))
            .ok_or_else(|| Fail(format!("{}: {n}", t().unknown_proxy)))
    };

    let personal = o.proxy.is_some() || o.user.is_some() || o.key.is_some() || o.password || o.agent || o.no_login;
    if personal {
        let all = ctx.c.call("group_prefs", json!({})).await?;
        let mut p = all.as_array().into_iter().flatten().find(|p| s(&p["key"]) == key).cloned().unwrap_or_else(|| json!({ "key": key, "hasPassword": false }));
        if let Some(n) = &o.proxy {
            p["proxy"] = match find_proxy(n, false)? {
                Some((vault, id)) => json!({ "vault": vault, "id": id }),
                None => Value::Null,
            };
        }
        if let Some(u) = &o.user {
            p["user"] = if u.is_empty() { Value::Null } else { json!(u) };
        }
        if o.no_login {
            p["user"] = Value::Null;
            p["auth"] = Value::Null;
        }
        if let Some(k) = &o.key {
            let keys = ctx.c.call("vault_get", json!({})).await?["keys"].clone();
            let key = find(&keys, k, "name", t().unknown_key)?;
            p["auth"] = json!({ "type": "key", "keyId": key["id"] });
        }
        if o.agent {
            p["auth"] = json!({ "type": "agent" });
        }
        if o.password {
            p["auth"] = json!({ "type": "password" });
            p["newPassword"] = json!(secret(t().host_password)?);
        }
        ctx.c.call("group_prefs_set", json!({ "prefs": p, "drop": false })).await?;
    }
    if let Some(n) = &o.team_proxy {
        let proxy = find_proxy(n, true)?;
        let mut next = d.clone();
        with_records(&mut next);
        for g in next["groups"].as_array_mut().unwrap() {
            if s(&g["name"]) == group {
                g["proxyId"] = proxy.as_ref().map(|(_, id)| json!(id)).unwrap_or(Value::Null);
            }
        }
        put(ctx, &d, &next).await?;
    }
    eprintln!("{}: {group}", t().group_saved);
    Ok(())
}

pub async fn group_add(ctx: &Ctx, name: &str) -> Res<()> {
    let base = data(ctx).await?;
    if find_group(&base, name).is_ok() {
        return fail(format!("{}: {name}", t().group_exists));
    }
    let mut next = base.clone();
    with_records(&mut next);
    let last = next["groups"].as_array().unwrap().iter().filter_map(|g| g["order"].as_f64()).fold(0.0, f64::max);
    next["groups"].as_array_mut().unwrap().push(json!({ "id": detour_core::new_id(), "name": name, "order": last + 1.0 }));
    put(ctx, &base, &next).await?;
    eprintln!("{}: {name}", t().group_added);
    Ok(())
}

pub async fn group_rename(ctx: &Ctx, name: &str, to: &str) -> Res<()> {
    let base = data(ctx).await?;
    let from = find_group(&base, name)?;
    let mut next = base.clone();
    for g in next["groups"].as_array_mut().into_iter().flatten() {
        if s(&g["name"]) == from {
            g["name"] = json!(to);
        }
    }
    for h in next["hosts"].as_array_mut().unwrap() {
        if group_of(h) == from {
            h["group"] = json!(to);
        }
    }
    put(ctx, &base, &next).await?;
    eprintln!("{}: {to}", t().group_saved);
    Ok(())
}

pub async fn group_rm(ctx: &Ctx, name: &str) -> Res<()> {
    let base = data(ctx).await?;
    let from = find_group(&base, name)?;
    let mut next = base.clone();
    if let Some(list) = next["groups"].as_array_mut() {
        list.retain(|g| s(&g["name"]) != from);
    }
    for h in next["hosts"].as_array_mut().unwrap() {
        if group_of(h) == from {
            h["group"] = json!("");
        }
    }
    put(ctx, &base, &next).await?;
    eprintln!("{}: {from}", t().group_removed);
    Ok(())
}

/// Moves a group before another, or last.
pub async fn group_move(ctx: &Ctx, name: &str, before: Option<&str>) -> Res<()> {
    let base = data(ctx).await?;
    let from = find_group(&base, name)?;
    let before = before.map(|b| find_group(&base, b)).transpose()?;
    let mut next = base.clone();
    with_records(&mut next);
    let mut others: Vec<(String, f64)> = next["groups"].as_array().unwrap().iter().filter(|g| s(&g["name"]) != from).map(|g| (s(&g["name"]), g["order"].as_f64().unwrap_or(0.0))).collect();
    others.sort_by(|a, b| a.1.total_cmp(&b.1));
    let i = before.as_ref().and_then(|b| others.iter().position(|(g, _)| g == b)).unwrap_or(others.len());
    let order = between(i.checked_sub(1).map(|k| others[k].1), others.get(i).map(|x| x.1));
    for g in next["groups"].as_array_mut().unwrap() {
        if s(&g["name"]) == from {
            g["order"] = json!(order);
        }
    }
    put(ctx, &base, &next).await?;
    eprintln!("{}: {from}", t().group_saved);
    Ok(())
}

/// Moves a host into a group (its own if none is given), before another host or last.
pub async fn host_move(ctx: &Ctx, name: &str, group: Option<&str>, before: Option<&str>) -> Res<()> {
    let base = data(ctx).await?;
    let host = find(&base["hosts"], name, "label", t().unknown_host)?.clone();
    let id = s(&host["id"]);
    let target = match group {
        Some(g) if g.is_empty() || g.eq_ignore_ascii_case("none") => String::new(),
        Some(g) => find_group(&base, g).unwrap_or_else(|_| g.to_string()),
        None => group_of(&host),
    };
    let before_id = before.map(|b| find(&base["hosts"], b, "label", t().unknown_host).map(|h| s(&h["id"]))).transpose()?;
    let mut next = base.clone();
    let list = sorted_hosts(base["hosts"].as_array().unwrap().iter().filter(|h| s(&h["id"]) != id && group_of(h) == target).collect());
    // Hosts of the group without a place get one first, in the order they are listed.
    let renumber = list.iter().any(|h| h["order"].as_f64().is_none());
    let place = |k: usize| if renumber { Some(k as f64 + 1.0) } else { list[k]["order"].as_f64() };
    let i = before_id.as_ref().and_then(|b| list.iter().position(|h| &s(&h["id"]) == b)).unwrap_or(list.len());
    let order = between(i.checked_sub(1).and_then(place), (i < list.len()).then(|| place(i)).flatten());
    let ids: Vec<String> = list.iter().map(|h| s(&h["id"])).collect();
    for h in next["hosts"].as_array_mut().unwrap() {
        let hid = s(&h["id"]);
        if hid == id {
            h["group"] = json!(target);
            h["order"] = json!(order);
        } else if renumber && let Some(k) = ids.iter().position(|x| *x == hid) {
            h["order"] = json!(k as f64 + 1.0);
        }
    }
    put(ctx, &base, &next).await?;
    eprintln!("{}: {name}", t().host_saved);
    Ok(())
}

pub async fn ls(ctx: &Ctx, query: Option<&str>) -> Res<()> {
    let d = data(ctx).await?;
    let q = query.map(str::to_lowercase);
    // In the app's order: group by group, host by host.
    let hosts: Vec<&Value> = ordered_groups(&d, false).into_iter().flat_map(|(_, h)| h).filter(|h| q.as_ref().is_none_or(|q| [&h["label"], &h["address"], &h["user"], &h["group"]].iter().any(|v| s(v).to_lowercase().contains(q.as_str())))).collect();
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
    #[arg(long, value_name = "HOST", conflicts_with = "proxy")]
    pub via: Option<String>,
    #[arg(long, value_name = "PROXY")]
    pub proxy: Option<String>,
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
        h["proxyId"] = Value::Null;
    }
    if let Some(p) = o.proxy {
        if p.eq_ignore_ascii_case("none") {
            h["proxy"] = Value::Null;
            h["proxyId"] = Value::Null;
        } else {
            let proxy = find(&d["proxies"], &p, "name", t().unknown_proxy)?;
            h["proxyId"] = proxy["id"].clone();
            // Unless it already went through this one, what older versions read goes too.
            if s(&h["proxyId"]) != s(&proxy["id"]) || h["proxy"]["type"] == "jump" {
                h["proxy"] = Value::Null;
            }
        }
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

/// What a Detour file leaves out (`hosts export --file`).
#[derive(clap::Args, Debug, Default)]
pub struct LeaveOut {
    #[arg(long, requires = "file")]
    no_passwords: bool,
    #[arg(long, requires = "file")]
    no_keys: bool,
    #[arg(long, requires = "file")]
    no_proxies: bool,
    #[arg(long, requires = "file")]
    no_known_hosts: bool,
    #[arg(long, requires = "file")]
    no_groups: bool,
}

/// A password for a Detour file, typed twice.
fn new_file_password() -> Res<String> {
    let pw = crate::conn::secret(t().file_password)?;
    if pw.is_empty() {
        return fail(t().cancelled);
    }
    if crate::conn::secret(t().repeat_password)? != pw {
        return fail(t().passwords_differ);
    }
    Ok(pw)
}

/// What a Detour file holds, in one line.
fn file_counts(c: &Value) -> String {
    let n = |k: &str| c[k].as_u64().unwrap_or(0);
    let parts = [("hosts", t().count_hosts), ("keys", t().count_keys), ("passwords", t().count_passwords), ("proxies", t().count_proxies), ("knownHosts", t().count_known_hosts)];
    parts.iter().filter(|(k, _)| n(k) > 0).map(|(k, name)| format!("{name}: {}", n(k))).collect::<Vec<_>>().join(", ")
}

pub async fn host_export_file(ctx: &Ctx, names: Vec<String>, file: &std::path::Path, out: LeaveOut) -> Res<()> {
    let d = data(ctx).await?;
    let ids: Vec<String> = if names.is_empty() {
        d["hosts"].as_array().into_iter().flatten().map(|h| s(&h["id"])).collect()
    } else {
        names.iter().map(|n| find(&d["hosts"], n, "label", t().unknown_host).map(|h| s(&h["id"]))).collect::<Res<_>>()?
    };
    let include = json!({ "passwords": !out.no_passwords, "keys": !out.no_keys, "proxies": !out.no_proxies, "knownHosts": !out.no_known_hosts, "groups": !out.no_groups });
    let password = new_file_password()?;
    let path = std::path::absolute(file).unwrap_or(file.to_path_buf()).display().to_string();
    let r = ctx.c.call("hosts_file_export", json!({ "vaultId": ctx.vault, "hostIds": ids, "include": include, "password": password, "path": path })).await?;
    if ctx.json {
        print_json(&r);
        return Ok(());
    }
    eprintln!("{} {} ({})", t().saved_to, s(&r["path"]), file_counts(&r["counts"]));
    Ok(())
}

pub async fn host_import_file(ctx: &Ctx, file: &std::path::Path, names: Vec<String>) -> Res<()> {
    let path = std::path::absolute(file).unwrap_or(file.to_path_buf()).display().to_string();
    let password = crate::conn::secret(t().file_password)?;
    let preview = ctx.c.call("hosts_file_open", json!({ "path": path, "password": password })).await?;
    let hosts = &preview["hosts"];
    // By default the hosts not here yet; named ones even if they are (they come as copies).
    let ids: Vec<String> = if names.is_empty() {
        hosts.as_array().into_iter().flatten().filter(|h| h["exists"] == false).map(|h| s(&h["id"])).collect()
    } else {
        names.iter().map(|n| find(hosts, n, "label", t().unknown_host).map(|h| s(&h["id"]))).collect::<Res<_>>()?
    };
    if ids.is_empty() {
        eprintln!("{}", t().nothing_to_import);
        return Ok(());
    }
    let r = ctx.c.call("hosts_file_import", json!({ "path": path, "password": password, "hostIds": ids })).await?;
    if ctx.json {
        print_json(&r);
        return Ok(());
    }
    let n = |k: &str| r[k].as_u64().unwrap_or(0);
    let parts = [("hosts", t().count_hosts), ("keys", t().count_keys), ("proxies", t().count_proxies), ("known", t().count_known_hosts)];
    let done: Vec<String> = parts.iter().filter(|(k, _)| n(k) > 0).map(|(k, name)| format!("{name}: {}", n(k))).collect();
    eprintln!("{}: {}", t().imported, done.join(", "));
    Ok(())
}

/* ---------- running commands ---------- */

/// The most of piped input `exec` passes on: it travels in one message to the core.
const MAX_STDIN: usize = 32 * 1024 * 1024;

pub async fn exec(ctx: &Ctx, name: &str, command: &str) -> Res<i32> {
    let h = host(ctx, name).await?;
    // Piped input goes to the command; a terminal's doesn't (the command gets none, as with ssh -n).
    let stdin = if std::io::stdin().is_terminal() {
        None
    } else {
        let mut data = Vec::new();
        std::io::stdin().take(MAX_STDIN as u64 + 1).read_to_end(&mut data).map_err(|e| Fail(e.to_string()))?;
        if data.len() > MAX_STDIN {
            return fail(t().stdin_too_big);
        }
        (!data.is_empty()).then(|| base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &data))
    };
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
        let (c, vault, host_id, command, stdin) = (c.clone(), vault.clone(), host_id.clone(), command.to_string(), stdin.clone());
        async move { c.call("host_exec", json!({ "vaultId": vault, "hostId": host_id, "command": command, "secret": secret, "trustFingerprint": trust, "stream": out_id, "errStream": err_id, "stdin": stdin })).await }
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

/* ---------- proxies ---------- */

fn proxy_summary(k: &Value) -> String {
    match k["type"].as_str() {
        Some("wireguard") => s(&k["endpoint"]),
        Some("vless") => {
            let mut parts = vec![format!("{}:{}", s(&k["address"]), s(&k["port"]))];
            match k["security"].as_str() {
                Some("reality") => parts.push("REALITY".into()),
                Some("tls") => parts.push("TLS".into()),
                _ => {}
            }
            if k["transport"] == "xhttp" {
                parts.push("XHTTP".into());
            }
            parts.join(" ")
        }
        _ => {
            let user = s(&k["user"]);
            format!("{}{}:{}", if user.is_empty() { String::new() } else { format!("{user}@") }, s(&k["address"]), s(&k["port"]))
        }
    }
}

pub async fn proxies(ctx: &Ctx) -> Res<()> {
    let d = data(ctx).await?;
    if ctx.json {
        print_json(&d["proxies"]);
        return Ok(());
    }
    let used = |id: &Value| d["hosts"].as_array().into_iter().flatten().filter(|h| &h["proxyId"] == id).count();
    let rows = d["proxies"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| vec![s(&p["name"]), s(&p["kind"]["type"]), proxy_summary(&p["kind"]), used(&p["id"]).to_string()])
        .collect();
    table(&t().headers_proxies, rows);
    Ok(())
}

/// What a new proxy is made from.
pub enum NewProxy {
    /// A `vless://` link or a WireGuard config.
    Text(String),
    /// A file holding one (`-`: standard input).
    File(std::path::PathBuf),
    /// SOCKS5 or HTTP at `host:port`, with a user whose password is asked.
    Plain(&'static str, String, Option<String>),
}

pub async fn proxy_add(ctx: &Ctx, name: &str, what: NewProxy) -> Res<()> {
    let base = data(ctx).await?;
    if !base.get("proxies").is_some_and(Value::is_array) {
        return fail(t().old_core);
    }
    let (kind, secrets) = match what {
        NewProxy::Text(text) => {
            let p = ctx.c.call("proxy_parse", json!({ "text": text })).await?;
            (p["kind"].clone(), p["secrets"].clone())
        }
        NewProxy::File(path) => {
            let mut text = String::new();
            if path.as_os_str() == "-" {
                std::io::stdin().read_to_string(&mut text).map_err(|e| Fail(e.to_string()))?;
            } else {
                text = std::fs::read_to_string(&path).map_err(|e| Fail(format!("{}: {e}", path.display())))?;
            }
            let p = ctx.c.call("proxy_parse", json!({ "text": text })).await?;
            (p["kind"].clone(), p["secrets"].clone())
        }
        NewProxy::Plain(kind, at, user) => {
            let (address, port) = at.rsplit_once(':').and_then(|(a, p)| Some((a.trim_matches(['[', ']']).to_string(), p.parse::<u16>().ok()?))).ok_or_else(|| Fail(format!("{at}: host:port")))?;
            let user = user.unwrap_or_default();
            let secrets = if user.is_empty() { json!({}) } else { json!({ "password": secret(t().proxy_password)? }) };
            (json!({ "type": kind, "address": address, "port": port, "user": user }), secrets)
        }
    };
    let mut next = base.clone();
    next["proxies"].as_array_mut().unwrap().push(json!({ "id": detour_core::new_id(), "name": name, "kind": kind, "saved": [], "newSecrets": secrets }));
    put(ctx, &base, &next).await?;
    eprintln!("{}: {name}", t().proxy_added);
    Ok(())
}

pub async fn proxy_rm(ctx: &Ctx, name: &str) -> Res<()> {
    let base = data(ctx).await?;
    let id = find(&base["proxies"], name, "name", t().unknown_proxy)?["id"].clone();
    let mut next = base.clone();
    next["proxies"].as_array_mut().unwrap().retain(|p| p["id"] != id);
    // Hosts that went through it connect directly.
    for h in next["hosts"].as_array_mut().unwrap() {
        if h["proxyId"] == id {
            h["proxyId"] = Value::Null;
            h["proxy"] = Value::Null;
        }
    }
    put(ctx, &base, &next).await?;
    eprintln!("{}: {name}", t().proxy_removed);
    Ok(())
}

pub async fn proxy_check(ctx: &Ctx, name: &str) -> Res<()> {
    let d = data(ctx).await?;
    let p = find(&d["proxies"], name, "name", t().unknown_proxy)?;
    let r = ctx.c.call("proxy_check", json!({ "vaultId": ctx.vault, "proxy": p })).await?;
    if ctx.json {
        print_json(&r);
    } else {
        println!("{}", (t().proxy_works)(r["ms"].as_u64().unwrap_or(0)));
    }
    Ok(())
}

pub async fn proxy_export(ctx: &Ctx, name: &str) -> Res<()> {
    let d = data(ctx).await?;
    let p = find(&d["proxies"], name, "name", t().unknown_proxy)?;
    let r = ctx.c.call("proxy_export", json!({ "vaultId": ctx.vault, "proxyId": p["id"] })).await?;
    println!("{}", s(&r["text"]).trim_end());
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
