//! `detour`: Detour on the command line. It talks to the Detour core, starting it
//! when none runs, and asks for the master password only in a terminal not trusted yet.
//! `detour daemon` is the core itself.

mod agents;
mod cmds;
mod conn;
mod help;
mod i18n;
mod mcp;
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
    Keys(Keys),
    #[command(subcommand)]
    Snippets(Snippets),
    Sync {
        #[arg(long)]
        now: bool,
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
    Import {
        aliases: Vec<String>,
        #[arg(long)]
        known_hosts: bool,
    },
    Export {
        hosts: Vec<String>,
        #[arg(long, short)]
        output: Option<PathBuf>,
        #[arg(long, value_name = "DIR")]
        keys: Option<PathBuf>,
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
enum Teams {
    Ls,
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
    if let Command::Daemon { app } = cli.command {
        return match detour_daemon::run(&dir, app.or_else(app_beside)).await {
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
        Command::Quit => {
            let c = conn::ready(dir).await?;
            let _ = c.call("quit", serde_json::json!({})).await;
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
        Command::Hosts(Hosts::Import { aliases, known_hosts }) => cmds::host_import(&ctx, aliases, known_hosts).await?,
        Command::Hosts(Hosts::Export { hosts, output, keys }) => cmds::host_export(&ctx, hosts, output, keys).await?,
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
        Command::Keys(Keys::Ls) => cmds::keys(&ctx).await?,
        Command::Keys(Keys::Gen { name, r#type, passphrase }) => cmds::key_gen(&ctx, &name, &r#type, passphrase).await?,
        Command::Keys(Keys::Import { file, name }) => cmds::key_import(&ctx, &file, name).await?,
        Command::Keys(Keys::Export { key, output }) => cmds::key_export(&ctx, &key, output).await?,
        Command::Keys(Keys::Rm { key }) => cmds::key_rm(&ctx, &key).await?,
        Command::Snippets(Snippets::Ls) => cmds::snippets(&ctx).await?,
        Command::Snippets(Snippets::Add { name, command }) => cmds::snippet_add(&ctx, &name, &command.join(" ")).await?,
        Command::Snippets(Snippets::Rm { name }) => cmds::snippet_rm(&ctx, &name).await?,
        Command::Snippets(Snippets::Run { name, host }) => return cmds::snippet_run(&ctx, &name, &host).await,
        Command::Sync { now } => cmds::sync(&ctx, now).await?,
        Command::Teams(Teams::Ls) => cmds::teams(&ctx).await?,
        Command::Teams(rest) => {
            if ctx.vault == cmds::PERSONAL {
                return Err(Fail("--team <TEAM>".into()));
            }
            match rest {
                Teams::Sync => cmds::team_sync(&ctx).await?,
                Teams::Members => cmds::members(&ctx).await?,
                Teams::Add { detour_id, name, role, access } => cmds::member_add(&ctx, &detour_id, &name, &role, &access).await?,
                Teams::Remove { member } => cmds::member_rm(&ctx, &member).await?,
                Teams::Role { member, role } => cmds::member_role(&ctx, &member, &role).await?,
                Teams::Access { record, member, level } => cmds::access(&ctx, &record, &member, &level).await?,
                Teams::Leave => cmds::team_leave(&ctx).await?,
                Teams::Ls => unreachable!(),
            }
        }
        Command::Id => cmds::my_id(&ctx.c, json).await?,
        Command::Approve => agents::approve(&ctx).await?,
        Command::Agents(Agents::Ls) => agents::list(&ctx).await?,
        Command::Agents(Agents::Revoke { agent }) => agents::revoke(&ctx, &agent).await?,
        Command::Status | Command::Quit | Command::Daemon { .. } | Command::Mcp { .. } => unreachable!(),
    }
    Ok(0)
}
