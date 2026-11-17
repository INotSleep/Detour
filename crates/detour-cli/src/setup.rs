//! Setting up where vaults live, from the command line: syncing the personal vault with
//! the person's other devices, signing in with GitHub, creating, joining and recovering
//! teams. A server without a desktop gets the person's vault this way.
//!
//! A place is a folder, a Git repository's URL (`ssh://`, `git@host:path`, `https://`), or a
//! cloud drive's folder: `onedrive:Detour/Team`, `gdrive:Detour/Team`, `dropbox:/Detour/Team`,
//! or a share link to it.
//! Over SSH Detour logs in with a key of the vault (`--key`) or ssh-agent; over https with
//! a token (asked) or by signing in with GitHub (`--github`). A server seen for the first
//! time shows its key fingerprint and is trusted only when the person says so.

use std::time::Duration;

use detour_ipc::Client;
use serde_json::{Value, json};

use crate::cmds::Ctx;
use crate::conn::{Fail, Res, confirm, fail, line, secret};
use crate::i18n::{error_text, t};

// How Detour logs in to a Git repository, as given on the command line (texts in i18n).
#[derive(clap::Args, Default)]
pub struct GitLogin {
    #[arg(long, conflicts_with_all = ["agent", "token", "github"])]
    pub key: Option<String>,
    #[arg(long, conflicts_with_all = ["token", "github"])]
    pub agent: bool,
    #[arg(long, conflicts_with = "github")]
    pub token: bool,
    #[arg(long)]
    pub github: bool,
}

const INSTALL_URL: &str = "https://github.com/apps/detour-sync/installations/new";

fn s(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// A folder, a repository with how to log in to it (the core's `url`, `auth`, `token`,
/// `github`), or a drive folder (the core's `DriveRemote`).
enum Place {
    Folder(String),
    Git(Value),
    Drive(Value),
}

/// `onedrive:path`, `gdrive:path`, `dropbox:/path`, or a share link: the drive and what follows.
fn drive_place(arg: &str) -> Option<(detour_core::drive::Provider, Option<&str>)> {
    let arg = arg.trim();
    if let Some(p) = detour_core::drive::link::provider_of(arg) {
        return Some((p, None));
    }
    let (name, rest) = arg.split_once(':')?;
    // `C:\folder` is a folder, not a drive.
    if name.len() < 2 {
        return None;
    }
    Some((detour_core::drive::Provider::parse(name)?, Some(rest)))
}

/// Finds a drive folder by its path from the top of the drive, signing in first if needed.
async fn drive_folder(c: &Client, provider: detour_core::drive::Provider, path: Option<&str>, url: &str) -> Res<Value> {
    let id = provider.id();
    let accounts = c.call("drive_accounts", json!({})).await?;
    if !accounts.as_array().into_iter().flatten().any(|a| a["provider"] == id && a["signedIn"] == true) {
        drive_login(c, id).await?;
    }
    let Some(path) = path else { return Ok(c.call("drive_link", json!({ "url": url, "provider": id })).await?) };
    let mut here = Value::Null;
    for part in path.split(['/', '\\']).filter(|p| !p.is_empty()) {
        let listing = c.call("drive_browse", json!({ "provider": id, "folder": here })).await?;
        let found = listing["folders"].as_array().into_iter().flatten().find(|f| s(&f["name"]).eq_ignore_ascii_case(part)).cloned();
        let Some(f) = found else { return fail(&format!("{}: {part}", t().drive_no_folder)) };
        let parent_label = s(&listing["folder"]["label"]);
        let label = if parent_label.is_empty() { s(&f["name"]) } else { format!("{parent_label}/{}", s(&f["name"])) };
        here = json!({ "provider": id, "id": f["id"], "drive": f["drive"], "label": label });
    }
    if here.is_null() {
        return fail(t().drive_no_folder);
    }
    Ok(here)
}

fn is_git(arg: &str) -> bool {
    if arg.contains("://") {
        return true;
    }
    // `user@host:path`, but not `C:\folder`.
    let scp = arg.split_once(':').is_some_and(|(host, _)| host.contains('@') && !host.contains(['/', '\\']));
    scp && !std::path::Path::new(arg).exists()
}

async fn place(c: &Client, arg: &str, login: &GitLogin) -> Res<Place> {
    if let Some((provider, path)) = drive_place(arg) {
        return Ok(Place::Drive(drive_folder(c, provider, path, arg).await?));
    }
    if !is_git(arg) {
        let path = std::path::absolute(arg).map_err(|e| Fail(format!("{arg}: {e}")))?;
        return Ok(Place::Folder(path.display().to_string()));
    }
    let url = arg.trim().to_string();
    let https = url.starts_with("https://") || url.starts_with("http://");
    if login.github {
        github_ready(c, &url).await?;
        return Ok(Place::Git(json!({ "url": url, "auth": { "type": "password" }, "token": null, "github": true })));
    }
    if https {
        if login.key.is_some() || login.agent {
            return fail(t().https_needs_token);
        }
        let token = secret(t().token_prompt)?;
        if token.trim().is_empty() {
            return fail(t().cancelled);
        }
        return Ok(Place::Git(json!({ "url": url, "auth": { "type": "password" }, "token": token.trim(), "github": false })));
    }
    if login.token {
        return fail(t().token_needs_https);
    }
    let auth = match &login.key {
        Some(name) => {
            let keys = c.call("vault_get", json!({})).await?["keys"].clone();
            let key = keys
                .as_array()
                .into_iter()
                .flatten()
                .find(|k| s(&k["id"]) == *name || s(&k["name"]).eq_ignore_ascii_case(name))
                .ok_or_else(|| Fail(format!("{}: {name}", t().unknown_key)))?;
            json!({ "type": "key", "keyId": key["id"] })
        }
        None => json!({ "type": "agent" }),
    };
    Ok(Place::Git(json!({ "url": url, "auth": auth, "token": null, "github": false })))
}

/// Calls the core with the place's fields and a server key the person trusts, asking them
/// about a key seen for the first time.
async fn trusting(c: &Client, method: &str, mut params: Value) -> Res<Value> {
    let mut trust: Option<String> = None;
    loop {
        params["trustFingerprint"] = json!(trust);
        match c.call(method, params.clone()).await {
            Ok(v) => return Ok(v),
            Err(e) if e["kind"] == "hostKeyUnknown" && trust.is_none() => {
                eprintln!("{} {}", t().new_host_key, s(&e["fingerprint"]));
                if !confirm(t().trust_it) {
                    return fail(t().cancelled);
                }
                trust = Some(s(&e["fingerprint"]));
            }
            Err(e) => return Err(e.into()),
        }
    }
}

fn with_place(p: &Value, extra: Value) -> Value {
    let mut v = p.clone();
    v.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    v
}

/* ---------- GitHub ---------- */

/// Signs in with GitHub on this device: a code to enter on github.com, from any device.
pub async fn github_login(c: &Client) -> Res<String> {
    let code = c.call("github_start", json!({})).await?;
    eprintln!("{} {}", t().github_open, s(&code["verificationUri"]));
    eprintln!("{} {}", t().github_enter_code, s(&code["userCode"]));
    let mut interval = code["interval"].as_u64().unwrap_or(5);
    loop {
        tokio::time::sleep(Duration::from_secs(interval)).await;
        let r = c.call("github_poll", json!({ "deviceCode": code["deviceCode"] })).await?;
        match r["status"].as_str() {
            Some("done") => {
                eprintln!("{} {}", t().github_signed_in, s(&r["user"]));
                return Ok(s(&r["user"]));
            }
            Some("slowDown") => interval = r["interval"].as_u64().unwrap_or(interval + 5),
            _ => {}
        }
    }
}

/// Signed in, and the Detour Sync app reaches the repository (it is installed on it).
async fn github_ready(c: &Client, url: &str) -> Res<()> {
    if c.call("github_user", json!({})).await?.is_null() {
        github_login(c).await?;
    }
    let mut asked = false;
    for _ in 0..150 {
        if c.call("github_repo_access", json!({ "url": url })).await? == true {
            return Ok(());
        }
        if !asked {
            eprintln!("{} {INSTALL_URL}", t().github_install);
            asked = true;
        }
        tokio::time::sleep(Duration::from_secs(4)).await;
    }
    fail(t().github_no_access)
}

/* ---------- cloud drives ---------- */

/// Signs in to a drive on this device: a code to enter (OneDrive), or a page to open in a
/// browser on this device (Google Drive, Dropbox).
pub async fn drive_login(c: &Client, provider: &str) -> Res<String> {
    let start = c.call("drive_sign_in", json!({ "provider": provider })).await?;
    let mut interval = start["interval"].as_u64().unwrap_or(2);
    match start["kind"].as_str() {
        Some("device") => {
            eprintln!("{} {}", t().drive_open, s(&start["verificationUri"]));
            eprintln!("{} {}", t().github_enter_code, s(&start["userCode"]));
        }
        _ => {
            eprintln!("{} {}", t().drive_open, s(&start["url"]));
            if start["paste"] == true {
                let code = line(t().drive_paste_code)?;
                let account = s(&c.call("drive_sign_in_code", json!({ "provider": provider, "code": code.trim() })).await?);
                eprintln!("{} {account}", t().drive_signed_in);
                return Ok(account);
            }
        }
    }
    loop {
        tokio::time::sleep(Duration::from_secs(interval)).await;
        let r = c.call("drive_sign_in_poll", json!({ "provider": provider })).await?;
        match r["status"].as_str() {
            Some("done") => {
                eprintln!("{} {}", t().drive_signed_in, s(&r["account"]));
                return Ok(s(&r["account"]));
            }
            Some("slowDown") => interval += 5,
            Some("expired" | "denied") => return fail(t().cancelled),
            _ => {}
        }
    }
}

pub async fn drive(c: &Client, action: Option<crate::DriveAction>) -> Res<()> {
    let check = |p: &str| -> Res<&'static str> { detour_core::drive::Provider::parse(p).map(|p| p.id()).ok_or_else(|| Fail(format!("{}: {p}", t().drive_unknown))) };
    match action {
        None => {
            let accounts = c.call("drive_accounts", json!({})).await?;
            let list = accounts.as_array().cloned().unwrap_or_default();
            if list.is_empty() {
                eprintln!("{}", t().drive_none);
            }
            for a in list {
                let state = if a["signedIn"] == true { String::new() } else { format!(" ({})", t().drive_expired) };
                println!("{}: {}{state}", s(&a["provider"]), s(&a["account"]));
            }
        }
        Some(crate::DriveAction::Login { provider }) => {
            drive_login(c, check(&provider)?).await?;
        }
        Some(crate::DriveAction::Logout { provider, account }) => {
            c.call("drive_sign_out", json!({ "provider": check(&provider)?, "account": account })).await?;
            eprintln!("{}", t().done);
        }
    }
    Ok(())
}

pub async fn github(c: &Client, action: Option<crate::GithubAction>) -> Res<()> {
    match action {
        None => match c.call("github_user", json!({})).await? {
            Value::Null => eprintln!("{}", t().github_signed_out),
            user => println!("{} {}", t().github_signed_in, s(&user)),
        },
        Some(crate::GithubAction::Login) => {
            github_login(c).await?;
        }
        Some(crate::GithubAction::Logout) => {
            c.call("github_sign_out", json!({})).await?;
            eprintln!("{}", t().done);
        }
    }
    Ok(())
}

/* ---------- the personal vault's sync ---------- */

fn print_recovery(key: &str) {
    println!("{}\n\n    {key}\n", t().recovery_key);
    eprintln!("{}", t().recovery_keep);
}

/// Starts syncing this vault to a folder or repository that has none yet.
pub async fn sync_enable(ctx: &Ctx, arg: &str, login: &GitLogin) -> Res<()> {
    match place(&ctx.c, arg, login).await? {
        Place::Folder(path) => ctx.c.call("sync_enable_folder", json!({ "path": path })).await?,
        Place::Git(p) => trusting(&ctx.c, "sync_enable_git", p).await?,
        Place::Drive(d) => ctx.c.call("sync_enable_drive", json!({ "drive": d })).await?,
    };
    eprintln!("{}", t().sync_enabled);
    print_recovery(&s(&ctx.c.call("sync_recovery_key", json!({})).await?));
    Ok(())
}

/// Takes the vault the person syncs on their other devices, with a code from one of them.
pub async fn sync_join(ctx: &Ctx, arg: &str, code: Option<String>, login: &GitLogin) -> Res<()> {
    let place = place(&ctx.c, arg, login).await?;
    let code = match code {
        Some(c) => c,
        None => line(t().code_prompt)?,
    };
    match place {
        Place::Folder(path) => ctx.c.call("sync_join_folder", json!({ "path": path, "code": code.trim() })).await?,
        Place::Git(p) => trusting(&ctx.c, "sync_join_git", with_place(&p, json!({ "code": code.trim() }))).await?,
        Place::Drive(d) => ctx.c.call("sync_join_drive", json!({ "drive": d, "code": code.trim() })).await?,
    };
    eprintln!("{}", t().sync_joined);
    Ok(())
}

pub async fn sync_add_device(ctx: &Ctx) -> Res<()> {
    let status = ctx.c.call("sync_status", json!({})).await?;
    if status["enabled"] != true {
        return fail(t().sync_off);
    }
    let pairing = ctx.c.call("sync_add_device", json!({})).await?;
    let code = s(&pairing["code"]);
    println!("{}\n\n    {code}\n", t().device_code);
    eprintln!("{}\n    detour sync join {} --code {code}", t().device_code_use, s(&status["location"]));
    Ok(())
}

pub async fn sync_recovery_key(ctx: &Ctx) -> Res<()> {
    print_recovery(&s(&ctx.c.call("sync_recovery_key", json!({})).await?));
    Ok(())
}

pub async fn sync_disable(ctx: &Ctx) -> Res<()> {
    ctx.c.call("sync_disable", json!({})).await?;
    eprintln!("{}", t().sync_disabled);
    Ok(())
}

pub async fn sync_token(ctx: &Ctx, github: bool) -> Res<()> {
    let status = ctx.c.call("sync_status", json!({})).await?;
    let r = if github {
        github_ready(&ctx.c, &s(&status["location"])).await?;
        ctx.c.call("sync_use_github", json!({})).await?
    } else {
        let token = secret(t().token_prompt)?;
        ctx.c.call("sync_set_token", json!({ "token": token.trim() })).await?
    };
    show_sync(ctx, &r["status"])
}

/// The sync's state; syncing first with `now`, when a changed server key is asked about.
pub async fn sync(ctx: &Ctx, now: bool) -> Res<()> {
    let mut st = if now { ctx.c.call("sync_now", json!({})).await?["status"].clone() } else { ctx.c.call("sync_status", json!({})).await? };
    if now && st["error"]["kind"] == "hostKeyChanged" {
        let e = &st["error"];
        eprintln!("{} {}, {}", t().changed_host_key, s(&e["expected"]), s(&e["actual"]));
        if confirm(t().replace_it) {
            st = ctx.c.call("sync_replace_server_key", json!({ "fingerprint": e["actual"] })).await?["status"].clone();
        }
    }
    show_sync(ctx, &st)
}

fn show_sync(ctx: &Ctx, st: &Value) -> Res<()> {
    if ctx.json {
        println!("{}", serde_json::to_string_pretty(st).unwrap_or_default());
        return Ok(());
    }
    if st["enabled"] != true {
        println!("{}", t().sync_off);
        return Ok(());
    }
    match st["error"].is_null() {
        true => println!("{}: {}", t().synced, s(&st["location"])),
        false => println!("{}: {}", s(&st["location"]), error_text(&st["error"])),
    }
    Ok(())
}

/* ---------- teams ---------- */

/// A new team in a folder or an empty repository; its recovery key is shown once.
pub async fn team_create(ctx: &Ctx, arg: &str, name: &str, me: &str, login: &GitLogin) -> Res<()> {
    let recovery = s(&ctx.c.call("recovery_key_new", json!({})).await?);
    let common = json!({ "name": name, "myName": me, "recoveryKey": recovery });
    match place(&ctx.c, arg, login).await? {
        Place::Folder(path) => ctx.c.call("team_create", with_place(&json!({ "path": path }), common)).await?,
        Place::Git(p) => trusting(&ctx.c, "team_create_git", with_place(&p, common)).await?,
        Place::Drive(d) => ctx.c.call("team_create_drive", with_place(&json!({ "drive": d }), common)).await?,
    };
    eprintln!("{} {name}", t().team_created);
    println!("{}\n\n    {recovery}\n", t().team_recovery_key);
    eprintln!("{}", t().team_recovery_keep);
    Ok(())
}

/// Joins a team after the person compared its creator's fingerprint with the one they were given.
pub async fn team_join(ctx: &Ctx, arg: &str, login: &GitLogin) -> Res<()> {
    let place = place(&ctx.c, arg, login).await?;
    let (info, clone) = match &place {
        Place::Folder(path) => (ctx.c.call("team_inspect", json!({ "path": path })).await?, Value::Null),
        Place::Git(p) => {
            let r = trusting(&ctx.c, "team_inspect_git", p.clone()).await?;
            (r["info"].clone(), r["clone"].clone())
        }
        Place::Drive(d) => {
            let r = ctx.c.call("team_inspect_drive", json!({ "drive": d })).await?;
            (r["info"].clone(), r["clone"].clone())
        }
    };
    eprintln!("{}\n    {}", t().team_creator, s(&info["rootFingerprint"]));
    if !confirm(t().team_join_question) {
        return fail(t().cancelled);
    }
    let view = match &place {
        Place::Folder(path) => ctx.c.call("team_join", json!({ "path": path, "root": info["root"] })).await?,
        Place::Git(p) => ctx.c.call("team_join_git", with_place(p, json!({ "clone": clone, "root": info["root"] }))).await?,
        Place::Drive(d) => ctx.c.call("team_join_drive", json!({ "drive": d, "clone": clone, "root": info["root"] })).await?,
    };
    joined(&view);
    Ok(())
}

/// Takes a team back with its recovery key, as its owner, when nobody who runs it is left.
pub async fn team_recover(ctx: &Ctx, arg: &str, me: &str, login: &GitLogin) -> Res<()> {
    let place = place(&ctx.c, arg, login).await?;
    let key = secret(t().team_recovery_prompt)?;
    let common = json!({ "recoveryKey": key.trim(), "myName": me });
    let view = match place {
        Place::Folder(path) => ctx.c.call("team_recover", with_place(&json!({ "path": path }), common)).await?,
        Place::Git(p) => trusting(&ctx.c, "team_recover_git", with_place(&p, common)).await?,
        Place::Drive(d) => ctx.c.call("team_recover_drive", with_place(&json!({ "drive": d }), common)).await?,
    };
    joined(&view);
    Ok(())
}

fn joined(view: &Value) {
    if view["joined"] == true {
        eprintln!("{} {}", t().team_joined, s(&view["name"]));
    } else {
        eprintln!("{}", t().team_waiting);
    }
}

/// Reads a team again now; a changed server key is asked about.
pub async fn team_sync(ctx: &Ctx) -> Res<()> {
    match ctx.c.call("team_sync", json!({ "id": ctx.vault })).await {
        Ok(_) => {}
        Err(e) if e["kind"] == "hostKeyChanged" => {
            eprintln!("{} {}, {}", t().changed_host_key, s(&e["expected"]), s(&e["actual"]));
            if !confirm(t().replace_it) {
                return fail(t().cancelled);
            }
            ctx.c.call("team_replace_server_key", json!({ "id": ctx.vault, "fingerprint": e["actual"] })).await?;
        }
        Err(e) => return Err(e.into()),
    }
    eprintln!("{}", t().done);
    Ok(())
}

/// A new token for a team's repository, or GitHub's sign-in instead.
pub async fn team_token(ctx: &Ctx, github: bool) -> Res<()> {
    if github {
        let teams = ctx.c.call("teams_list", json!({})).await?;
        let url = teams.as_array().into_iter().flatten().find(|e| e["id"] == ctx.vault.as_str()).map(|e| s(&e["view"]["remote"]["url"])).unwrap_or_default();
        github_ready(&ctx.c, &url).await?;
        ctx.c.call("team_use_github", json!({ "id": ctx.vault })).await?;
    } else {
        let token = secret(t().token_prompt)?;
        ctx.c.call("team_set_git_token", json!({ "id": ctx.vault, "token": token.trim() })).await?;
    }
    eprintln!("{}", t().done);
    Ok(())
}

/// Teams joined on the person's other devices and not here yet: a folder one needs its folder;
/// one in Git joins by itself, or says why it couldn't.
pub async fn pending_teams(c: &Client) -> Res<()> {
    let pending = c.call("teams_pending", json!({})).await?;
    let all = pending.as_array().cloned().unwrap_or_default();
    let folders: Vec<String> = all.iter().filter(|p| p["url"].is_null() && p["drive"].is_null()).map(|p| s(&p["name"])).collect();
    if !folders.is_empty() {
        eprintln!("\n{} {}", t().teams_elsewhere, folders.join(", "));
        eprintln!("    detour teams join <folder>");
    }
    for p in all.iter().filter(|p| !p["url"].is_null() || !p["drive"].is_null()) {
        match p["error"].is_null() {
            true => eprintln!("\n{} {}", t().team_joining, s(&p["name"])),
            false => eprintln!("\n{} {}: {}", t().team_cant_join, s(&p["name"]), error_text(&p["error"])),
        }
        // A team in a drive joins once this device is signed in to it.
        if !p["drive"].is_null() {
            eprintln!("    detour drive login {}", s(&p["drive"]["provider"]));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::is_git;

    #[test]
    fn drive_places() {
        use detour_core::drive::Provider;
        assert_eq!(super::drive_place("onedrive:Detour/Team"), Some((Provider::OneDrive, Some("Detour/Team"))));
        assert_eq!(super::drive_place("dropbox:/Team"), Some((Provider::Dropbox, Some("/Team"))));
        assert_eq!(super::drive_place("https://1drv.ms/f/s!abc"), Some((Provider::OneDrive, None)));
        assert_eq!(super::drive_place(r"C:\Sync\team"), None);
        assert_eq!(super::drive_place("git@github.com:me/vault.git"), None);
    }

    #[test]
    fn repositories_and_folders() {
        for url in ["git@github.com:me/vault.git", "ssh://git@host:2222/v.git", "https://github.com/me/vault.git", "deploy@nas:vaults/team"] {
            assert!(is_git(url), "{url}");
        }
        for path in [r"C:\Sync\team", "/srv/sync/team", "./team", "D:team"] {
            assert!(!is_git(path), "{path}");
        }
    }
}
