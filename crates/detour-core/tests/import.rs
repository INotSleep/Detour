//! Moving over from OpenSSH: a made-up ~/.ssh with the usual tricks.

use std::path::PathBuf;

use detour_core::keys;
use detour_core::model::{Auth, KeyAlgorithm, Proxy};
use detour_core::store::Store;

fn temp_dir(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let d = std::env::temp_dir().join(format!("detour-import-{tag}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

const CONFIG: &str = r#"
# Work
Host web
  HostName 10.0.0.5
  User deploy
  IdentityFile ~/.ssh/id_test

Host db
  HostName db.internal
  ProxyJump web

Host far
    HostName=10.9.9.9
    ProxyJump ops@hop1.example:2200,web

Host *.prod !api-old.prod
  User prodadmin

Host api.prod api-old.prod
  IdentityFile ~/.ssh/id_locked

Include conf.d/*

Host *
  User defuser
  Port 2022
"#;

/// A home folder with `.ssh` in it; returns `.ssh`.
fn home() -> PathBuf {
    let ssh = temp_dir("home").join(".ssh");
    std::fs::create_dir_all(ssh.join("conf.d")).unwrap();
    std::fs::write(ssh.join("config"), CONFIG).unwrap();
    std::fs::write(ssh.join("conf.d/legacy"), "Host legacy\n  HostName 1.2.3.4\n  ProxyCommand nc %h %p\n").unwrap();
    let plain = keys::generate("test", KeyAlgorithm::Ed25519, None, false).unwrap();
    std::fs::write(ssh.join("id_test"), keys::export(&plain, None).unwrap()).unwrap();
    let locked = keys::generate("locked", KeyAlgorithm::Ed25519, Some("secret"), false).unwrap();
    std::fs::write(ssh.join("id_locked"), keys::export(&locked, None).unwrap()).unwrap();
    let pk = plain.public_key.clone();
    let mut parts = pk.split_whitespace();
    let (kind, data) = (parts.next().unwrap(), parts.next().unwrap());
    std::fs::write(
        ssh.join("known_hosts"),
        format!("10.0.0.5,web.example {kind} {data}\n[10.9.9.9]:2022 {kind} {data}\n|1|abc=|def= {kind} {data}\n@cert-authority *.prod {kind} {data}\n*.wild {kind} {data}\n"),
    )
    .unwrap();
    ssh
}

#[test]
fn reads_hosts_as_ssh_does() {
    let ssh = home();
    let mut s = Store::open(temp_dir("store"));
    s.create("m").unwrap();
    let p = s.ssh_import_preview_from(&ssh).unwrap();
    assert!(p.config_found);
    let get = |a: &str| p.hosts.iter().find(|h| h.host.alias == a).unwrap_or_else(|| panic!("{a}"));
    let names: Vec<&str> = p.hosts.iter().map(|h| h.host.alias.as_str()).collect();
    assert_eq!(names, ["web", "db", "far", "api.prod", "api-old.prod", "legacy"], "patterns are not hosts");

    let web = &get("web").host;
    assert_eq!((web.address.as_str(), web.port, web.user.as_str()), ("10.0.0.5", 2022, "deploy"), "Host * fills in what web did not say");
    assert_eq!(web.key_file.as_deref(), Some(ssh.join("id_test").to_str().unwrap()));
    assert_eq!(get("db").host.jumps, ["web"]);
    assert_eq!(get("far").host.address, "10.9.9.9");
    assert_eq!(get("api.prod").host.user, "prodadmin");
    assert_eq!(get("api-old.prod").host.user, "defuser", "a negated pattern keeps a host out");
    assert_eq!(get("legacy").host.unsupported.as_deref(), Some("proxyCommand"));
    assert_eq!((p.known_new, p.known_hashed), (3, 1), "names of a line count one by one; hashed, wildcard and CA lines don't");
}

#[test]
fn imports_hosts_keys_routes_and_known_hosts() {
    let ssh = home();
    let mut s = Store::open(temp_dir("store"));
    s.create("m").unwrap();
    let r = s.ssh_import_from(&ssh, &["far".into(), "db".into(), "api.prod".into()], true).unwrap();
    // far, db, web (they go through it), hop1 (written out in far's ProxyJump), api.prod.
    assert_eq!((r.hosts, r.keys, r.known), (5, 2, 3));

    let v = s.view().unwrap();
    let id = |label: &str| v.hosts.iter().find(|h| h.label == label).unwrap_or_else(|| panic!("{label}")).id.clone();
    let host = |label: &str| v.hosts.iter().find(|h| h.label == label).unwrap().clone();
    assert_eq!(host("db").proxy, Some(Proxy::Jump { host_id: id("web") }));
    assert_eq!(host("far").proxy, Some(Proxy::Jump { host_id: id("web") }));
    let hop = host("ops@hop1.example:2200");
    assert_eq!((hop.address.as_str(), hop.port, hop.user.as_str()), ("hop1.example", 2200, "ops"));
    assert!(matches!(host("web").auth, Auth::Key { .. }));
    // An encrypted key comes in as it is; its passphrase is asked when it is used.
    let locked = v.keys.iter().find(|k| k.name == "id_locked").unwrap();
    assert!(locked.has_passphrase && !locked.passphrase_saved);
    assert!(s.export_key(&locked.id, None).unwrap().contains("OPENSSH PRIVATE KEY"));

    // The connection to far goes far <- web; web is reachable directly.
    let req = s.connect_request(&id("far"), None, None).unwrap();
    assert!(matches!(&req.route, detour_core::ssh::Route::Jump(j) if j.host.label == "web"));
    assert_eq!(req.known.as_ref().map(|k| k.endpoint.as_str()), Some("10.9.9.9:2022"));

    // A second import changes nothing.
    let again = s.ssh_import_preview_from(&ssh).unwrap();
    assert!(again.hosts.iter().filter(|h| ["far", "db", "web", "api.prod"].contains(&h.host.alias.as_str())).all(|h| h.exists));
    assert_eq!(again.known_new, 0);
    let r = s.ssh_import_from(&ssh, &["far".into(), "db".into()], true).unwrap();
    assert_eq!((r.hosts, r.keys, r.known), (0, 0, 0));
}

#[test]
fn no_config_is_an_empty_preview() {
    let ssh = temp_dir("empty").join(".ssh");
    let mut s = Store::open(temp_dir("store"));
    s.create("m").unwrap();
    let p = s.ssh_import_preview_from(&ssh).unwrap();
    assert!(!p.config_found && p.hosts.is_empty() && p.known_new == 0);
}
