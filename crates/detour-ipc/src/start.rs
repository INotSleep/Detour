//! Reaching the core: connect to the one that runs, or start it first.

use std::ffi::OsString;
use std::io;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::{Client, socket_name};

/// Starts a program on its own: no console window, not stopped by Ctrl+C in the
/// terminal that started it, outliving its starter.
pub fn start_process(program: &Path, args: &[OsString]) -> io::Result<()> {
    let mut cmd = Command::new(program);
    cmd.args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
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

/// A program to start the core from that leaves the real one free. On Windows a running
/// program can't be replaced, so the core runs from a copy in the temp folder and an update
/// (or a rebuild) can still write the installed file. Copies no core uses any more go.
pub fn runnable_copy(program: &Path) -> io::Result<std::path::PathBuf> {
    if !cfg!(windows) {
        return Ok(program.to_path_buf());
    }
    let dir = std::env::temp_dir().join("detour-core");
    std::fs::create_dir_all(&dir)?;
    for old in std::fs::read_dir(&dir)?.flatten() {
        // One a core still runs from is in use and stays.
        let _ = std::fs::remove_file(old.path());
    }
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let name = program.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "detour".into());
    let copy = dir.join(format!("{name}-core-{stamp:x}.exe"));
    std::fs::copy(program, &copy)?;
    Ok(copy)
}

/// Starts `<detour> daemon --data-dir <dir>`.
pub fn start_core(detour: &Path, data_dir: &Path) -> io::Result<()> {
    start_process(&runnable_copy(detour)?, &["daemon".into(), "--data-dir".into(), data_dir.as_os_str().to_owned()])
}

/// The core of a vault folder, started with `detour daemon` if it doesn't run yet.
pub async fn connect_or_start(detour: &Path, data_dir: &Path) -> io::Result<Client> {
    connect_or_start_with(data_dir, || start_core(detour, data_dir)).await
}

/// The core of a vault folder, started by `start` if it doesn't run yet.
pub async fn connect_or_start_with(data_dir: &Path, start: impl FnOnce() -> io::Result<()>) -> io::Result<Client> {
    let socket = socket_name(data_dir);
    if let Ok(c) = Client::connect(&socket).await {
        return Ok(c);
    }
    start()?;
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
