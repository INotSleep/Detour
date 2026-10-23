//! Local SSH server for trying Detour by hand, no real machine needed.
//!
//! cargo run -p detour-core --example test_server -- [port] [password]
//!
//! Accepts user `demo` with the given password (default `demo-pass`) and any
//! public key, and runs a tiny line shell (`exit` ends the session).
//!
//! Also serves Git repositories on this machine to user `git` with any key,
//! through the installed `git`: `ssh://git@127.0.0.1:2222/C:/repos/team.git`.
//!
//! SFTP runs OpenSSH's `sftp-server` (Git for Windows ships one) in the
//! directory given by DETOUR_SFTP_ROOT, or the current one.

use std::sync::Arc;

use russh::keys::{Algorithm, PrivateKey, PublicKey};
use russh::server::{self, Msg, Server as _, Session};
use russh::{Channel, ChannelId};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Clone)]
struct Srv {
    password: String,
    line: String,
    /// Input of the `git` process this connection runs, if any.
    git: Option<Arc<tokio::sync::Mutex<tokio::process::ChildStdin>>>,
    /// The channel the line shell runs on; data of other channels (tunnels) is not ours.
    shell: Option<ChannelId>,
    /// The channel running `sftp-server`, and its input.
    sftp: Option<(ChannelId, Arc<tokio::sync::Mutex<tokio::process::ChildStdin>>)>,
}

fn sftp_server() -> Option<std::path::PathBuf> {
    ["C:/Program Files/Git/usr/lib/ssh/sftp-server.exe", "/usr/lib/openssh/sftp-server", "/usr/libexec/sftp-server", "/usr/lib/ssh/sftp-server"]
        .iter()
        .map(std::path::PathBuf::from)
        .find(|p| p.exists())
}

const PROMPT: &[u8] = b"\x1b[1;32mdemo@detour-test\x1b[0m:\x1b[1;34m~\x1b[0m$ ";

impl server::Server for Srv {
    type Handler = Self;
    fn new_client(&mut self, peer: Option<std::net::SocketAddr>) -> Self {
        println!("client {peer:?}");
        self.clone()
    }
}

impl server::Handler for Srv {
    type Error = russh::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<server::Auth, Self::Error> {
        Ok(if user == "demo" && password == self.password { server::Auth::Accept } else { server::Auth::reject() })
    }

    async fn auth_publickey(&mut self, user: &str, _: &PublicKey) -> Result<server::Auth, Self::Error> {
        Ok(if user == "demo" || user == "git" { server::Auth::Accept } else { server::Auth::reject() })
    }

    async fn exec_request(&mut self, channel: ChannelId, data: &[u8], session: &mut Session) -> Result<(), Self::Error> {
        let cmd = String::from_utf8_lossy(data).to_string();
        println!("exec {cmd}");
        let Some((service @ ("git-upload-pack" | "git-receive-pack"), path)) = cmd.split_once(' ') else {
            session.exit_status_request(channel, 127)?;
            session.close(channel)?;
            return Ok(());
        };
        let path = path.trim_matches('\'');
        // ssh://host/C:/x arrives as /C:/x
        let path = if path.as_bytes().get(2) == Some(&b':') { &path[1..] } else { path };
        let mut child = tokio::process::Command::new("git")
            .arg(&service[4..])
            .arg(path)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()?;
        self.git = Some(Arc::new(tokio::sync::Mutex::new(child.stdin.take().unwrap())));
        let mut stdout = child.stdout.take().unwrap();
        let handle = session.handle();
        tokio::spawn(async move {
            let mut buf = vec![0u8; 32 * 1024];
            while let Ok(n) = stdout.read(&mut buf).await {
                if n == 0 || handle.data(channel, buf[..n].to_vec()).await.is_err() {
                    break;
                }
            }
            let code = child.wait().await.map(|s| s.code().unwrap_or(1) as u32).unwrap_or(1);
            let _ = handle.exit_status_request(channel, code).await;
            let _ = handle.eof(channel).await;
            let _ = handle.close(channel).await;
        });
        Ok(())
    }

    /// `ssh -L` and `ssh -D`: connect where the client asks, from here.
    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<Msg>,
        host: &str,
        port: u32,
        _: &str,
        _: u32,
        reply: server::ChannelOpenHandle,
        _: &mut Session,
    ) -> Result<(), Self::Error> {
        println!("direct-tcpip {host}:{port}");
        match tokio::net::TcpStream::connect((host, port as u16)).await {
            Ok(mut tcp) => {
                reply.accept().await;
                tokio::spawn(async move {
                    let mut ch = channel.into_stream();
                    let _ = tokio::io::copy_bidirectional(&mut ch, &mut tcp).await;
                });
            }
            Err(_) => reply.reject(russh::ChannelOpenFailure::ConnectFailed).await,
        }
        Ok(())
    }

    /// `ssh -R`: listen on this machine's loopback and send connections to the client.
    async fn tcpip_forward(&mut self, address: &str, port: &mut u32, session: &mut Session) -> Result<bool, Self::Error> {
        println!("tcpip-forward {address}:{port}");
        let Ok(listener) = tokio::net::TcpListener::bind(("127.0.0.1", *port as u16)).await else { return Ok(false) };
        let (handle, address, port) = (session.handle(), address.to_string(), *port);
        tokio::spawn(async move {
            while let Ok((mut sock, from)) = listener.accept().await {
                let Ok(ch) = handle.channel_open_forwarded_tcpip(address.clone(), port, from.ip().to_string(), from.port() as u32).await else { break };
                tokio::spawn(async move {
                    let mut ch = ch.into_stream();
                    let _ = tokio::io::copy_bidirectional(&mut ch, &mut sock).await;
                });
            }
        });
        Ok(true)
    }

    async fn channel_open_session(&mut self, _: Channel<Msg>, reply: server::ChannelOpenHandle, _: &mut Session) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }

    async fn subsystem_request(&mut self, channel: ChannelId, name: &str, session: &mut Session) -> Result<(), Self::Error> {
        println!("subsystem {name}");
        let Some(program) = sftp_server().filter(|_| name == "sftp") else {
            session.channel_failure(channel)?;
            return Ok(());
        };
        let root = std::env::var("DETOUR_SFTP_ROOT").unwrap_or_else(|_| ".".into());
        // Git for Windows' sftp-server needs the DLLs in Git's usr/bin.
        let mut path = std::ffi::OsString::from(program.ancestors().nth(3).map(|u| u.join("bin")).unwrap_or_default());
        path.push(if cfg!(windows) { ";" } else { ":" });
        path.push(std::env::var_os("PATH").unwrap_or_default());
        let mut child = tokio::process::Command::new(&program)
            .env("PATH", path)
            .current_dir(root)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .spawn()?;
        self.sftp = Some((channel, Arc::new(tokio::sync::Mutex::new(child.stdin.take().unwrap()))));
        let mut stdout = child.stdout.take().unwrap();
        let handle = session.handle();
        tokio::spawn(async move {
            let mut buf = vec![0u8; 64 * 1024];
            while let Ok(n) = stdout.read(&mut buf).await {
                if n == 0 || handle.data(channel, buf[..n].to_vec()).await.is_err() {
                    break;
                }
            }
            let _ = child.wait().await;
            let _ = handle.close(channel).await;
        });
        session.channel_success(channel)?;
        Ok(())
    }

    async fn shell_request(&mut self, channel: ChannelId, session: &mut Session) -> Result<(), Self::Error> {
        self.shell = Some(channel);
        session.data(channel, b"Welcome to the Detour test server\r\n".to_vec())?;
        session.data(channel, PROMPT.to_vec())?;
        Ok(())
    }

    async fn data(&mut self, channel: ChannelId, data: &[u8], session: &mut Session) -> Result<(), Self::Error> {
        if let Some(git) = &self.git {
            let _ = git.lock().await.write_all(data).await;
            return Ok(());
        }
        if let Some((ch, input)) = &self.sftp {
            if *ch == channel {
                let _ = input.lock().await.write_all(data).await;
                return Ok(());
            }
        }
        if self.shell != Some(channel) {
            return Ok(());
        }
        for &b in data {
            match b {
                b'\r' => {
                    let line = std::mem::take(&mut self.line);
                    session.data(channel, b"\r\n".to_vec())?;
                    match line.trim() {
                        "exit" => {
                            session.data(channel, b"logout\r\n".to_vec())?;
                            session.exit_status_request(channel, 0)?;
                            session.close(channel)?;
                            return Ok(());
                        }
                        "" => {}
                        l => session.data(channel, format!("{l}: ok\r\n").into_bytes())?,
                    }
                    session.data(channel, PROMPT.to_vec())?;
                }
                0x7f => {
                    if self.line.pop().is_some() {
                        session.data(channel, b"\x08 \x08".to_vec())?;
                    }
                }
                b if b >= b' ' => {
                    self.line.push(b as char);
                    session.data(channel, vec![b])?;
                }
                _ => {}
            }
        }
        Ok(())
    }
}

#[tokio::main]
async fn main() {
    let mut args = std::env::args().skip(1);
    let port: u16 = args.next().map(|p| p.parse().expect("port")).unwrap_or(2222);
    let password = args.next().unwrap_or_else(|| "demo-pass".into());
    let key = PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap();
    println!("host key {}", key.public_key().fingerprint(Default::default()));
    println!("listening on 127.0.0.1:{port}, user demo / {password}");
    let config = Arc::new(server::Config { keys: vec![key], ..Default::default() });
    Srv { password, line: String::new(), git: None, shell: None, sftp: None }.run_on_address(config, ("127.0.0.1", port)).await.unwrap();
}
