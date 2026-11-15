use super::*;
use std::time::Duration as StdDuration;

struct Fixture {
    me: Client,
    agent: Detour,
    socket: String,
    daemon: tokio::task::JoinHandle<std::io::Result<()>>,
}

impl Fixture {
    async fn start() -> Self {
        let dir = std::env::temp_dir().join(format!("detour-mcp-access-{}", detour_core::new_id()));
        std::fs::create_dir_all(&dir).unwrap();
        let socket = detour_ipc::socket_name(&dir);
        let daemon = tokio::spawn(async move { detour_daemon::run(&dir, None).await });
        let me = tokio::time::timeout(StdDuration::from_secs(5), async {
            loop {
                if let Ok(c) = Client::connect(&socket).await {
                    break c;
                }
                tokio::time::sleep(StdDuration::from_millis(10)).await;
            }
        }).await.unwrap();
        me.call("hello", json!({ "client": "cli" })).await.unwrap();
        me.call("vault_create", json!({ "masterPassword": "test-only" })).await.unwrap();
        let hosts: Vec<Value> = ["web", "db"].into_iter().map(|name| json!({
            "id": name, "label": name, "group": "", "address": "127.0.0.1", "port": 9,
            "user": "test", "auth": { "type": "agent" }, "tags": [], "hasPassword": false,
        })).collect();
        me.call("vault_put", json!({ "update": { "hosts": hosts, "keys": [], "snippets": [], "tunnels": [] } })).await.unwrap();
        let agent = Self::connect(&socket).await;
        Self { me, agent, socket, daemon }
    }

    async fn connect(socket: &str) -> Detour {
        let client = Client::connect(socket).await.unwrap();
        client.call("hello", json!({ "client": "mcp" })).await.unwrap();
        let agent = Detour::new(PathBuf::new());
        agent.grants.lock().unwrap().clear();
        *agent.link.lock().await = Some(client);
        agent
    }

    async fn request(&self) -> String {
        let r = self.agent.core("agent_request", json!({
            "agent": "Test MCP client", "hosts": ["web"], "scopes": ["connect"], "waitSecs": 0,
        })).await.unwrap();
        assert_eq!(r["status"], "pending");
        s(&r["requestId"])
    }

    async fn answer(&self, id: &str, allow: bool) -> Value {
        self.me.call("agents_answer", json!({
            "id": id, "allow": allow, "ttl": "never", "hosts": [], "reach": [{ "type": "all" }],
            "scopes": ["connect", "exec", "files:read", "files:write", "tunnels", "vault:edit", "hostkeys:new"],
        })).await.unwrap()
    }

    async fn hosts(&self) -> Vec<Value> {
        serde_json::from_str(&self.agent.list_hosts().await.unwrap()).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.daemon.abort();
    }
}

#[tokio::test]
async fn a_late_approval_reaches_the_next_tool_without_another_wait_call() {
    let f = Fixture::start().await;
    let id = f.request().await;
    let waiting = f.agent.wait_for_access(Parameters(RequestId { request_id: id.clone() }));
    assert!(tokio::time::timeout(StdDuration::from_millis(50), waiting).await.is_err());
    let given = f.answer(&id, true).await;
    assert!(f.me.call("agents_list", json!({})).await.unwrap()[0]["expiresAt"].is_null());

    let hosts = tokio::time::timeout(StdDuration::from_secs(2), async {
        loop {
            let hosts = f.hosts().await;
            if hosts.iter().all(|h| h["access"].as_array().unwrap().len() == 7) {
                break hosts;
            }
            tokio::time::sleep(StdDuration::from_millis(10)).await;
        }
    }).await.expect("approved access never reached the MCP server");
    assert_eq!(hosts.len(), 2);
    assert_eq!(f.agent.grants.lock().unwrap().len(), 1);

    let other = Fixture::connect(&f.socket).await;
    let other_hosts: Vec<Value> = serde_json::from_str(&other.list_hosts().await.unwrap()).unwrap();
    assert!(other_hosts.iter().all(|h| h["access"] == json!([])));

    let wait = || f.agent.wait_for_access(Parameters(RequestId { request_id: id.clone() }));
    let (a, b) = tokio::join!(wait(), wait());
    assert_eq!(a, b);
    assert!(a.unwrap().starts_with("Allowed:"));
    assert_eq!(f.agent.grants.lock().unwrap().len(), 1);

    f.me.call("agents_revoke", json!({ "id": given["tokenId"] })).await.unwrap();
    assert!(f.hosts().await.iter().all(|h| h["access"] == json!([])));
}

#[tokio::test]
async fn a_late_denial_is_shared_by_waiters_and_never_grants_access() {
    let f = Fixture::start().await;
    let id = f.request().await;
    let waiting = f.agent.wait_for_access(Parameters(RequestId { request_id: id.clone() }));
    assert!(tokio::time::timeout(StdDuration::from_millis(50), waiting).await.is_err());
    f.answer(&id, false).await;
    let wait = || f.agent.wait_for_access(Parameters(RequestId { request_id: id.clone() }));
    let (a, b) = tokio::join!(wait(), wait());
    assert_eq!(a, Err("The person denied the request.".into()));
    assert_eq!(a, b);
    assert!(f.agent.grants.lock().unwrap().is_empty());
    assert!(f.hosts().await.iter().all(|h| h["access"] == json!([])));
}

#[tokio::test]
async fn an_unknown_access_request_stays_an_error() {
    let f = Fixture::start().await;
    let result = f.agent.wait_for_access(Parameters(RequestId { request_id: "missing".into() })).await;
    assert_eq!(result, Err("Not found: request.".into()));
    assert!(f.agent.grants.lock().unwrap().is_empty());
}
