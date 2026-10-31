//! AI agents and their tokens: an agent gets only what the person allowed, works
//! only in its own sessions, never sees a secret, and loses everything on revoke.

use std::sync::Arc;
use std::time::Duration;

use detour_ipc::Client;
use russh::keys::{Algorithm, PrivateKey};
use russh::server::{self, Msg, Server as _, Session};
use russh::{Channel, ChannelId};
use serde_json::{Value, json};
use tokio::net::TcpListener;

/// A real core (the window not trusted by itself), as an agent finds it.
async fn start(tag: &str) -> String {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("detour-agents-{tag}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let socket = detour_ipc::socket_name(&dir);
    tokio::spawn(async move { detour_daemon::run(&dir, None).await.unwrap() });
    for _ in 0..100 {
        if Client::connect(&socket).await.is_ok() {
            return socket;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the core did not start");
}

async fn hello(socket: &str, client: &str) -> Client {
    let c = Client::connect(socket).await.unwrap();
    c.call("hello", json!({ "client": client })).await.unwrap();
    c
}

async fn ok(c: &Client, method: &str, params: Value) -> Value {
    c.call(method, params).await.unwrap_or_else(|e| panic!("{method}: {e}"))
}

fn kind(r: Result<Value, Value>) -> String {
    r.err().map(|e| e["kind"].as_str().unwrap_or_default().to_string()).unwrap_or_else(|| "ok".into())
}

#[derive(Clone)]
struct Sshd;

impl server::Server for Sshd {
    type Handler = Self;
    fn new_client(&mut self, _: Option<std::net::SocketAddr>) -> Self {
        self.clone()
    }
}

impl server::Handler for Sshd {
    type Error = russh::Error;

    async fn auth_password(&mut self, user: &str, password: &str) -> Result<server::Auth, Self::Error> {
        Ok(if user == "u" && password == "pw" { server::Auth::Accept } else { server::Auth::reject() })
    }

    async fn channel_open_session(&mut self, _: Channel<Msg>, reply: server::ChannelOpenHandle, _: &mut Session) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }

    async fn shell_request(&mut self, channel: ChannelId, session: &mut Session) -> Result<(), Self::Error> {
        session.data(channel, b"ready$ ".to_vec())?;
        Ok(())
    }

    async fn data(&mut self, channel: ChannelId, data: &[u8], session: &mut Session) -> Result<(), Self::Error> {
        session.data(channel, [b"echo:".as_slice(), data].concat())?;
        Ok(())
    }

    // `fail` writes to stderr and exits 3; anything else is echoed back.
    async fn exec_request(&mut self, channel: ChannelId, data: &[u8], session: &mut Session) -> Result<(), Self::Error> {
        let code = if data == b"fail" {
            session.extended_data(channel, 1, b"it broke\n".to_vec())?;
            3
        } else {
            session.data(channel, [b"ran:".as_slice(), data, b"\n"].concat())?;
            0
        };
        session.exit_status_request(channel, code)?;
        session.eof(channel)?;
        session.close(channel)?;
        Ok(())
    }
}

async fn sshd() -> u16 {
    let config = Arc::new(server::Config {
        keys: vec![PrivateKey::random(&mut rand::rng(), Algorithm::Ed25519).unwrap()],
        auth_rejection_time: Duration::from_millis(10),
        auth_rejection_time_initial: Some(Duration::from_millis(0)),
        ..Default::default()
    });
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        let _ = Sshd.run_on_socket(config, &listener).await;
    });
    port
}

fn host(id: &str, label: &str, port: u16) -> Value {
    json!({ "id": id, "group": "", "label": label, "address": "127.0.0.1", "port": port, "user": "u",
            "auth": { "type": "password" }, "tags": [], "hasPassword": false, "newPassword": "pw" })
}

/// The person's vault: `web` (its server key pinned) and `db` (never connected).
async fn person(socket: &str) -> Client {
    let (port, other) = (sshd().await, sshd().await);
    let me = hello(socket, "cli").await;
    ok(&me, "vault_create", json!({ "masterPassword": "master" })).await;
    ok(&me, "vault_put", json!({ "update": { "hosts": [host("h1", "web", port), host("h2", "db", other)], "keys": [], "snippets": [], "tunnels": [] } })).await;
    let open = |fp: Value| json!({ "args": { "vaultId": "personal", "hostId": "h1", "cols": 80, "rows": 24, "trustFingerprint": fp } });
    let fp = me.call("session_open", open(Value::Null)).await.unwrap_err()["fingerprint"].clone();
    let s = ok(&me, "session_open", open(fp)).await;
    ok(&me, "session_close", json!({ "sessionId": s["sessionId"] })).await;
    me
}

/// Asks for access and has the person answer as `detour approve` does.
async fn granted(agent: &Client, me: &Client, hosts: Value, scopes: Value) -> String {
    let asked = ok(agent, "agent_request", json!({ "agent": "Test agent", "hosts": hosts, "scopes": scopes, "reason": "deploy" })).await;
    assert_eq!(asked["status"], "pending");
    let pending = ok(me, "agents_pending", json!({})).await;
    let req = pending.as_array().unwrap().iter().find(|p| p["id"] == asked["requestId"]).unwrap().clone();
    assert_eq!((req["type"].as_str(), req["agent"].as_str(), req["reason"].as_str()), (Some("access"), Some("Test agent"), Some("deploy")));
    ok(me, "agents_answer", json!({ "id": req["id"], "allow": true, "ttl": "2h" })).await;
    let got = ok(agent, "agent_wait", json!({ "requestId": asked["requestId"] })).await;
    assert_eq!(got["status"], "approved");
    got["token"].as_str().unwrap().to_string()
}

/// Answers the next command an agent waits on.
async fn answer_command(me: &Client, allow: bool, always: bool) -> Value {
    for _ in 0..200 {
        let pending = ok(me, "agents_pending", json!({})).await;
        if let Some(p) = pending.as_array().unwrap().iter().find(|p| p["type"] == "command") {
            ok(me, "agents_answer", json!({ "id": p["id"], "allow": allow, "always": always })).await;
            return p.clone();
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("no command to answer");
}

#[tokio::test]
async fn an_agent_gets_what_it_was_allowed_and_nothing_else() {
    let socket = start("scopes").await;
    let me = person(&socket).await;
    let agent = hello(&socket, "mcp").await;

    // Without a token it may only look at the host list and ask.
    assert_eq!(kind(agent.call("vault_get", json!({})).await), "notTrusted");
    assert_eq!(kind(agent.call("agents_pending", json!({})).await), "notTrusted");
    let hosts = ok(&agent, "agent_hosts", json!({})).await;
    assert_eq!(hosts.as_array().unwrap().len(), 2);
    assert!(hosts.to_string().find("pw").is_none(), "no password in {hosts}");
    assert_eq!(kind(agent.call("agent_request", json!({ "agent": "x", "hosts": ["nope"], "scopes": ["exec"] })).await), "notFound");

    // Allowed to connect to `web`: commands wait for the person.
    let token = granted(&agent, &me, json!(["web"]), json!(["connect"])).await;
    assert_eq!(kind(agent.call("agent_session_open", json!({ "token": token, "host": "db" })).await), "agentHost");
    let opened = ok(&agent, "agent_session_open", json!({ "token": token, "host": "web" })).await;
    let session = opened["sessionId"].clone();
    let list = ok(&me, "sessions_list", json!({})).await;
    assert_eq!(list[0]["owner"], json!({ "type": "agent", "tokenId": list[0]["owner"]["tokenId"], "agent": "Test agent" }));

    // The person denies one command, then allows the next and every one after it.
    let run = |cmd: &str| json!({ "token": token, "sessionId": session, "command": cmd });
    let (r, asked) = tokio::join!(agent.call("agent_run", run("rm -rf /")), answer_command(&me, false, false));
    assert_eq!((kind(r), asked["command"].as_str()), ("denied".into(), Some("rm -rf /")));
    let (r, _) = tokio::join!(agent.call("agent_run", run("uptime")), answer_command(&me, true, true));
    assert_eq!(r.unwrap()["stdout"], "ran:uptime\n");
    let r = ok(&agent, "agent_run", run("fail")).await;
    assert_eq!((r["exitStatus"].as_u64(), r["stderr"].as_str(), r["stdout"].as_str()), (Some(3), Some("it broke\n"), Some("")));
    // What it ran shows in the session's tab, for the person to see.
    let runs = ok(&me, "session_runs", json!({ "sessionId": session })).await;
    let shown: Vec<(String, Value, String)> = runs.as_array().unwrap().iter().map(|r| (r["command"].as_str().unwrap().into(), r["exitStatus"].clone(), r["output"].as_str().unwrap().into())).collect();
    assert_eq!(shown, [("uptime".into(), json!(0), "ran:uptime\n".into()), ("fail".into(), json!(3), "it broke\n".into())]);
    ok(&agent, "agent_input", json!({ "token": token, "sessionId": session, "data": "ls\r" })).await;

    // Secrets and the person's own methods stay out of reach, even through agent_call.
    for m in ["host_password", "key_export", "team_member_add", "agents_answer", "vault_lock"] {
        assert_eq!(kind(agent.call("agent_call", json!({ "token": token, "method": m, "params": {} })).await), "forbidden", "{m}");
    }
    assert_eq!(kind(agent.call("agent_call", json!({ "token": token, "method": "vault_put", "params": { "update": {} } })).await), "agentScope");
    assert_eq!(kind(agent.call("agent_call", json!({ "token": token, "method": "vault_get" })).await), "ok");
    assert_eq!(kind(agent.call("agent_call", json!({ "token": "dtr_made_up", "method": "vault_get" })).await), "tokenInvalid");
}

#[tokio::test]
async fn sessions_are_their_agents_own_and_revoking_ends_them() {
    let socket = start("own").await;
    let me = person(&socket).await;
    let (a, b) = (hello(&socket, "mcp").await, hello(&socket, "mcp").await);
    let ta = granted(&a, &me, json!(["web"]), json!(["exec"])).await;
    let tb = granted(&b, &me, json!(["web", "db"]), json!(["exec", "files:read"])).await;

    let sa = ok(&a, "agent_session_open", json!({ "token": ta, "host": "web" })).await["sessionId"].clone();
    // Another agent's session doesn't exist for it, whatever it tries.
    for (m, extra) in [("agent_input", json!({ "data": "x" })), ("agent_screen", json!({})), ("agent_run", json!({ "command": "id" })), ("agent_session_close", json!({}))] {
        let mut p = json!({ "token": tb, "sessionId": sa });
        p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        assert_eq!(kind(b.call(m, p).await), "notYourSession", "{m}");
    }
    assert_eq!(ok(&b, "agent_sessions", json!({ "token": tb })).await, json!([]));
    assert_eq!(ok(&a, "agent_sessions", json!({ "token": ta })).await[0]["id"], sa);

    // An agent can't accept a server key: `db` was never connected to.
    assert_eq!(kind(b.call("agent_session_open", json!({ "token": tb, "host": "db" })).await), "hostKeyUnknown");

    // Revoked: its session closes and the token stops working.
    let tokens = ok(&me, "agents_list", json!({})).await;
    assert_eq!(tokens.as_array().unwrap().len(), 2);
    assert!(tokens.to_string().find(&ta).is_none(), "the vault keeps only a hash");
    let id = tokens.as_array().unwrap().iter().find(|t| t["scopes"] == json!(["exec"])).unwrap()["id"].clone();
    assert_eq!(ok(&me, "agents_revoke", json!({ "id": id })).await, true);
    assert_eq!(kind(a.call("agent_whoami", json!({ "token": ta })).await), "tokenInvalid");
    for _ in 0..100 {
        if ok(&me, "sessions_list", json!({})).await == json!([]) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(ok(&me, "sessions_list", json!({})).await, json!([]));
    assert_eq!(ok(&b, "agent_whoami", json!({ "token": tb })).await["agent"], "Test agent");
}

#[tokio::test]
async fn the_person_may_deny_or_narrow_a_request() {
    let socket = start("deny").await;
    let me = person(&socket).await;
    let agent = hello(&socket, "mcp").await;

    let asked = ok(&agent, "agent_request", json!({ "agent": "A", "hosts": ["web"], "scopes": ["exec"] })).await;
    let (got, _) = tokio::join!(
        agent.call("agent_wait", json!({ "requestId": asked["requestId"], "waitSecs": 5 })),
        async { ok(&me, "agents_answer", json!({ "id": asked["requestId"], "allow": false })).await }
    );
    assert_eq!(got.unwrap()["status"], "denied");
    assert_eq!(ok(&me, "agents_pending", json!({})).await, json!([]));

    // Asked for more than it needs: the person keeps one host and one scope, until revoked.
    let asked = ok(&agent, "agent_request", json!({ "agent": "A", "hosts": ["web", "db"], "scopes": ["exec", "files:write"], "ttl": "7d" })).await;
    let given = ok(&me, "agents_answer", json!({ "id": asked["requestId"], "allow": true, "ttl": "never", "scopes": ["files:write"], "hosts": [{ "vaultId": "personal", "hostId": "h2" }] })).await;
    let got = ok(&agent, "agent_wait", json!({ "requestId": asked["requestId"] })).await;
    assert_eq!(got["token"], given["token"]);
    let me_too = ok(&agent, "agent_whoami", json!({ "token": got["token"] })).await;
    assert_eq!((me_too["scopes"].clone(), me_too["expiresAt"].clone(), me_too["hosts"][0]["label"].clone()), (json!(["files:write"]), Value::Null, json!("db")));
    assert_eq!(kind(agent.call("agent_session_open", json!({ "token": got["token"], "host": "db" })).await), "agentScope");
    // The token is taken once.
    assert_eq!(kind(agent.call("agent_wait", json!({ "requestId": asked["requestId"] })).await), "notFound");
}
