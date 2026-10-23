//! Local SSH server for trying Detour by hand, no real machine needed.
//!
//! cargo run -p detour-core --example test_server -- [port] [password]
//!
//! Accepts user `demo` with the given password (default `demo-pass`) and any
//! public key, and runs a tiny line shell (`exit` ends the session).
//!
//! Also serves Git repositories on this machine to user `git` with any key,
//! through the installed `git`: `ssh://git@127.0.0.1:2222/C:/repos/team.git`.

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

    async fn channel_open_session(&mut self, _: Channel<Msg>, reply: server::ChannelOpenHandle, _: &mut Session) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }

    async fn shell_request(&mut self, channel: ChannelId, session: &mut Session) -> Result<(), Self::Error> {
        session.data(channel, b"Welcome to the Detour test server\r\n".to_vec())?;
        session.data(channel, PROMPT.to_vec())?;
        Ok(())
    }

    async fn data(&mut self, channel: ChannelId, data: &[u8], session: &mut Session) -> Result<(), Self::Error> {
        if let Some(git) = &self.git {
            let _ = git.lock().await.write_all(data).await;
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
    Srv { password, line: String::new(), git: None }.run_on_address(config, ("127.0.0.1", port)).await.unwrap();
}
