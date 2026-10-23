//! Local SSH server for trying Detour by hand, no real machine needed.
//!
//! cargo run -p detour-core --example test_server -- [port] [password]
//!
//! Accepts user `demo` with the given password (default `demo-pass`) and any
//! public key, and runs a tiny line shell (`exit` ends the session).

use std::sync::Arc;

use russh::keys::{Algorithm, PrivateKey, PublicKey};
use russh::server::{self, Msg, Server as _, Session};
use russh::{Channel, ChannelId};

#[derive(Clone)]
struct Srv {
    password: String,
    line: String,
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
        Ok(if user == "demo" { server::Auth::Accept } else { server::Auth::reject() })
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
    Srv { password, line: String::new() }.run_on_address(config, ("127.0.0.1", port)).await.unwrap();
}
