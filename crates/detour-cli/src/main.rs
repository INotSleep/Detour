//! `detour`: the command line, the core (`detour daemon`) and, later, the MCP server.

use std::path::PathBuf;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "detour", version, about = "Detour SSH client")]
struct Cli {
    /// Where the vault lives (default: the app's data folder, or $DETOUR_DATA_DIR)
    #[arg(long, global = true, value_name = "DIR")]
    data_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Run the core in this process (the app and the CLI start it by themselves)
    Daemon,
}

fn data_dir(given: Option<PathBuf>) -> PathBuf {
    given.or_else(|| std::env::var_os("DETOUR_DATA_DIR").map(Into::into)).unwrap_or_else(detour_core::store::Store::default_dir)
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    let dir = data_dir(cli.data_dir);
    match cli.command {
        Command::Daemon => match detour_daemon::run(&dir).await {
            Ok(()) => std::process::ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("detour daemon: {e}");
                std::process::ExitCode::FAILURE
            }
        },
    }
}
