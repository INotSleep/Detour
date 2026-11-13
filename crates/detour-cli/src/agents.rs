//! `detour approve` and `detour agents`: the person's side of AI agents' access,
//! for when there is no window to answer in.

use std::io::{IsTerminal, Write};

use serde_json::{Value, json};

use crate::cmds::{Ctx, table};
use crate::conn::{Fail, Res, fail};
use crate::i18n::t;

const TTLS: [&str; 4] = ["2h", "1d", "7d", "never"];

fn s(v: &Value) -> String {
    v.as_str().map(str::to_string).unwrap_or_default()
}

/// A line typed in answer to a question on the terminal.
fn ask(prompt: &str) -> String {
    eprint!("{prompt}");
    let _ = std::io::stderr().flush();
    let mut line = String::new();
    let _ = std::io::stdin().read_line(&mut line);
    line.trim().to_lowercase()
}

fn yes(a: &str) -> bool {
    matches!(a, "y" | "yes" | "д" | "да")
}

fn host_name(h: &Value) -> String {
    match h["team"].as_str() {
        Some(team) => format!("{team}/{}", s(&h["label"])),
        None => s(&h["label"]),
    }
}

fn scopes(v: &Value) -> String {
    v.as_array().into_iter().flatten().map(s).collect::<Vec<_>>().join(", ")
}

fn ttl_name(ttl: &str) -> &'static str {
    TTLS.iter().position(|x| *x == ttl).map(|i| t().ttl_names[i]).unwrap_or("")
}

/// Goes through what agents wait on, one by one.
pub async fn approve(ctx: &Ctx) -> Res<()> {
    // Only the person at this terminal answers: never a pipe an agent could write to.
    if !std::io::stdin().is_terminal() {
        return fail(t().approve_needs_terminal);
    }
    let pending = ctx.c.call("agents_pending", json!({})).await?;
    let list = pending.as_array().cloned().unwrap_or_default();
    if list.is_empty() {
        eprintln!("{}", t().no_requests);
        return Ok(());
    }
    for p in list {
        let agent = s(&p["agent"]);
        let answer = if p["type"] == "access" {
            let hosts: Vec<String> = p["hosts"].as_array().into_iter().flatten().map(host_name).collect();
            eprintln!("\n{agent} {}", t().asks_access);
            eprintln!("  {}: {}", t().label_hosts, hosts.join(", "));
            eprintln!("  {}: {}", t().label_scopes, scopes(&p["scopes"]));
            if let Some(reason) = p["reason"].as_str() {
                eprintln!("  {}: {reason}", t().label_reason);
            }
            let asked = s(&p["ttl"]);
            let options: Vec<String> = t().ttl_names.iter().enumerate().map(|(i, n)| format!("[{}] {n}", i + 1)).collect();
            let picked = ask(&format!("{} {} ({}: {}): ", t().ttl_question, options.join("  "), t().enter_default, ttl_name(&asked)));
            let ttl = picked.parse::<usize>().ok().and_then(|i| TTLS.get(i.wrapping_sub(1))).map(|x| x.to_string()).unwrap_or(asked);
            let allow = yes(&ask(t().allow_question));
            json!({ "id": p["id"], "allow": allow, "ttl": ttl })
        } else {
            eprintln!("\n{agent} {} {}:", t().wants_to_run, host_name(&p["host"]));
            eprintln!("  {}", s(&p["command"]).replace('\n', "\n  "));
            let a = ask(t().allow_command_question);
            json!({ "id": p["id"], "allow": yes(&a) || a == "a", "always": a == "a" })
        };
        let allowed = answer["allow"] == true;
        match ctx.c.call("agents_answer", answer).await {
            Ok(_) => eprintln!("{}", if allowed { t().allowed } else { t().refused }),
            // Waited too long and went away, or someone answered in the window meanwhile.
            Err(e) if e["kind"] == "notFound" => eprintln!("{}", t().request_gone),
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

fn until(ms: &Value) -> String {
    let Some(ms) = ms.as_i64() else { return t().until_revoked.into() };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
    let min = (ms - now) / 60_000;
    match min {
        m if m <= 0 => t().expired.into(),
        m if m < 60 => format!("{m}m"),
        m if m < 60 * 24 => format!("{}h", m / 60),
        m => format!("{}d", m / 60 / 24),
    }
}

fn ago(ms: &Value) -> String {
    let Some(ms) = ms.as_i64() else { return String::new() };
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0);
    match (now - ms) / 60_000 {
        m if m < 1 => t().just_now.into(),
        m if m < 60 => format!("{m}m"),
        m if m < 60 * 24 => format!("{}h", m / 60),
        m => format!("{}d", m / 60 / 24),
    }
}

/// What an access made ahead reaches and allows, as given on the command line.
#[derive(clap::Args, Default)]
pub struct RuleOpts {
    #[arg(long)]
    pub all: bool,
    #[arg(long = "vault", value_name = "VAULT")]
    pub vaults: Vec<String>,
    #[arg(long = "group", value_name = "GROUP")]
    pub groups: Vec<String>,
    #[arg(long = "my-group", value_name = "GROUP")]
    pub my_groups: Vec<String>,
    #[arg(long = "host", value_name = "HOST")]
    pub hosts: Vec<String>,
    #[arg(long)]
    pub exec: bool,
    #[arg(long)]
    pub files: bool,
    #[arg(long)]
    pub files_write: bool,
    #[arg(long)]
    pub tunnels: bool,
    #[arg(long)]
    pub edit: bool,
    #[arg(long)]
    pub accept_new_keys: bool,
    #[arg(long, value_parser = TTLS)]
    pub ttl: Option<String>,
}

/// What the options reach and allow, in the core's words.
async fn rule_of(ctx: &Ctx, o: &RuleOpts) -> Res<(Value, Value, Value)> {
    let mut reach = Vec::new();
    if o.all {
        reach.push(json!({ "type": "all" }));
    }
    for v in &o.vaults {
        reach.push(json!({ "type": "vault", "vaultId": crate::cmds::vault_id(&ctx.c, Some(v)).await? }));
    }
    // Groups are of the vault `-T` names (the personal one by default).
    for g in &o.groups {
        reach.push(json!({ "type": "group", "vaultId": ctx.vault, "name": g }));
    }
    if !o.my_groups.is_empty() {
        let teams = ctx.c.call("teams_list", json!({})).await?;
        let layout = teams.as_array().into_iter().flatten().find(|e| s(&e["id"]) == ctx.vault).map(|e| e["view"]["layout"].clone()).unwrap_or(Value::Null);
        for g in &o.my_groups {
            let id = layout["groups"].as_array().into_iter().flatten().find(|x| s(&x["name"]).eq_ignore_ascii_case(g) || s(&x["id"]) == *g).map(|x| s(&x["id"])).ok_or_else(|| Fail(format!("{}: {g}", t().unknown_group)))?;
            reach.push(json!({ "type": "myGroup", "vaultId": ctx.vault, "id": id }));
        }
    }
    let all = ctx.c.call("agent_hosts", json!({})).await?;
    let mut hosts = Vec::new();
    for name in &o.hosts {
        let n = name.to_lowercase();
        let found: Vec<&Value> = all
            .as_array()
            .into_iter()
            .flatten()
            .filter(|h| s(&h["id"]) == *name || s(&h["name"]).to_lowercase() == n || h["team"].as_str().is_some_and(|tm| format!("{}/{}", tm.to_lowercase(), s(&h["name"]).to_lowercase()) == n))
            .collect();
        match found.as_slice() {
            [h] => hosts.push(json!({ "vaultId": h["vaultId"], "hostId": h["id"] })),
            [] => return fail(format!("{}: {name}", t().unknown_host)),
            _ => return fail(format!("{}: {name}", t().which_host)),
        }
    }
    if reach.is_empty() && hosts.is_empty() {
        return fail(t().rule_needs_hosts);
    }
    let mut scopes = vec![if o.exec { "exec" } else { "connect" }];
    for (on, scope) in [(o.files || o.files_write, "files:read"), (o.files_write, "files:write"), (o.tunnels, "tunnels"), (o.edit, "vault:edit"), (o.accept_new_keys, "hostkeys:new")] {
        if on {
            scopes.push(scope);
        }
    }
    Ok((json!(reach), json!(hosts), json!(scopes)))
}

/// Gives an agent access ahead of its asking. With `client`, the token goes into that AI
/// client's config of Detour's MCP server; otherwise it is printed, once.
pub async fn add(ctx: &Ctx, dir: &std::path::Path, agent: &str, o: RuleOpts, client: Option<&str>) -> Res<()> {
    let (reach, hosts, scopes) = rule_of(ctx, &o).await?;
    let made = ctx.c.call("agents_set", json!({ "agent": agent, "reach": reach, "hosts": hosts, "scopes": scopes, "ttl": o.ttl.as_deref().unwrap_or("never") })).await?;
    let token = s(&made["token"]);
    match client {
        Some(id) => {
            use detour_core::mcp_clients::{self, Dirs, Server};
            let dirs = Dirs::of_user().ok_or_else(|| Fail("home directory".into()))?;
            let me = std::env::current_exe().map_err(|e| Fail(e.to_string()))?;
            let own_dir = (dir != detour_core::store::Store::default_dir()).then_some(dir);
            let all = mcp_clients::clients(&dirs, &Server::new(me.clone(), own_dir));
            let c = all.iter().find(|c| c.id.eq_ignore_ascii_case(id) || c.name.eq_ignore_ascii_case(id)).ok_or_else(|| Fail(format!("{}: {id}", t().unknown_client)))?;
            mcp_clients::add_token(&dirs, c.id, &Server::new(me, own_dir), true, &token).map_err(|e| Fail(crate::i18n::error_text(&serde_json::to_value(&e).unwrap_or_default())))?;
            eprintln!("{} {}", t().rule_given_to, c.name);
            eprintln!("{}", t().mcp_restart);
        }
        None => {
            eprintln!("{}", t().rule_token);
            println!("{token}");
        }
    }
    Ok(())
}

/// Changes what an access reaches and allows; its token stays.
pub async fn edit(ctx: &Ctx, which: &str, o: RuleOpts) -> Res<()> {
    let tokens = ctx.c.call("agents_list", json!({})).await?;
    let all = tokens.as_array().cloned().unwrap_or_default();
    let by_id: Vec<&Value> = all.iter().filter(|tk| s(&tk["id"]).starts_with(which)).collect();
    let picked: Vec<&Value> = if by_id.len() == 1 { by_id } else { all.iter().filter(|tk| s(&tk["agent"]).eq_ignore_ascii_case(which)).collect() };
    let tk = match picked.as_slice() {
        [one] => *one,
        [] => return fail(format!("{}: {which}", t().unknown_agent)),
        _ => return fail(format!("{}: {which}", t().which_agent)),
    };
    let (reach, hosts, scopes) = rule_of(ctx, &o).await?;
    let mut p = json!({ "id": tk["id"], "reach": reach, "hosts": hosts, "scopes": scopes });
    if let Some(ttl) = &o.ttl {
        p["ttl"] = json!(ttl);
    }
    ctx.c.call("agents_set", p).await?;
    eprintln!("{}: {}", t().rule_saved, s(&tk["agent"]));
    Ok(())
}

/// What a token reaches, briefly: "all", "Acme", "Acme: Prod", and the hosts it names.
fn reach_text(tk: &Value, teams: &Value) -> String {
    let team = |id: &str| if id == crate::cmds::PERSONAL { t().personal_vault.to_string() } else { teams.as_array().into_iter().flatten().find(|e| s(&e["id"]) == id).map(|e| s(&e["view"]["name"])).unwrap_or_else(|| id.to_string()) };
    let mut parts: Vec<String> = tk["reach"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|r| match s(&r["type"]).as_str() {
            "all" => t().all_hosts.to_string(),
            "vault" => team(&s(&r["vaultId"])),
            "group" => format!("{}: {}", team(&s(&r["vaultId"])), s(&r["name"])),
            _ => {
                let layout = teams.as_array().into_iter().flatten().find(|e| e["id"] == r["vaultId"]).map(|e| e["view"]["layout"].clone()).unwrap_or(Value::Null);
                let name = layout["groups"].as_array().into_iter().flatten().find(|g| g["id"] == r["id"]).map(|g| s(&g["name"])).unwrap_or_default();
                format!("{}: {name}", team(&s(&r["vaultId"])))
            }
        })
        .collect();
    parts.extend(tk["named"].as_array().into_iter().flatten().map(host_name));
    parts.join(", ")
}

pub async fn list(ctx: &Ctx) -> Res<()> {
    let tokens = ctx.c.call("agents_list", json!({})).await?;
    if ctx.json {
        println!("{}", serde_json::to_string_pretty(&tokens).unwrap_or_default());
        return Ok(());
    }
    let all = tokens.as_array().cloned().unwrap_or_default();
    if all.is_empty() {
        eprintln!("{}", t().no_agents);
        return Ok(());
    }
    let teams = ctx.c.call("teams_list", json!({})).await.unwrap_or(json!([]));
    let rows = all
        .iter()
        .map(|tk| vec![s(&tk["id"])[..8.min(s(&tk["id"]).len())].to_string(), s(&tk["agent"]), reach_text(tk, &teams), scopes(&tk["scopes"]), until(&tk["expiresAt"]), ago(&tk["lastUsed"])])
        .collect();
    table(&t().headers_agents, rows);
    Ok(())
}

/// Revokes a token by its id (or the start of it), or every token of an agent by its name.
pub async fn revoke(ctx: &Ctx, which: &str) -> Res<()> {
    let tokens = ctx.c.call("agents_list", json!({})).await?;
    let all = tokens.as_array().cloned().unwrap_or_default();
    let by_id: Vec<&Value> = all.iter().filter(|tk| s(&tk["id"]).starts_with(which)).collect();
    let picked: Vec<&Value> = if by_id.len() == 1 { by_id } else { all.iter().filter(|tk| s(&tk["agent"]).eq_ignore_ascii_case(which)).collect() };
    if picked.is_empty() {
        return Err(Fail(format!("{}: {which}", t().unknown_agent)));
    }
    for tk in picked {
        ctx.c.call("agents_revoke", json!({ "id": tk["id"] })).await?;
        eprintln!("{} {}", t().revoked, s(&tk["agent"]));
    }
    Ok(())
}
