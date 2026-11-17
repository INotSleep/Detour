//! `detour`: Detour on the command line. It talks to the Detour core, starting it
//! when none runs, and asks for the master password only in a terminal not trusted yet.
//! `detour daemon` is the core itself.

mod agents;
mod cmds;
mod conn;
mod help;
mod i18n;
mod mcp;
mod setup;
mod term;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use cmds::{Ctx, FileOp, HostOpts};
use conn::{Fail, Res};

#[derive(Parser)]
// What each command and option means is in i18n (`help` there), in both languages.
#[command(name = "detour", version, disable_help_subcommand = true)]
struct Cli {
    #[arg(long, global = true, value_name = "DIR")]
    data_dir: Option<PathBuf>,
    #[arg(long, short = 'T', global = true, value_name = "TEAM")]
    team: Option<String>,
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    Status,
    Unlock,
    Lock,
    Quit,
    Logs {
        #[arg(long, short = 'n', default_value_t = 50)]
        lines: usize,
        #[arg(long)]
        path: bool,
    },
    Ls { query: Option<String> },
    #[command(subcommand)]
    Hosts(Hosts),
    Ssh { host: String },
    Exec {
        host: String,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
        command: Vec<String>,
    },
    Cp { from: String, to: String },
    #[command(subcommand)]
    Files(Files),
    #[command(subcommand)]
    Tunnels(Tunnels),
    #[command(subcommand)]
    Proxies(Proxies),
    #[command(subcommand)]
    Groups(Groups),
    #[command(subcommand)]
    Keys(Keys),
    #[command(subcommand)]
    Snippets(Snippets),
    Sync {
        #[arg(long)]
        now: bool,
        #[command(subcommand)]
        action: Option<SyncAction>,
    },
    Github {
        #[command(subcommand)]
        action: Option<GithubAction>,
    },
    Drive {
        #[command(subcommand)]
        action: Option<DriveAction>,
    },
    #[command(subcommand)]
    Teams(Teams),
    Id,
    Approve,
    #[command(subcommand)]
    Agents(Agents),
    Mcp {
        #[command(subcommand)]
        action: Option<McpAction>,
    },
    // The core in this process (the app and the CLI start it by themselves); `--app`
    // is the window's program, which the core trusts.
    #[command(hide = true)]
    Daemon {
        #[arg(long)]
        app: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum Hosts {
    Ls { query: Option<String> },
    Add {
        name: String,
        #[command(flatten)]
        opts: HostOpts,
    },
    Edit {
        host: String,
        #[arg(long)]
        name: Option<String>,
        #[command(flatten)]
        opts: HostOpts,
    },
    Rm { host: String },
    Move {
        host: String,
        #[arg(long)]
        group: Option<String>,
        #[arg(long, value_name = "HOST", conflicts_with = "last")]
        before: Option<String>,
        #[arg(long)]
        last: bool,
        #[arg(long)]
        mine: bool,
    },
    Import {
        aliases: Vec<String>,
        #[arg(long)]
        known_hosts: bool,
        #[arg(long, value_name = "FILE", conflicts_with = "known_hosts")]
        file: Option<PathBuf>,
    },
    Export {
        hosts: Vec<String>,
        #[arg(long, short, conflicts_with = "file")]
        output: Option<PathBuf>,
        #[arg(long, value_name = "DIR", conflicts_with = "file")]
        keys: Option<PathBuf>,
        #[arg(long, value_name = "FILE")]
        file: Option<PathBuf>,
        #[command(flatten)]
        leave_out: cmds::LeaveOut,
    },
}

#[derive(Subcommand)]
enum Files {
    Ls { target: String },
    Mkdir { target: String },
    Rm { target: String },
    Mv { target: String, to: String },
    Chmod { target: String, mode: String },
}

#[derive(Subcommand)]
enum Tunnels {
    Ls,
    Up { name: String },
    Down { name: String },
}

#[derive(Subcommand)]
enum Groups {
    Ls {
        #[arg(long)]
        mine: bool,
    },
    Add {
        name: String,
        #[arg(long)]
        mine: bool,
    },
    Rename {
        group: String,
        name: String,
        #[arg(long)]
        mine: bool,
    },
    Rm {
        group: String,
        #[arg(long)]
        mine: bool,
    },
    Move {
        group: String,
        #[arg(long, value_name = "GROUP", conflicts_with = "last")]
        before: Option<String>,
        #[arg(long)]
        last: bool,
        #[arg(long)]
        mine: bool,
    },
    Set {
        group: String,
        #[command(flatten)]
        opts: cmds::GroupOpts,
    },
}

#[derive(Subcommand)]
enum Proxies {
    Ls,
    Add {
        name: String,
        #[arg(long, value_name = "URL", conflicts_with_all = ["wireguard", "socks5", "http"])]
        link: Option<String>,
        #[arg(long, value_name = "FILE", conflicts_with_all = ["socks5", "http"])]
        wireguard: Option<PathBuf>,
        #[arg(long, value_name = "HOST:PORT", conflicts_with = "http")]
        socks5: Option<String>,
        #[arg(long, value_name = "HOST:PORT")]
        http: Option<String>,
        #[arg(long)]
        user: Option<String>,
    },
    Rm { name: String },
    Check { name: String },
    Export { name: String },
}

#[derive(Subcommand)]
enum Keys {
    Ls,
    Gen {
        name: String,
        #[arg(long, default_value = "ed25519", value_parser = ["ed25519", "rsa", "ecdsa"])]
        r#type: String,
        #[arg(long)]
        passphrase: bool,
    },
    Import {
        file: PathBuf,
        #[arg(long)]
        name: Option<String>,
    },
    Export {
        key: String,
        #[arg(long, short)]
        output: Option<PathBuf>,
    },
    Rm { key: String },
}

#[derive(Subcommand)]
enum Snippets {
    Ls,
    Add {
        name: String,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
        command: Vec<String>,
    },
    Rm { name: String },
    Run { name: String, host: String },
}

#[derive(Subcommand)]
enum SyncAction {
    Enable {
        place: String,
        #[command(flatten)]
        login: setup::GitLogin,
    },
    Join {
        place: String,
        #[arg(long)]
        code: Option<String>,
        #[command(flatten)]
        login: setup::GitLogin,
    },
    AddDevice,
    RecoveryKey,
    Token {
        #[arg(long)]
        github: bool,
    },
    Disable,
}

#[derive(Subcommand)]
pub enum GithubAction {
    Login,
    Logout,
}

#[derive(Subcommand)]
pub enum DriveAction {
    Login { provider: String },
    Logout { provider: String },
}

#[derive(Subcommand)]
enum Teams {
    Ls,
    Create {
        place: String,
        #[arg(long)]
        name: String,
        #[arg(long)]
        me: String,
        #[command(flatten)]
        login: setup::GitLogin,
    },
    Join {
        place: String,
        #[command(flatten)]
        login: setup::GitLogin,
    },
    Recover {
        place: String,
        #[arg(long)]
        me: String,
        #[command(flatten)]
        login: setup::GitLogin,
    },
    Token {
        #[arg(long)]
        github: bool,
    },
    Sync,
    Members,
    Add {
        detour_id: String,
        name: String,
        #[arg(long, default_value = "member", value_parser = ["member", "admin", "owner"])]
        role: String,
        #[arg(long, default_value = "view", value_parser = ["none", "view", "secret", "edit"])]
        access: String,
    },
    Remove { member: String },
    Role {
        member: String,
        #[arg(value_parser = ["member", "admin", "owner"])]
        role: String,
    },
    Access {
        record: String,
        member: String,
        #[arg(value_parser = ["none", "view", "secret", "edit"])]
        level: String,
    },
    Leave,
}

#[derive(Subcommand)]
pub enum McpAction {
    Clients,
    Add { client: Option<String> },
    Remove { client: String },
}

#[derive(Subcommand)]
enum Agents {
    Ls,
    Add {
        agent: String,
        #[command(flatten)]
        opts: agents::RuleOpts,
        #[arg(long)]
        client: Option<String>,
    },
    Edit {
        agent: String,
        #[command(flatten)]
        opts: agents::RuleOpts,
    },
    Revoke { agent: String },
}

fn data_dir(given: Option<PathBuf>) -> PathBuf {
    given.or_else(|| std::env::var_os("DETOUR_DATA_DIR").map(Into::into)).unwrap_or_else(detour_core::store::Store::default_dir)
}

/// The window's program next to this one, as installers put them.
fn app_beside() -> Option<PathBuf> {
    let me = std::env::current_exe().ok()?;
    let app = me.with_file_name(format!("detour-desktop{}", std::env::consts::EXE_SUFFIX));
    app.exists().then_some(app)
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = help::parse();
    let dir = data_dir(cli.data_dir);
    detour_core::logfile::init(&dir, "cli");
    if let Command::Daemon { app } = cli.command {
        return match detour_daemon::run_with_tray(&dir, app.or_else(app_beside)).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("detour daemon: {e}");
                ExitCode::FAILURE
            }
        };
    }
    if let Command::Mcp { action } = cli.command {
        if let Some(action) = action {
            return match mcp::setup(action, &dir, cli.json) {
                Ok(()) => ExitCode::SUCCESS,
                Err(Fail(msg)) => {
                    eprintln!("detour: {msg}");
                    ExitCode::FAILURE
                }
            };
        }
        return match mcp::serve(&dir).await {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("detour mcp: {e}");
                ExitCode::FAILURE
            }
        };
    }
    match run(cli.command, &dir, cli.team, cli.json).await {
        Ok(code) => ExitCode::from(code.clamp(0, 255) as u8),
        Err(Fail(msg)) => {
            eprintln!("detour: {msg}");
            ExitCode::FAILURE
        }
    }
}

async fn run(command: Command, dir: &std::path::Path, team: Option<String>, json: bool) -> Res<i32> {
    // What doesn't need the vault.
    match command {
        Command::Status => {
            let c = conn::connect(dir).await?;
            cmds::status(&c, json).await?;
            return Ok(0);
        }
        // Read from the file: a core that won't start can still be traced.
        Command::Logs { lines, path } => {
            let file = detour_core::logfile::path(dir);
            if path {
                println!("{}", file.display());
                return Ok(0);
            }
            let text = detour_core::logfile::tail(dir, 4 << 20);
            if text.is_empty() {
                eprintln!("{}: {}", i18n::t().log_empty, file.display());
                return Ok(0);
            }
            let all: Vec<&str> = text.lines().collect();
            for l in &all[all.len().saturating_sub(lines)..] {
                println!("{l}");
            }
            eprintln!("
{}", file.display());
            return Ok(0);
        }
        Command::Quit => {
            // An idle core stops for anyone; one that holds something, for the person.
            let c = conn::connect(dir).await?;
            if c.call("quit", serde_json::json!({})).await.is_err() {
                let _ = conn::ready(dir).await?.call("quit", serde_json::json!({})).await;
            }
            eprintln!("{}", i18n::t().stopped);
            return Ok(0);
        }
        _ => {}
    }
    let c = conn::ready(dir).await?;
    let vault = cmds::vault_id(&c, team.as_deref()).await?;
    let ctx = Ctx { c, vault, json };
    match command {
        Command::Unlock => eprintln!("{}", i18n::t().unlocked),
        Command::Lock => {
            ctx.c.call("vault_lock", serde_json::json!({})).await?;
            eprintln!("{}", i18n::t().locked);
        }
        Command::Ls { query } | Command::Hosts(Hosts::Ls { query }) => cmds::ls(&ctx, query.as_deref()).await?,
        Command::Hosts(Hosts::Add { name, opts }) => cmds::host_add(&ctx, &name, opts).await?,
        Command::Hosts(Hosts::Edit { host, name, opts }) => cmds::host_edit(&ctx, &host, name, opts).await?,
        Command::Hosts(Hosts::Rm { host }) => cmds::host_rm(&ctx, &host).await?,
        Command::Hosts(Hosts::Move { host, group, before, last: _, mine: true }) => cmds::mine_host_move(&ctx, &host, group.as_deref(), before.as_deref()).await?,
        Command::Hosts(Hosts::Move { host, group, before, last: _, mine: false }) => cmds::host_move(&ctx, &host, group.as_deref(), before.as_deref()).await?,
        Command::Groups(Groups::Ls { mine }) => cmds::groups(&ctx, mine).await?,
        Command::Groups(Groups::Add { name, mine: true }) => cmds::mine_group_add(&ctx, &name).await?,
        Command::Groups(Groups::Add { name, mine: false }) => cmds::group_add(&ctx, &name).await?,
        Command::Groups(Groups::Rename { group, name, mine: true }) => cmds::mine_group_rename(&ctx, &group, &name).await?,
        Command::Groups(Groups::Rename { group, name, mine: false }) => cmds::group_rename(&ctx, &group, &name).await?,
        Command::Groups(Groups::Rm { group, mine: true }) => cmds::mine_group_rm(&ctx, &group).await?,
        Command::Groups(Groups::Rm { group, mine: false }) => cmds::group_rm(&ctx, &group).await?,
        Command::Groups(Groups::Move { group, before, last: _, mine: true }) => cmds::mine_group_move(&ctx, &group, before.as_deref()).await?,
        Command::Groups(Groups::Move { group, before, last: _, mine: false }) => cmds::group_move(&ctx, &group, before.as_deref()).await?,
        Command::Groups(Groups::Set { group, opts }) => cmds::group_set(&ctx, &group, opts).await?,
        Command::Hosts(Hosts::Import { aliases, file: Some(file), .. }) => cmds::host_import_file(&ctx, &file, aliases).await?,
        Command::Hosts(Hosts::Import { aliases, known_hosts, file: None }) => cmds::host_import(&ctx, aliases, known_hosts).await?,
        Command::Hosts(Hosts::Export { hosts, file: Some(file), leave_out, .. }) => cmds::host_export_file(&ctx, hosts, &file, leave_out).await?,
        Command::Hosts(Hosts::Export { hosts, output, keys, .. }) => cmds::host_export(&ctx, hosts, output, keys).await?,
        Command::Ssh { host } => return term::ssh(&ctx, &host).await,
        Command::Exec { host, command } => return cmds::exec(&ctx, &host, &command.join(" ")).await,
        Command::Cp { from, to } => cmds::cp(&ctx, &from, &to).await?,
        Command::Files(f) => {
            let (target, op) = match f {
                Files::Ls { target } => (target, FileOp::Ls),
                Files::Mkdir { target } => (target, FileOp::Mkdir),
                Files::Rm { target } => (target, FileOp::Rm),
                Files::Mv { target, to } => (target, FileOp::Mv(to)),
                Files::Chmod { target, mode } => (target.clone(), FileOp::Chmod(u32::from_str_radix(&mode, 8).map_err(|_| Fail(format!("{mode}: 755, 644...")))?)),
            };
            cmds::files(&ctx, &target, op).await?;
        }
        Command::Tunnels(Tunnels::Ls) => cmds::tunnels(&ctx).await?,
        Command::Tunnels(Tunnels::Up { name }) => cmds::tunnel_up(&ctx, &name).await?,
        Command::Tunnels(Tunnels::Down { name }) => cmds::tunnel_down(&ctx, &name).await?,
        Command::Proxies(Proxies::Ls) => cmds::proxies(&ctx).await?,
        Command::Proxies(Proxies::Add { name, link, wireguard, socks5, http, user }) => {
            let what = match (link, wireguard, socks5, http) {
                (Some(l), _, _, _) => cmds::NewProxy::Text(l),
                (_, Some(f), _, _) => cmds::NewProxy::File(f),
                (_, _, Some(a), _) => cmds::NewProxy::Plain("socks5", a, user),
                (_, _, _, Some(a)) => cmds::NewProxy::Plain("http", a, user),
                _ => return Err(Fail("--link, --wireguard, --socks5 or --http".into())),
            };
            cmds::proxy_add(&ctx, &name, what).await?
        }
        Command::Proxies(Proxies::Rm { name }) => cmds::proxy_rm(&ctx, &name).await?,
        Command::Proxies(Proxies::Check { name }) => cmds::proxy_check(&ctx, &name).await?,
        Command::Proxies(Proxies::Export { name }) => cmds::proxy_export(&ctx, &name).await?,
        Command::Keys(Keys::Ls) => cmds::keys(&ctx).await?,
        Command::Keys(Keys::Gen { name, r#type, passphrase }) => cmds::key_gen(&ctx, &name, &r#type, passphrase).await?,
        Command::Keys(Keys::Import { file, name }) => cmds::key_import(&ctx, &file, name).await?,
        Command::Keys(Keys::Export { key, output }) => cmds::key_export(&ctx, &key, output).await?,
        Command::Keys(Keys::Rm { key }) => cmds::key_rm(&ctx, &key).await?,
        Command::Snippets(Snippets::Ls) => cmds::snippets(&ctx).await?,
        Command::Snippets(Snippets::Add { name, command }) => cmds::snippet_add(&ctx, &name, &command.join(" ")).await?,
        Command::Snippets(Snippets::Rm { name }) => cmds::snippet_rm(&ctx, &name).await?,
        Command::Snippets(Snippets::Run { name, host }) => return cmds::snippet_run(&ctx, &name, &host).await,
        Command::Sync { now, action: None } => setup::sync(&ctx, now).await?,
        Command::Sync { action: Some(a), .. } => match a {
            SyncAction::Enable { place, login } => setup::sync_enable(&ctx, &place, &login).await?,
            SyncAction::Join { place, code, login } => setup::sync_join(&ctx, &place, code, &login).await?,
            SyncAction::AddDevice => setup::sync_add_device(&ctx).await?,
            SyncAction::RecoveryKey => setup::sync_recovery_key(&ctx).await?,
            SyncAction::Token { github } => setup::sync_token(&ctx, github).await?,
            SyncAction::Disable => setup::sync_disable(&ctx).await?,
        },
        Command::Github { action } => setup::github(&ctx.c, action).await?,
        Command::Drive { action } => setup::drive(&ctx.c, action).await?,
        Command::Teams(Teams::Ls) => {
            cmds::teams(&ctx).await?;
            if !json {
                setup::pending_teams(&ctx.c).await?;
            }
        }
        Command::Teams(Teams::Create { place, name, me, login }) => setup::team_create(&ctx, &place, &name, &me, &login).await?,
        Command::Teams(Teams::Join { place, login }) => setup::team_join(&ctx, &place, &login).await?,
        Command::Teams(Teams::Recover { place, me, login }) => setup::team_recover(&ctx, &place, &me, &login).await?,
        Command::Teams(rest) => {
            if ctx.vault == cmds::PERSONAL {
                return Err(Fail("--team <TEAM>".into()));
            }
            match rest {
                Teams::Sync => setup::team_sync(&ctx).await?,
                Teams::Token { github } => setup::team_token(&ctx, github).await?,
                Teams::Members => cmds::members(&ctx).await?,
                Teams::Add { detour_id, name, role, access } => cmds::member_add(&ctx, &detour_id, &name, &role, &access).await?,
                Teams::Remove { member } => cmds::member_rm(&ctx, &member).await?,
                Teams::Role { member, role } => cmds::member_role(&ctx, &member, &role).await?,
                Teams::Access { record, member, level } => cmds::access(&ctx, &record, &member, &level).await?,
                Teams::Leave => cmds::team_leave(&ctx).await?,
                Teams::Ls | Teams::Create { .. } | Teams::Join { .. } | Teams::Recover { .. } => unreachable!(),
            }
        }
        Command::Id => cmds::my_id(&ctx.c, json).await?,
        Command::Approve => agents::approve(&ctx).await?,
        Command::Agents(Agents::Ls) => agents::list(&ctx).await?,
        Command::Agents(Agents::Revoke { agent }) => agents::revoke(&ctx, &agent).await?,
        Command::Agents(Agents::Add { agent, opts, client }) => agents::add(&ctx, dir, &agent, opts, client.as_deref()).await?,
        Command::Agents(Agents::Edit { agent, opts }) => agents::edit(&ctx, &agent, opts).await?,
        Command::Status | Command::Quit | Command::Logs { .. } | Command::Daemon { .. } | Command::Mcp { .. } => unreachable!(),
    }
    Ok(0)
}
