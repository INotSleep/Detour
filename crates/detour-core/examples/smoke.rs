//! Manual smoke test of a real SSH login through detour-core.
//!
//! cargo run -p detour-core --example smoke -- <user@host[:port]> <private key file> [command]
//!
//! Trusts the server key on first use (prints its fingerprint), opens a PTY
//! shell, runs the command (default `uname -a`) and exits.

use std::time::Duration;

use detour_core::model::{Auth, Host};
use detour_core::ssh::{self, ConnectRequest, SessionEvent};
use detour_core::{Error, keys};

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [target, key_path, rest @ ..] = args.as_slice() else {
        eprintln!("usage: smoke <user@host[:port]> <private key file> [command]");
        std::process::exit(2);
    };
    let command = if rest.is_empty() { "uname -a".to_string() } else { rest.join(" ") };
    let (user, hostport) = target.split_once('@').expect("user@host");
    let (address, port) = match hostport.rsplit_once(':') {
        Some((a, p)) => (a.to_string(), p.parse().expect("port")),
        None => (hostport.to_string(), 22),
    };

    let entry = keys::import("smoke", &std::fs::read_to_string(key_path).expect("key file"), None, false).expect("key");
    let host = Host { id: "smoke".into(), group: String::new(), label: address.clone(), address, port, user: user.into(), auth: Auth::Key { key_id: entry.id.clone() }, tags: vec![], last_connected: None, password: None };
    let mk = |trust: Option<String>| ConnectRequest { host: host.clone(), key: Some(entry.clone()), secret: None, known: None, trust_fingerprint: trust };

    let fp = match ssh::connect(mk(None), 120, 30).await {
        Err(Error::HostKeyUnknown { fingerprint }) => fingerprint,
        Err(e) => panic!("connect: {e} ({e:?})"),
        Ok(_) => unreachable!("an unpinned host must ask for trust first"),
    };
    println!("host key: {fp}");

    let mut c = ssh::connect(mk(Some(fp)), 120, 30).await.unwrap_or_else(|e| panic!("connect: {e} ({e:?})"));
    println!("connected, pinned: {}", c.new_known_host.is_some());
    c.handle.input(format!("{command}; exit\r").into_bytes());

    let mut out = Vec::new();
    let status = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            match c.events.recv().await {
                Some(SessionEvent::Data(d)) => out.extend(d),
                Some(SessionEvent::Closed { exit_status }) => return exit_status,
                None => return None,
            }
        }
    })
    .await
    .expect("session did not finish in time");
    println!("--- output ---\n{}\n--- exit status: {status:?}", String::from_utf8_lossy(&out));
}
