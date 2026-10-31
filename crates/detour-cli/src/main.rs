//! `detour`: Detour on the command line. It talks to the Detour core, starting it
//! when none runs, and asks for the master password only in a terminal not trusted yet.
//! `detour daemon` is the core itself.

mod cmds;
mod conn;
mod i18n;
mod term;

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use cmds::{Ctx, FileOp, HostOpts};
use conn::{Fail, Res};

#[derive(Parser)]
#[command(name = "detour", version, about = "Detour SSH client", disable_help_subcommand = true)]
struct Cli {
    /// Where the vault lives (default: the app's data folder, or $DETOUR_DATA_DIR)
    #[arg(long, global = true, value_name = "DIR")]
    data_dir: Option<PathBuf>,
    /// A team instead of the personal vault (its name or id)
    #[arg(long, short = 'T', global = true, value_name = "TEAM")]
    team: Option<String>,
    /// Print JSON instead of tables
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// The core: running, locked or not
    Status,
    /// Unlock the vault (and trust this terminal)
    Unlock,
    /// Lock the vault
    Lock,
    /// Stop the core (sessions and tunnels end)
    Quit,
    /// List hosts
    Ls { query: Option<String> },
    /// Add, edit, remove, import and export hosts
    #[command(subcommand)]
    Hosts(Hosts),
    /// Open a terminal session on a host
    Ssh { host: String },
    /// Run a command on a host; the exit code is the command's
    Exec {
        host: String,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
        command: Vec<String>,
    },
    /// Copy a file or folder: `detour cp file host:path` or `detour cp host:path file`
    Cp { from: String, to: String },
    /// Files on a host
    #[command(subcommand)]
    Files(Files),
    /// Saved tunnels
    #[command(subcommand)]
    Tunnels(Tunnels),
    /// Keys
    #[command(subcommand)]
    Keys(Keys),
    /// Snippets
    #[command(subcommand)]
    Snippets(Snippets),
    /// The personal vault's sync with your other devices
    Sync {
        /// Sync now instead of showing the state
        #[arg(long)]
        now: bool,
    },
    /// Teams
    #[command(subcommand)]
    Teams(Teams),
    /// Your Detour ID, to be added to a team
    Id,
    /// Run the core in this process (the app and the CLI start it by themselves)
    #[command(hide = true)]
    Daemon {
        /// The window's program, which the core trusts
        #[arg(long)]
        app: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum Hosts {
    /// List hosts
    Ls { query: Option<String> },
    /// Add a host
    Add {
        name: String,
        #[command(flatten)]
        opts: HostOpts,
    },
    /// Change a host
    Edit {
        host: String,
        /// A new name
        #[arg(long)]
        name: Option<String>,
        #[command(flatten)]
        opts: HostOpts,
    },
    /// Remove a host
    Rm { host: String },
    /// Import hosts from ~/.ssh/config (all new ones, or those named)
    Import {
        aliases: Vec<String>,
        /// Also pin server keys from ~/.ssh/known_hosts
        #[arg(long)]
        known_hosts: bool,
    },
    /// Hosts as ~/.ssh/config entries
    Export {
        hosts: Vec<String>,
        /// Write to this file instead of printing
        #[arg(long, short)]
        output: Option<PathBuf>,
        /// Also save their keys as files in this folder
        #[arg(long, value_name = "DIR")]
        keys: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum Files {
    /// List a folder: host:path
    Ls { target: String },
    Mkdir { target: String },
    /// Remove a file or a folder with everything in it
    Rm { target: String },
    /// Rename or move: host:path new-path
    Mv { target: String, to: String },
    /// Change permissions: host:path 755
    Chmod { target: String, mode: String },
}

#[derive(Subcommand)]
enum Tunnels {
    Ls,
    /// Start a saved tunnel
    Up { name: String },
    /// Stop a tunnel
    Down { name: String },
}

#[derive(Subcommand)]
enum Keys {
    Ls,
    /// Create a key; prints its public part
    Gen {
        name: String,
        #[arg(long, default_value = "ed25519", value_parser = ["ed25519", "rsa", "ecdsa"])]
        r#type: String,
        /// Protect it with a passphrase (asked)
        #[arg(long)]
        passphrase: bool,
    },
    /// Import a private key file
    Import {
        file: PathBuf,
        #[arg(long)]
        name: Option<String>,
    },
    /// Print a private key, or save it to a file or folder
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
    /// Save a command: detour snippets add disk -- df -h
    Add {
        name: String,
        #[arg(trailing_var_arg = true, allow_hyphen_values = true, required = true)]
        command: Vec<String>,
    },
    Rm { name: String },
    /// Run a snippet on a host
    Run { name: String, host: String },
}

#[derive(Subcommand)]
enum Teams {
    Ls,
    /// Read a team again now (with --team)
    Sync,
    /// Members of a team (with --team)
    Members,
    /// Add someone by their Detour ID (with --team)
    Add {
        detour_id: String,
        name: String,
        #[arg(long, default_value = "member", value_parser = ["member", "admin", "owner"])]
        role: String,
        #[arg(long, default_value = "view", value_parser = ["none", "view", "secret", "edit"])]
        access: String,
    },
    /// Remove a member (with --team)
    Remove { member: String },
    /// Change a member's role (with --team)
    Role {
        member: String,
        #[arg(value_parser = ["member", "admin", "owner"])]
        role: String,
    },
    /// What a member may do with a host, key, snippet or tunnel (with --team)
    Access {
        record: String,
        member: String,
        #[arg(value_parser = ["none", "view", "secret", "edit"])]
        level: String,
    },
    /// Leave a team on this device (with --team)
    Leave,
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
    let cli = Cli::parse();
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
        Command::Status | Command::Quit | Command::Daemon { .. } => unreachable!(),
    }
    Ok(0)
}
