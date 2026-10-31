//! Reaching the core: connect to the one that runs, or start it first.

use std::io;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::{Client, socket_name};

/// Starts `<detour> daemon --data-dir <dir>` on its own: no console window, not
/// stopped by Ctrl+C in the terminal that started it, outliving its starter.
pub fn start_core(detour: &Path, data_dir: &Path) -> io::Result<()> {
    let mut cmd = Command::new(detour);
    cmd.arg("daemon").arg("--data-dir").arg(data_dir).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    cmd.spawn().map(|_| ())
}

/// The core of a vault folder, started with `detour` if it doesn't run yet.
pub async fn connect_or_start(detour: &Path, data_dir: &Path) -> io::Result<Client> {
    let socket = socket_name(data_dir);
    if let Ok(c) = Client::connect(&socket).await {
        return Ok(c);
    }
    start_core(detour, data_dir)?;
    let mut last = io::Error::new(io::ErrorKind::TimedOut, "the Detour core did not start");
    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        match Client::connect(&socket).await {
            Ok(c) => return Ok(c),
            Err(e) => last = e,
        }
    }
    Err(last)
}
