//! Proxies as people pass them around: `vless://` links and WireGuard `.conf` files.

use std::collections::BTreeMap;

use crate::model::{ProxyEntry, ProxyKind, VlessSecurity, VlessTransport};
use crate::{Error, Result};

fn bad(what: &str) -> Error {
    Error::Proxy { reason: format!("unsupported:{what}") }
}

fn invalid() -> Error {
    Error::Proxy { reason: "protocol".into() }
}

/// Percent-decoding of a URL part (`+` stays as it is: links use %20).
fn unescape(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Ok(v) = u8::from_str_radix(std::str::from_utf8(&b[i + 1..i + 3]).unwrap_or(""), 16)
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn escape(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// A proxy read from a `vless://` link or a WireGuard config, without an id.
pub fn parse(text: &str) -> Result<ProxyEntry> {
    let t = text.trim();
    if t.len() > 6 && t[..7].eq_ignore_ascii_case("vless:/") {
        return vless(t);
    }
    if t.contains("[Interface]") || t.contains("[interface]") {
        return wireguard(t);
    }
    Err(invalid())
}

fn vless(link: &str) -> Result<ProxyEntry> {
    let rest = &link["vless://".len()..];
    let (rest, name) = rest.split_once('#').map(|(r, n)| (r, unescape(n))).unwrap_or((rest, String::new()));
    let (main, query) = rest.split_once('?').unwrap_or((rest, ""));
    let main = main.trim_end_matches('/');
    let (uuid, at) = main.rsplit_once('@').ok_or_else(invalid)?;
    let (address, port) = if let Some(v6) = at.strip_prefix('[') {
        let (a, p) = v6.split_once("]:").ok_or_else(invalid)?;
        (a.to_string(), p)
    } else {
        let (a, p) = at.rsplit_once(':').ok_or_else(invalid)?;
        (a.to_string(), p)
    };
    let port: u16 = port.parse().map_err(|_| invalid())?;
    let mut q: BTreeMap<String, String> = query.split('&').filter(|p| !p.is_empty()).map(|p| p.split_once('=').map(|(k, v)| (k.to_string(), unescape(v))).unwrap_or((p.to_string(), String::new()))).collect();
    let mut take = |k: &str| q.remove(k).unwrap_or_default();

    let encryption = take("encryption").trim().to_string();
    let encryption = if super::encryption::parse(&encryption)?.is_some() { encryption } else { String::new() };
    let security = match take("security").as_str() {
        "" | "none" => VlessSecurity::None,
        "tls" => VlessSecurity::Tls,
        "reality" => VlessSecurity::Reality,
        other => return Err(bad(other)),
    };
    let transport = match take("type").as_str() {
        "" | "tcp" | "raw" => VlessTransport::Tcp,
        "xhttp" | "splithttp" => VlessTransport::Xhttp,
        other => return Err(bad(other)),
    };
    let header = take("headerType");
    if !header.is_empty() && header != "none" {
        return Err(bad(&format!("headerType={header}")));
    }
    let flow = take("flow");
    let flow = if flow.starts_with("xtls-rprx-vision") { "xtls-rprx-vision".to_string() } else if flow.is_empty() { flow } else { return Err(bad(&flow)) };
    let alpn: Vec<String> = take("alpn").split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect();
    if transport == VlessTransport::Xhttp && alpn.len() == 1 && alpn[0] == "h3" {
        return Err(bad("HTTP/3"));
    }
    let kind = ProxyKind::Vless {
        address,
        port,
        security,
        sni: take("sni"),
        alpn,
        public_key: take("pbk"),
        short_id: take("sid"),
        pinned_cert: take("pcs"),
        transport,
        flow,
        encryption,
        path: take("path"),
        host: take("host"),
        mode: take("mode"),
        extra: take("extra"),
        params: q,
    };
    let name = if name.is_empty() { at.to_string() } else { name };
    Ok(ProxyEntry { id: String::new(), name, kind, password: None, private_key: None, preshared_key: None, uuid: Some(unescape(uuid)) })
}

/// A `vless://` link of a proxy, with its user id.
pub fn vless_link(p: &ProxyEntry) -> Option<String> {
    let ProxyKind::Vless { address, port, security, sni, alpn, public_key, short_id, pinned_cert, transport, flow, encryption, path, host, mode, extra, params } = &p.kind else { return None };
    let mut q: Vec<(String, String)> = vec![("encryption".into(), if encryption.is_empty() { "none".into() } else { encryption.clone() })];
    let mut put = |k: &str, v: &str| {
        if !v.is_empty() {
            q.push((k.to_string(), v.to_string()));
        }
    };
    put("security", match security {
        VlessSecurity::None => "none",
        VlessSecurity::Tls => "tls",
        VlessSecurity::Reality => "reality",
    });
    put("type", match transport {
        VlessTransport::Tcp => "tcp",
        VlessTransport::Xhttp => "xhttp",
    });
    put("flow", flow);
    put("sni", sni);
    put("alpn", &alpn.join(","));
    put("pbk", public_key);
    put("sid", short_id);
    put("pcs", pinned_cert);
    put("path", path);
    put("host", host);
    put("mode", mode);
    put("extra", extra);
    for (k, v) in params {
        put(k, v);
    }
    let at = if address.contains(':') { format!("[{address}]") } else { address.clone() };
    let query: Vec<String> = q.iter().map(|(k, v)| format!("{k}={}", escape(v))).collect();
    Some(format!("vless://{}@{at}:{port}?{}#{}", escape(p.uuid.as_deref().unwrap_or_default()), query.join("&"), escape(&p.name)))
}

fn wireguard(text: &str) -> Result<ProxyEntry> {
    let mut section = String::new();
    let mut peers = 0;
    let (mut private, mut addresses, mut dns, mut mtu) = (None, vec![], vec![], None);
    let (mut public, mut psk, mut endpoint, mut allowed, mut keepalive) = (None, None, None, vec![], None);
    let list = |v: &str| v.split(',').map(str::trim).filter(|s| !s.is_empty()).map(String::from).collect::<Vec<_>>();
    for line in text.lines() {
        let line = line.split('#').next().unwrap_or("").trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            section = line.trim_matches(['[', ']']).to_ascii_lowercase();
            if section == "peer" {
                peers += 1;
            }
            continue;
        }
        let Some((k, v)) = line.split_once('=') else { continue };
        let (k, v) = (k.trim().to_ascii_lowercase(), v.trim());
        match (section.as_str(), k.as_str()) {
            ("interface", "privatekey") => private = Some(v.to_string()),
            ("interface", "address") => addresses.extend(list(v)),
            // Search domains may come along: only addresses are name servers.
            ("interface", "dns") => dns.extend(list(v).into_iter().filter(|d| d.parse::<std::net::IpAddr>().is_ok())),
            ("interface", "mtu") => mtu = v.parse().ok(),
            ("peer", "publickey") => public = Some(v.to_string()),
            ("peer", "presharedkey") => psk = Some(v.to_string()),
            ("peer", "endpoint") => endpoint = Some(v.to_string()),
            ("peer", "allowedips") => allowed.extend(list(v)),
            ("peer", "persistentkeepalive") => keepalive = v.parse().ok(),
            _ => {}
        }
    }
    if peers > 1 {
        return Err(bad("several peers"));
    }
    let (Some(private), Some(public), Some(endpoint)) = (private, public, endpoint) else { return Err(invalid()) };
    let name = endpoint.rsplit_once(':').map(|(h, _)| h.trim_matches(['[', ']']).to_string()).unwrap_or_else(|| endpoint.clone());
    let kind = ProxyKind::Wireguard { endpoint, public_key: public, addresses, dns, mtu, allowed_ips: allowed, keepalive };
    Ok(ProxyEntry { id: String::new(), name, kind, password: None, private_key: Some(private), preshared_key: psk, uuid: None })
}

/// A WireGuard config of a proxy, with its private key.
pub fn wireguard_conf(p: &ProxyEntry) -> Option<String> {
    let ProxyKind::Wireguard { endpoint, public_key, addresses, dns, mtu, allowed_ips, keepalive } = &p.kind else { return None };
    let mut s = String::from("[Interface]\n");
    if let Some(k) = &p.private_key {
        s.push_str(&format!("PrivateKey = {k}\n"));
    }
    if !addresses.is_empty() {
        s.push_str(&format!("Address = {}\n", addresses.join(", ")));
    }
    if !dns.is_empty() {
        s.push_str(&format!("DNS = {}\n", dns.join(", ")));
    }
    if let Some(m) = mtu {
        s.push_str(&format!("MTU = {m}\n"));
    }
    s.push_str(&format!("\n[Peer]\nPublicKey = {public_key}\n"));
    if let Some(k) = &p.preshared_key {
        s.push_str(&format!("PresharedKey = {k}\n"));
    }
    let allowed = if allowed_ips.is_empty() { "0.0.0.0/0, ::/0".to_string() } else { allowed_ips.join(", ") };
    s.push_str(&format!("AllowedIPs = {allowed}\nEndpoint = {endpoint}\n"));
    if let Some(k) = keepalive {
        s.push_str(&format!("PersistentKeepalive = {k}\n"));
    }
    Some(s)
}

/// A proxy as text to pass on: a link or a config, secrets included.
pub fn export(p: &ProxyEntry) -> Option<String> {
    vless_link(p).or_else(|| wireguard_conf(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reality_vision_link_reads_and_writes_back() {
        let link = "vless://b831381d-6324-4d53-ad4f-8cda48b30811@nl.example.net:443?encryption=none&flow=xtls-rprx-vision&security=reality&sni=www.microsoft.com&fp=chrome&pbk=Z84J2IelR9ch3k8VtlVhhs5ycBUlXA7wHBWcBrjqnAw&sid=6ba85179e30d4fc2&spx=%2F&type=tcp&headerType=none#NL%20Amsterdam";
        let p = parse(link).unwrap();
        assert_eq!(p.name, "NL Amsterdam");
        assert_eq!(p.uuid.as_deref(), Some("b831381d-6324-4d53-ad4f-8cda48b30811"));
        let ProxyKind::Vless { address, port, security, sni, public_key, short_id, flow, transport, params, .. } = &p.kind else { panic!() };
        assert_eq!((address.as_str(), *port, *security, *transport), ("nl.example.net", 443, VlessSecurity::Reality, VlessTransport::Tcp));
        assert_eq!((sni.as_str(), short_id.as_str(), flow.as_str()), ("www.microsoft.com", "6ba85179e30d4fc2", "xtls-rprx-vision"));
        assert_eq!(public_key, "Z84J2IelR9ch3k8VtlVhhs5ycBUlXA7wHBWcBrjqnAw");
        // Not used here, kept for the link.
        assert_eq!(params.get("fp").map(String::as_str), Some("chrome"));
        assert_eq!(params.get("spx").map(String::as_str), Some("/"));
        let again = parse(&vless_link(&p).unwrap()).unwrap();
        assert_eq!(again.kind, p.kind);
        assert_eq!((again.name, again.uuid), (p.name, p.uuid));
    }

    #[test]
    fn xhttp_links_and_what_is_refused() {
        let p = parse("vless://id@[2001:db8::1]:8443?security=tls&type=xhttp&path=%2Fxh&mode=stream-one&extra=%7B%22xPaddingBytes%22%3A%22100-200%22%7D").unwrap();
        let ProxyKind::Vless { address, port, transport, path, mode, extra, .. } = &p.kind else { panic!() };
        assert_eq!((address.as_str(), *port, *transport), ("2001:db8::1", 8443, VlessTransport::Xhttp));
        assert_eq!((path.as_str(), mode.as_str(), extra.as_str()), ("/xh", "stream-one", r#"{"xPaddingBytes":"100-200"}"#));
        let refused = |l: &str| match parse(l) {
            Err(Error::Proxy { reason }) => reason,
            other => panic!("{other:?}"),
        };
        assert_eq!(refused("vless://id@h:1?type=ws"), "unsupported:ws");
        assert_eq!(refused("vless://id@h:1?encryption=mlkem768x25519plus.native.0rtt.x"), "unsupported:VLESS encryption");
        assert_eq!(refused("vless://id@h:1?type=xhttp&alpn=h3"), "unsupported:HTTP/3");
        assert_eq!(refused("vless://nothing"), "protocol");
    }

    #[test]
    fn links_with_vless_encryption_keep_it() {
        let enc = "mlkem768x25519plus.xorpub.0rtt.100-111-1111.75-0-111.50-0-3333.ptjHQxBQxTJ9MWr2cd5qWIflBSACHOevTauCQwa_71U";
        let p = parse(&format!("vless://b831381d-6324-4d53-ad4f-8cda48b30811@h:443?encryption={enc}&flow=xtls-rprx-vision&type=xhttp&path=%2F#E")).unwrap();
        let ProxyKind::Vless { encryption, flow, transport, .. } = &p.kind else { panic!() };
        assert_eq!((encryption.as_str(), flow.as_str(), *transport), (enc, "xtls-rprx-vision", VlessTransport::Xhttp));
        let again = parse(&vless_link(&p).unwrap()).unwrap();
        assert_eq!(again.kind, p.kind);
        let none = parse("vless://id@h:1?encryption=none").unwrap();
        assert!(matches!(&none.kind, ProxyKind::Vless { encryption, .. } if encryption.is_empty()));
    }

    #[test]
    fn a_wireguard_config_reads_and_writes_back() {
        let conf = "[Interface]\nPrivateKey = yAnz5TF+lXXJte14tji3zlMNq+hd2rYUIgJBgB3fBmk=\nAddress = 10.66.0.12/32, fd00::12/128\nDNS = 10.66.0.1, corp.local\nMTU = 1380\n\n[Peer]\nPublicKey = xTIBA5rboUvnH4htodjb6e697QjLERt1NAB4mZqp8Dg=\nPresharedKey = FpCyhws9cxwWoV4xELtfJvjJN+zQVRPISllRWgeopVE=\nAllowedIPs = 10.0.0.0/8\nEndpoint = vpn.acme.io:51820\nPersistentKeepalive = 25 # NAT\n";
        let p = parse(conf).unwrap();
        assert_eq!(p.name, "vpn.acme.io");
        let ProxyKind::Wireguard { endpoint, addresses, dns, mtu, keepalive, .. } = &p.kind else { panic!() };
        assert_eq!((endpoint.as_str(), addresses.len(), dns.as_slice(), *mtu, *keepalive), ("vpn.acme.io:51820", 2, ["10.66.0.1".to_string()].as_slice(), Some(1380), Some(25)));
        assert!(p.preshared_key.is_some());
        let again = parse(&wireguard_conf(&p).unwrap()).unwrap();
        assert_eq!((again.kind, again.private_key), (p.kind, p.private_key));
        assert!(matches!(parse("[Interface]\nPrivateKey=a\n[Peer]\nPublicKey=b\nEndpoint=c:1\n[Peer]\nPublicKey=d\n"), Err(Error::Proxy { reason }) if reason == "unsupported:several peers"));
    }
}
