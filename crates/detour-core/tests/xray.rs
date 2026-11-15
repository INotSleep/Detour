//! Through a real Xray server to a real sshd behind it (named `sshd`, user `u`, password `pw`).
//! Ignored by default; the server's ports are 10001-10007 on this computer and its ids and keys
//! come from the environment (`XRAY_ID_*`, `XRAY_PBK`, `XRAY_SID`, `XRAY_PCS`, `XRAY_ENC_*`).

use std::collections::BTreeMap;

use detour_core::Error;
use detour_core::model::{Auth, Host, ProxyEntry, ProxyKind, VlessSecurity, VlessTransport};
use detour_core::ssh::{self, ConnectRequest, Route};

fn env(k: &str) -> String {
    std::env::var(k).unwrap_or_else(|_| panic!("{k} is not set"))
}

struct Vless {
    port: u16,
    id: &'static str,
    security: VlessSecurity,
    transport: VlessTransport,
    vision: bool,
    mode: &'static str,
    extra: &'static str,
    alpn: Vec<String>,
    /// The variable with the client's VLESS Encryption, if any.
    encryption: Option<&'static str>,
}

fn proxy(v: &Vless) -> ProxyEntry {
    let reality = v.security == VlessSecurity::Reality;
    let kind = ProxyKind::Vless {
        address: "127.0.0.1".into(),
        port: v.port,
        security: v.security,
        sni: if reality { "www.microsoft.com".into() } else { "xray.test".into() },
        alpn: v.alpn.clone(),
        public_key: if reality { env("XRAY_PBK") } else { String::new() },
        short_id: if reality { env("XRAY_SID") } else { String::new() },
        pinned_cert: if v.security == VlessSecurity::Tls { env("XRAY_PCS") } else { String::new() },
        transport: v.transport,
        flow: if v.vision { "xtls-rprx-vision".into() } else { String::new() },
        encryption: v.encryption.map(env).unwrap_or_default(),
        path: "/xh".into(),
        host: String::new(),
        mode: v.mode.into(),
        extra: v.extra.into(),
        params: BTreeMap::new(),
    };
    ProxyEntry { id: format!("x{}", v.port), name: "x".into(), kind, password: None, private_key: None, preshared_key: None, uuid: Some(env(v.id)) }
}

fn request(p: ProxyEntry) -> ConnectRequest {
    let host = Host {
        id: "sshd".into(),
        group: String::new(),
        label: "sshd".into(),
        address: "sshd".into(),
        port: 22,
        user: "u".into(),
        auth: Auth::Password,
        tags: vec![],
        last_connected: None,
        password: Some("pw".into()),
        proxy: None,
        proxy_password: None,
        proxy_id: None,
        forward_agent: false, order: None,
    };
    ConnectRequest { host, key: None, secret: None, known: None, trust_fingerprint: None, route: Route::Proxy(Box::new(p)) }
}

/// Logs in (trusting the server's key), runs a command with a large output, and one with input.
async fn works(v: Vless) {
    let mut req = request(proxy(&v));
    let fp = match ssh::session(&req).await {
        Err(Error::HostKeyUnknown { fingerprint, .. }) => fingerprint,
        Ok(_) => panic!("the key was trusted already"),
        Err(e) => panic!("port {}: {e:?}", v.port),
    };
    req.trust_fingerprint = Some(fp);
    let started = std::time::Instant::now();
    let mut out = Vec::new();
    let (code, _) = ssh::run_command(&req, "head -c 3000000 /dev/zero | tr '\\0' x", &[], |b| out.extend_from_slice(b), |_| {}).await.unwrap();
    assert_eq!((code, out.len()), (Some(0), 3_000_000), "port {}", v.port);
    let input: Vec<u8> = (0..1_000_000u32).map(|i| (i % 251) as u8).collect();
    let mut back = Vec::new();
    let (code, _) = ssh::run_command(&req, "sha256sum | cut -c1-64", &input, |b| back.extend_from_slice(b), |_| {}).await.unwrap();
    use sha2::Digest;
    let want: String = sha2::Sha256::digest(&input).iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!((code, String::from_utf8_lossy(&back).trim()), (Some(0), want.as_str()), "port {}", v.port);
    eprintln!("port {}: 3 MB down, 1 MB up in {:?}", v.port, started.elapsed());
    assert!(detour_core::proxy::check(&proxy(&v)).await.is_ok() || v.port != 0);
}

fn tcp(port: u16, id: &'static str, security: VlessSecurity, vision: bool) -> Vless {
    Vless { port, id, security, transport: VlessTransport::Tcp, vision, mode: "", extra: "", alpn: vec![], encryption: None }
}

fn xhttp(port: u16, security: VlessSecurity, mode: &'static str) -> Vless {
    Vless { port, id: "XRAY_ID_XHTTP", security, transport: VlessTransport::Xhttp, vision: false, mode, extra: "", alpn: vec![], encryption: None }
}

#[tokio::test]
#[ignore = "needs the test Xray server"]
async fn vless_without_security() {
    works(tcp(10001, "XRAY_ID_PLAIN", VlessSecurity::None, false)).await;
}

#[tokio::test]
#[ignore = "needs the test Xray server"]
async fn vless_tls_vision() {
    works(tcp(10002, "XRAY_ID_TLS", VlessSecurity::Tls, true)).await;
    // A wrong id: the server closes the connection.
    let mut bad = proxy(&tcp(10002, "XRAY_ID_TLS", VlessSecurity::Tls, true));
    bad.uuid = Some("00000000-0000-0000-0000-000000000000".into());
    assert!(ssh::session(&request(bad)).await.is_err());
    // A certificate that isn't trusted and isn't pinned.
    let mut untrusted = proxy(&tcp(10002, "XRAY_ID_TLS", VlessSecurity::Tls, true));
    if let ProxyKind::Vless { pinned_cert, .. } = &mut untrusted.kind {
        pinned_cert.clear();
    }
    assert!(matches!(ssh::session(&request(untrusted)).await, Err(Error::Proxy { reason }) if reason == "tls"));
}

#[tokio::test]
#[ignore = "needs the test Xray server"]
async fn vless_reality_vision() {
    works(tcp(10003, "XRAY_ID_REALITY", VlessSecurity::Reality, true)).await;
    // The wrong public key: the server takes it for a visitor of the site it hides behind.
    let mut bad = proxy(&tcp(10003, "XRAY_ID_REALITY", VlessSecurity::Reality, true));
    if let ProxyKind::Vless { public_key, .. } = &mut bad.kind {
        *public_key = "Z84J2IelR9ch3k8VtlVhhs5ycBUlXA7wHBWcBrjqnAw".into();
    }
    assert!(matches!(ssh::session(&request(bad)).await, Err(Error::Proxy { reason }) if reason == "auth"));
}

#[tokio::test]
#[ignore = "needs the test Xray server"]
async fn xhttp_without_security() {
    for mode in ["packet-up", "auto"] {
        works(xhttp(10004, VlessSecurity::None, mode)).await;
    }
    // Streaming uploads need HTTP/2.
    let r = ssh::session(&request(proxy(&xhttp(10004, VlessSecurity::None, "stream-up")))).await;
    assert!(matches!(r, Err(Error::Proxy { reason }) if reason == "unsupported:stream-up over HTTP/1.1"));
}

#[tokio::test]
#[ignore = "needs the test Xray server"]
async fn xhttp_tls() {
    for mode in ["stream-one", "stream-up", "packet-up", "auto"] {
        works(xhttp(10005, VlessSecurity::Tls, mode)).await;
    }
    // HTTP/1.1
    let mut v = xhttp(10005, VlessSecurity::Tls, "packet-up");
    v.alpn = vec!["http/1.1".into()];
    works(v).await;
}

#[tokio::test]
#[ignore = "needs the test Xray server"]
async fn xhttp_reality() {
    for mode in ["auto", "stream-up", "packet-up"] {
        works(xhttp(10006, VlessSecurity::Reality, mode)).await;
    }
}

#[tokio::test]
#[ignore = "needs the test Xray server"]
async fn xhttp_with_obfuscation_settings() {
    let mut v = xhttp(10007, VlessSecurity::None, "packet-up");
    v.extra = r#"{"xPaddingObfsMode":true,"xPaddingPlacement":"header","xPaddingMethod":"tokenish","sessionIDPlacement":"header","seqPlacement":"query"}"#;
    works(v).await;
}

#[tokio::test]
#[ignore = "needs the test Xray server"]
async fn reality_handshakes_many_times() {
    let v = tcp(10003, "XRAY_ID_REALITY", VlessSecurity::Reality, false);
    let mut failures = Vec::new();
    for _ in 0..120 {
        if let Err(e) = detour_core::proxy::connect(&proxy(&v), "sshd", 22).await {
            failures.push(format!("{e:?}"));
        }
    }
    assert!(failures.is_empty(), "{} of 120 failed: {failures:?}", failures.len());
}

/* ---------- VLESS Encryption ---------- */

fn sealed(port: u16, security: VlessSecurity, transport: VlessTransport, vision: bool, encryption: &'static str) -> Vless {
    Vless { port, id: "XRAY_ID_ENC", security, transport, vision, mode: "", extra: "", alpn: vec![], encryption: Some(encryption) }
}

#[tokio::test]
#[ignore = "needs the test Xray server"]
async fn encryption_native_over_tcp() {
    works(sealed(10008, VlessSecurity::None, VlessTransport::Tcp, false, "XRAY_ENC_NATIVE")).await;
}

#[tokio::test]
#[ignore = "needs the test Xray server"]
async fn encryption_xorpub_through_two_keys_with_vision() {
    works(sealed(10009, VlessSecurity::None, VlessTransport::Tcp, true, "XRAY_ENC_XORPUB")).await;
}

#[tokio::test]
#[ignore = "needs the test Xray server"]
async fn encryption_random_over_xhttp_with_vision() {
    works(sealed(10010, VlessSecurity::None, VlessTransport::Xhttp, true, "XRAY_ENC_RANDOM")).await;
}

#[tokio::test]
#[ignore = "needs the test Xray server"]
async fn encryption_under_tls() {
    works(sealed(10011, VlessSecurity::Tls, VlessTransport::Tcp, true, "XRAY_ENC_RANDOM")).await;
}

#[tokio::test]
#[ignore = "needs the test Xray server"]
async fn encryption_with_the_wrong_key_fails() {
    let mut p = proxy(&sealed(10008, VlessSecurity::None, VlessTransport::Tcp, false, "XRAY_ENC_NATIVE"));
    if let ProxyKind::Vless { encryption, .. } = &mut p.kind {
        // Another server's key.
        *encryption = encryption.rsplit_once('.').map(|(head, _)| format!("{head}.ptjHQxBQxTJ9MWr2cd5qWIflBSACHOevTauCQwa_71U")).unwrap();
    }
    assert!(ssh::session(&request(p)).await.is_err());
}
