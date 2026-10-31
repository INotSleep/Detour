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
    let rows = all
        .iter()
        .map(|tk| {
            let hosts: Vec<String> = tk["hosts"].as_array().into_iter().flatten().map(host_name).collect();
            vec![s(&tk["id"])[..8.min(s(&tk["id"]).len())].to_string(), s(&tk["agent"]), hosts.join(", "), scopes(&tk["scopes"]), until(&tk["expiresAt"]), ago(&tk["lastUsed"])]
        })
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
