//! Moving over from other SSH clients: PuTTY sessions with a .ppk key, forwards and a proxy,
//! MobaXterm bookmarks through a gateway.

use std::path::PathBuf;

use detour_core::foreign::Client;
use detour_core::model::{Auth, Proxy, TunnelKind};
use detour_core::store::Store;

fn temp_dir(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let d = std::env::temp_dir().join(format!("detour-foreign-{tag}-{}-{nanos}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}

const PPK: &str = "PuTTY-User-Key-File-3: ssh-ed25519
Encryption: none
Comment: user@example.com
Public-Lines: 2
AAAAC3NzaC1lZDI1NTE5AAAAILM+rvN+ot98qgEN796jTiQfZfG1KaT0PtFDJ/XF
Sqti
Private-Lines: 1
AAAAILYGwiLRDBba4WxwpNRRc0cuxhfgXGVpINJuVsCPtZHt
Private-MAC: 94140d0344fad6aa1bf7b71e9c93db11ccac8a232f8a51e11c024869d608c82d
";

#[test]
fn putty_sessions_come_with_their_key_forwards_and_proxy() {
    let dir = temp_dir("putty");
    let key = dir.join("web.ppk");
    std::fs::write(&key, PPK).unwrap();
    // In a .reg file a backslash in a value is written twice.
    let k = key.display().to_string().replace('\\', r"\\");
    let reg = format!(
        r#"Windows Registry Editor Version 5.00

[HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\Sessions\web]
"HostName"="10.0.0.5"
"PortNumber"=dword:00000016
"UserName"="deploy"
"Protocol"="ssh"
"PublicKeyFile"="{k}"
"PortForwardings"="L8080=localhost:80,D1080"

[HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\Sessions\behind]
"HostName"="10.1.0.9"
"PortNumber"=dword:00000016
"Protocol"="ssh"
"ProxyMethod"=dword:00000002
"ProxyHost"="proxy.lan"
"ProxyPort"=dword:00000438
"#
    )
    .replace('\n', "\r\n");
    // .reg files are UTF-16 with a byte order mark.
    let mut bytes = vec![0xFF, 0xFE];
    bytes.extend(reg.encode_utf16().flat_map(u16::to_le_bytes));
    let file = dir.join("putty.reg");
    std::fs::write(&file, bytes).unwrap();

    let mut s = Store::open(temp_dir("store"));
    s.create("m").unwrap();
    let p = s.foreign_preview(Client::Putty, Some(&file)).unwrap();
    assert_eq!(p.hosts.len(), 2);
    assert!(p.hosts.iter().all(|h| !h.exists && !h.key_missing));

    let r = s.foreign_import(Client::Putty, Some(&file), &["web".into(), "behind".into()]).unwrap();
    assert_eq!((r.hosts, r.keys, r.tunnels), (2, 1, 2));
    let v = s.view().unwrap();
    let web = v.hosts.iter().find(|h| h.label == "web").unwrap();
    assert_eq!((web.address.as_str(), web.user.as_str(), web.group.as_str()), ("10.0.0.5", "deploy", "PuTTY"));
    let Auth::Key { key_id } = &web.auth else { panic!("{:?}", web.auth) };
    assert!(s.export_key(key_id, None).unwrap().contains("OPENSSH PRIVATE KEY"), "a .ppk key is kept as OpenSSH");
    let kinds: Vec<_> = v.tunnels.iter().map(|t| (t.kind, t.bind_port, t.via_host_id == web.id)).collect();
    assert_eq!(kinds, [(TunnelKind::Local, 8080, true), (TunnelKind::Dynamic, 1080, true)]);
    let behind = v.hosts.iter().find(|h| h.label == "behind").unwrap();
    assert!(matches!(&behind.proxy, Some(Proxy::Socks5 { address, port: 1080, .. }) if address == "proxy.lan"));

    // Again: nothing new.
    let p = s.foreign_preview(Client::Putty, Some(&file)).unwrap();
    assert!(p.hosts.iter().all(|h| h.exists));
    let r = s.foreign_import(Client::Putty, Some(&file), &["web".into()]).unwrap();
    assert_eq!(r.hosts, 0);
}

#[test]
fn mobaxterm_bookmarks_go_through_their_gateway() {
    let dir = temp_dir("moba");
    let ini = dir.join("MobaXterm.ini");
    std::fs::write(&ini, "[Bookmarks_1]\r\nSubRep=Prod\r\nImgNum=41\r\ndb= #109#0%db.lan%22%pg%%-1%-1%%gw.lan%2222%ops%0%0%0%%%-1%0%0%0%%1080%%0%0%1#MobaFont%10#0# #-1\r\n").unwrap();
    let mut s = Store::open(temp_dir("store"));
    s.create("m").unwrap();
    let r = s.foreign_import(Client::Mobaxterm, Some(&ini), &["Prod/db".into()]).unwrap();
    assert_eq!(r.hosts, 2, "the gateway comes as a host of its own");
    let v = s.view().unwrap();
    let db = v.hosts.iter().find(|h| h.label == "db").unwrap();
    let gw = v.hosts.iter().find(|h| h.address == "gw.lan").unwrap();
    assert_eq!((gw.port, gw.user.as_str(), db.group.as_str()), (2222, "ops", "Prod"));
    assert_eq!(db.proxy, Some(Proxy::Jump { host_id: gw.id.clone() }));
}
