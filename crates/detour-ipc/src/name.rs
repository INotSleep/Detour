//! Where the core listens: one socket per vault folder, so a test, a dev build and
//! the installed app never meet, reachable only by the user who runs it.

use std::io;
use std::path::Path;

use interprocess::local_socket::tokio::{Listener, Stream, prelude::*};
use interprocess::local_socket::{ListenerOptions, Name};

/// A short stable name for the vault folder (FNV-1a of its path).
pub fn socket_name(data_dir: &Path) -> String {
    let path = data_dir.display().to_string();
    let path = if cfg!(windows) { path.to_lowercase() } else { path };
    let hash = path.bytes().fold(0xcbf29ce484222325u64, |h, b| (h ^ b as u64).wrapping_mul(0x100000001b3));
    format!("detour-{hash:016x}")
}

#[cfg(windows)]
fn name(id: &str) -> io::Result<Name<'static>> {
    use interprocess::local_socket::GenericNamespaced;
    id.to_string().to_ns_name::<GenericNamespaced>()
}

/// On Unix the socket is a file in a folder only this user can enter.
#[cfg(unix)]
fn name(id: &str) -> io::Result<Name<'static>> {
    use interprocess::local_socket::GenericFilePath;
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    let base = std::env::var_os("XDG_RUNTIME_DIR").map(std::path::PathBuf::from).unwrap_or_else(std::env::temp_dir);
    // SAFETY: getuid has no preconditions and cannot fail.
    let uid = unsafe { libc::getuid() };
    let dir = base.join(format!("detour-{uid}"));
    match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    let meta = std::fs::symlink_metadata(&dir)?;
    if !meta.is_dir() || meta.uid() != uid || meta.permissions().mode() & 0o077 != 0 {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("{} is not this user's private folder", dir.display())));
    }
    dir.join(format!("{id}.sock")).to_fs_name::<GenericFilePath>().map(|n| n.into_owned())
}

/// The core's end of the socket.
pub struct Server(Listener);

impl Server {
    pub async fn accept(&self) -> io::Result<Stream> {
        self.0.accept().await
    }
}

/// Listens on the socket of a vault folder. Fails with `AddrInUse` when another core
/// answers there; a socket file left by one that crashed is replaced.
pub async fn listen(id: &str) -> io::Result<Server> {
    if connect(id).await.is_ok() {
        return Err(io::Error::new(io::ErrorKind::AddrInUse, "another Detour core is running"));
    }
    let opts = ListenerOptions::new().name(name(id)?).reclaim_name(true).try_overwrite(true);
    #[cfg(windows)]
    let opts = {
        use interprocess::os::windows::local_socket::ListenerOptionsExt;
        use interprocess::os::windows::security_descriptor::SecurityDescriptor;
        // Only the pipe's owner, this user, may connect.
        let sddl = widestring::U16CString::from_str("D:P(A;;GA;;;OW)").map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        opts.security_descriptor(SecurityDescriptor::deserialize(&sddl)?)
    };
    // Where the system lets a socket have its own mode; elsewhere (macOS) the 0700 folder
    // around it is what keeps other users out.
    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd"))]
    let opts = {
        use interprocess::os::unix::local_socket::ListenerOptionsExt;
        opts.mode(0o600)
    };
    opts.create_tokio().map(Server)
}

pub(crate) async fn connect(id: &str) -> io::Result<Stream> {
    Stream::connect(name(id)?).await
}
