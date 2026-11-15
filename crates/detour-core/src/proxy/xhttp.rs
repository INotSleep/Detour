//! XHTTP: VLESS carried in ordinary-looking HTTP requests, as Xray's client sends them.
//!
//! - stream-one: one POST whose body goes up and whose response comes down (HTTP/2).
//! - stream-up: a GET for what comes down, a streaming POST for what goes up.
//! - packet-up: the same GET, and what goes up in numbered POSTs.
//!
//! Every request carries random padding (by default an `x_padding` query in a Referer
//! header) whose length the server checks. The VLESS stream itself is a local pipe whose
//! other end these requests feed.

use std::io;
use std::time::Duration;

use bytes::Bytes;
use http::{Method, Request, Uri, header};
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, Empty, Full, StreamBody};
use hyper::body::{Frame, Incoming};
use hyper_util::rt::{TokioExecutor, TokioIo};
use rand::RngExt;
use serde::Deserialize;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::sync::mpsc;

use super::{Io, failed, reach};
use crate::Result;
use crate::model::{ProxyEntry, ProxyKind, VlessSecurity};

type Body = BoxBody<Bytes, io::Error>;

/// A range like Xray's: `"100-1000"` or a number.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Range(u32, u32);

impl<'de> Deserialize<'de> for Range {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        let v = serde_json::Value::deserialize(d)?;
        let bad = || serde::de::Error::custom("range");
        let (a, b) = match &v {
            serde_json::Value::Number(n) => {
                let n = n.as_u64().ok_or_else(bad)? as u32;
                (n, n)
            }
            serde_json::Value::String(s) => match s.split_once('-') {
                Some((a, b)) => (a.trim().parse().map_err(|_| bad())?, b.trim().parse().map_err(|_| bad())?),
                None => {
                    let n = s.trim().parse().map_err(|_| bad())?;
                    (n, n)
                }
            },
            _ => return Err(bad()),
        };
        Ok(Range(a.min(b), a.max(b)))
    }
}

impl Range {
    fn pick(self) -> u32 {
        if self.0 >= self.1 { self.0 } else { rand::rng().random_range(self.0..=self.1) }
    }
}

/// The settings of a link's `extra` this client follows.
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct Extra {
    headers: std::collections::BTreeMap<String, String>,
    x_padding_bytes: Option<Range>,
    x_padding_obfs_mode: bool,
    x_padding_key: String,
    x_padding_header: String,
    x_padding_placement: String,
    x_padding_method: String,
    #[serde(rename = "uplinkHTTPMethod")]
    uplink_http_method: String,
    #[serde(rename = "sessionIDPlacement")]
    session_id_placement: String,
    #[serde(rename = "sessionIDKey")]
    session_id_key: String,
    seq_placement: String,
    seq_key: String,
    uplink_data_placement: String,
    #[serde(rename = "noGRPCHeader")]
    no_grpc_header: bool,
    sc_max_each_post_bytes: Option<Range>,
    sc_min_posts_interval_ms: Option<Range>,
    download_settings: Option<serde_json::Value>,
    // Followed from the link itself unless set here.
    host: String,
    path: String,
    mode: String,
}

/// Everything about how requests are made.
struct Plan {
    https: bool,
    authority: String,
    /// Path (with a trailing slash when ids go in it) and query.
    path: String,
    query: String,
    extra: Extra,
}

impl Plan {
    fn session_placement(&self) -> &str {
        if self.extra.session_id_placement.is_empty() { "path" } else { &self.extra.session_id_placement }
    }
    fn seq_placement(&self) -> &str {
        if self.extra.seq_placement.is_empty() { "path" } else { &self.extra.seq_placement }
    }
    fn key(&self, custom: &str, placement: &str, header: &str, other: &str) -> String {
        if !custom.is_empty() {
            return custom.to_string();
        }
        if placement == "header" { header.to_string() } else { other.to_string() }
    }

    /// A request: padding, session id and sequence number where the settings put them.
    fn request(&self, method: Method, session: &str, seq: Option<u64>, body: Body, streaming: bool) -> Result<Request<Body>> {
        let e = &self.extra;
        let scheme = if self.https { "https" } else { "http" };
        let mut path = self.path.clone();
        let mut query: Vec<String> = if self.query.is_empty() { vec![] } else { vec![self.query.clone()] };
        let mut headers: Vec<(String, String)> = Vec::new();
        let mut cookies: Vec<String> = Vec::new();
        let base_url = format!("{scheme}://{}{}{}", self.authority, path, if self.query.is_empty() { String::new() } else { format!("?{}", self.query) });

        // Padding.
        let len = e.x_padding_bytes.unwrap_or(Range(100, 1000)).pick() as usize;
        let (placement, key, pad_header, method_name) = if e.x_padding_obfs_mode {
            let placement = if e.x_padding_placement.is_empty() { "queryInHeader" } else { e.x_padding_placement.as_str() };
            let key = if e.x_padding_key.is_empty() { "x_padding" } else { e.x_padding_key.as_str() };
            let h = if e.x_padding_header.is_empty() { "X-Padding" } else { e.x_padding_header.as_str() };
            (placement, key, h, e.x_padding_method.as_str())
        } else {
            ("queryInHeader", "x_padding", "Referer", "repeat-x")
        };
        let padding = if method_name == "tokenish" { tokenish(len) } else { "X".repeat(len) };
        match placement {
            "queryInHeader" => {
                let without_query = base_url.split('?').next().unwrap_or(&base_url).to_string();
                headers.push((pad_header.to_string(), format!("{without_query}?{key}={padding}")));
            }
            "header" => headers.push((pad_header.to_string(), padding)),
            "query" => query.push(format!("{key}={padding}")),
            "cookie" => cookies.push(format!("{key}={padding}")),
            other => return Err(failed(&format!("unsupported:xPaddingPlacement {other}"))),
        }

        // Session id and sequence number.
        let mut place = |placement: &str, key: String, value: String| -> Result<()> {
            match placement {
                "path" => {
                    if !path.ends_with('/') {
                        path.push('/');
                    }
                    path.push_str(&value);
                }
                "query" => query.push(format!("{key}={value}")),
                "header" => headers.push((key, value)),
                "cookie" => cookies.push(format!("{key}={value}")),
                other => return Err(failed(&format!("unsupported:placement {other}"))),
            }
            Ok(())
        };
        if !session.is_empty() {
            let key = self.key(&e.session_id_key, self.session_placement(), "X-Session", "x_session");
            place(self.session_placement(), key, session.to_string())?;
        }
        if let Some(seq) = seq {
            let key = self.key(&e.seq_key, self.seq_placement(), "X-Seq", "x_seq");
            place(self.seq_placement(), key, seq.to_string())?;
        }

        let pq = if query.is_empty() { path } else { format!("{path}?{}", query.join("&")) };
        let uri: Uri = format!("{scheme}://{}{pq}", self.authority).parse().map_err(|_| failed("protocol"))?;
        let mut b = Request::builder().method(method).uri(uri);
        for (k, v) in browser_headers() {
            b = b.header(k, v);
        }
        for (k, v) in &e.headers {
            b = b.header(k.as_str(), v.as_str());
        }
        for (k, v) in headers {
            b = b.header(k, v);
        }
        if !cookies.is_empty() {
            b = b.header(header::COOKIE, cookies.join("; "));
        }
        if streaming && !e.no_grpc_header {
            b = b.header(header::CONTENT_TYPE, "application/grpc");
        }
        b.body(body).map_err(|_| failed("protocol"))
    }
}

/// HPACK Huffman code lengths of `0-9A-Za-z` (RFC 7541, appendix B).
fn huffman_bits(c: u8) -> usize {
    match c {
        b'0'..=b'2' | b'a' | b'c' | b'e' | b'i' | b'o' | b's' | b't' => 5,
        b'3'..=b'9' | b'A' | b'b' | b'd' | b'f' | b'g' | b'h' | b'l' | b'm' | b'n' | b'p' | b'r' | b'u' => 6,
        b'X' | b'Z' => 8,
        _ => 7,
    }
}

/// Base62 padding whose HPACK-compressed length is about `target` bytes, as Xray's "tokenish".
fn tokenish(target: usize) -> String {
    const SET: &[u8] = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz";
    let mut rng = rand::rng();
    let mut s: Vec<u8> = (0..((target as f64 / 0.8).ceil() as usize).max(1)).map(|_| SET[rng.random_range(0..SET.len())]).collect();
    let len = |s: &[u8]| s.iter().map(|c| huffman_bits(*c)).sum::<usize>().div_ceil(8);
    let mut adjust = b'X';
    for _ in 0..150 {
        let diff = len(&s) as isize - target as isize;
        if diff.abs() <= 2 {
            break;
        }
        if diff < 0 {
            s.push(adjust);
            adjust = if adjust == b'X' { b'Z' } else { b'X' };
        } else if s.len() > 1 {
            s.pop();
        }
    }
    String::from_utf8(s).unwrap()
}

/// Headers a Chrome on Windows sends with a fetch.
fn browser_headers() -> Vec<(&'static str, String)> {
    let days = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() / 86400).unwrap_or(0) as i64;
    let v = 144 + ((days - 20466 - 35).max(0) / 35);
    vec![
        ("sec-ch-ua", format!("\"Chromium\";v=\"{v}\", \"Not-A.Brand\";v=\"24\", \"Google Chrome\";v=\"{v}\"")),
        ("sec-ch-ua-mobile", "?0".into()),
        ("sec-ch-ua-platform", "\"Windows\"".into()),
        ("user-agent", format!("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/{v}.0.0.0 Safari/537.36")),
        ("accept-language", "en-US,en;q=0.9".into()),
    ]
}

/* ---------- connections ---------- */

#[derive(Clone, Copy)]
enum Http {
    One,
    Two,
}

/// How to reach the server: TCP, then TLS or REALITY.
#[derive(Clone)]
struct Dialer {
    p: ProxyEntry,
    http: Http,
}

impl Dialer {
    async fn dial(&self) -> Result<Box<dyn Io>> {
        let ProxyKind::Vless { address, port, security, sni, alpn, pinned_cert, .. } = &self.p.kind else { return Err(failed("protocol")) };
        let tcp = reach(address, *port).await?;
        let sni = if sni.is_empty() { address.as_str() } else { sni.as_str() };
        Ok(match security {
            VlessSecurity::None => Box::new(tcp),
            VlessSecurity::Tls => {
                let alpn = match self.http {
                    Http::One => vec!["http/1.1".to_string()],
                    Http::Two if alpn.is_empty() => vec!["h2".to_string(), "http/1.1".to_string()],
                    Http::Two => alpn.clone(),
                };
                Box::new(super::tls::connect(tcp, sni, &alpn, pinned_cert).await?)
            }
            VlessSecurity::Reality => Box::new(super::reality::connect(tcp, &self.p, sni).await?),
        })
    }

    /// A connection to send requests on (HTTP/2: any number at once; 1.1: one after another).
    async fn connect(&self) -> Result<Sender> {
        let io = TokioIo::new(self.dial().await?);
        match self.http {
            Http::Two => {
                let (send, conn) = hyper::client::conn::http2::handshake(TokioExecutor::new(), io).await.map_err(|_| failed("protocol"))?;
                tokio::spawn(conn);
                Ok(Sender::Two(send))
            }
            Http::One => {
                let (send, conn) = hyper::client::conn::http1::handshake(io).await.map_err(|_| failed("protocol"))?;
                tokio::spawn(conn);
                Ok(Sender::One(send))
            }
        }
    }
}

enum Sender {
    One(hyper::client::conn::http1::SendRequest<Body>),
    Two(hyper::client::conn::http2::SendRequest<Body>),
}

impl Sender {
    async fn send(&mut self, req: Request<Body>) -> Result<hyper::Response<Incoming>> {
        let res = match self {
            Sender::One(s) => {
                s.ready().await.map_err(|_| failed("unreachable"))?;
                // HTTP/1.1: the path alone in the request line, the server's name in Host.
                let (mut parts, body) = req.into_parts();
                let host = parts.uri.authority().map(|a| a.to_string()).unwrap_or_default();
                parts.uri = parts.uri.path_and_query().map(|p| p.as_str()).unwrap_or("/").parse().map_err(|_| failed("protocol"))?;
                parts.headers.insert(header::HOST, host.parse().map_err(|_| failed("protocol"))?);
                s.send_request(Request::from_parts(parts, body)).await
            }
            Sender::Two(s) => {
                s.ready().await.map_err(|_| failed("unreachable"))?;
                s.send_request(req).await
            }
        };
        let res = res.map_err(|_| failed("unreachable"))?;
        // A wrong path or settings: the server says so with 4xx.
        if res.status() != http::StatusCode::OK {
            return Err(failed(if res.status().is_client_error() { "refused" } else { "protocol" }));
        }
        Ok(res)
    }
}

fn empty() -> Body {
    Empty::new().map_err(|never| match never {}).boxed()
}

/// A request body fed by a channel.
fn channel_body(rx: mpsc::Receiver<Bytes>) -> Body {
    let s = futures::stream::unfold(rx, |mut rx| async move { rx.recv().await.map(|b| (Ok::<_, io::Error>(Frame::data(b)), rx)) });
    StreamBody::new(s).boxed()
}

/// Copies a response body into the pipe until either ends.
fn download(mut body: Incoming, mut to: tokio::io::WriteHalf<tokio::io::DuplexStream>) {
    tokio::spawn(async move {
        while let Some(Ok(frame)) = body.frame().await {
            if let Ok(data) = frame.into_data()
                && to.write_all(&data).await.is_err()
            {
                break;
            }
        }
        let _ = to.shutdown().await;
    });
}

fn session_id() -> String {
    let b: [u8; 16] = rand::random();
    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!("{}-{}-4{}-{}-{}", &h[..8], &h[8..12], &h[13..16], &h[16..20], &h[20..32])
}

/// A VLESS stream to `target:port` through an XHTTP proxy.
pub async fn connect(p: &ProxyEntry) -> Result<tokio::io::DuplexStream> {
    let ProxyKind::Vless { address, security, sni, alpn, host, path, mode, extra, .. } = &p.kind else { return Err(failed("protocol")) };
    let extra: Extra = if extra.trim().is_empty() { Extra::default() } else { serde_json::from_str(extra).map_err(|_| failed("unsupported:extra"))? };
    if extra.download_settings.is_some() {
        return Err(failed("unsupported:downloadSettings"));
    }
    if !extra.uplink_data_placement.is_empty() && !matches!(extra.uplink_data_placement.as_str(), "body" | "auto") {
        return Err(failed(&format!("unsupported:uplinkDataPlacement {}", extra.uplink_data_placement)));
    }
    let http = match security {
        VlessSecurity::Reality => Http::Two,
        VlessSecurity::None => Http::One,
        VlessSecurity::Tls if alpn.len() == 1 && alpn[0] == "http/1.1" => Http::One,
        VlessSecurity::Tls if alpn.len() == 1 && alpn[0] == "h3" => return Err(failed("unsupported:HTTP/3")),
        VlessSecurity::Tls => Http::Two,
    };
    let mode = [extra.mode.as_str(), mode.as_str()].into_iter().find(|m| !m.is_empty()).unwrap_or("auto").to_string();
    let mode = match mode.as_str() {
        "auto" | "" if *security == VlessSecurity::Reality => "stream-one".to_string(),
        "auto" | "" => "packet-up".to_string(),
        "packet-up" | "stream-up" | "stream-one" => mode,
        other => return Err(failed(&format!("unsupported:mode {other}"))),
    };
    // Uploading in one stream while the answer comes needs HTTP/2 (Xray's server can't over 1.1).
    if mode != "packet-up" && matches!(http, Http::One) {
        return Err(failed(&format!("unsupported:{mode} over HTTP/1.1")));
    }

    let host = [extra.host.as_str(), host.as_str(), sni.as_str(), address.as_str()].into_iter().find(|h| !h.is_empty()).unwrap_or_default().to_string();
    let raw_path = if extra.path.is_empty() { path.clone() } else { extra.path.clone() };
    let (path_part, query) = raw_path.split_once('?').map(|(a, b)| (a.to_string(), b.to_string())).unwrap_or((raw_path.clone(), String::new()));
    let mut plan = Plan { https: *security != VlessSecurity::None, authority: host, path: if path_part.starts_with('/') { path_part } else { format!("/{path_part}") }, query, extra };
    if (plan.session_placement() == "path" || plan.seq_placement() == "path") && !plan.path.ends_with('/') {
        plan.path.push('/');
    }
    let upload_method = if plan.extra.uplink_http_method.is_empty() { Method::POST } else { Method::from_bytes(plan.extra.uplink_http_method.as_bytes()).map_err(|_| failed("protocol"))? };

    let dialer = Dialer { p: p.clone(), http };
    let (mine, theirs) = tokio::io::duplex(256 * 1024);
    let (mut up, down) = tokio::io::split(theirs);

    match mode.as_str() {
        "stream-one" => {
            let (tx, rx) = mpsc::channel::<Bytes>(16);
            let mut send = dialer.connect().await?;
            let req = plan.request(upload_method, "", None, channel_body(rx), true)?;
            pump_stream(up, tx);
            download(send.send(req).await?.into_body(), down);
        }
        _ => {
            let session = session_id();
            let mut downlink = dialer.connect().await?;
            let res = downlink.send(plan.request(Method::GET, &session, None, empty(), false)?).await?;
            download(res.into_body(), down);
            if mode == "stream-up" {
                let (tx, rx) = mpsc::channel::<Bytes>(16);
                let mut send = downlink;
                let req = plan.request(upload_method, &session, None, channel_body(rx), true)?;
                pump_stream(up, tx);
                // The response is only filler; it ends when the upload does.
                tokio::spawn(async move {
                    if let Ok(res) = send.send(req).await {
                        let mut body = res.into_body();
                        while let Some(Ok(_)) = body.frame().await {}
                    }
                });
            } else {
                let mut posts = match http {
                    Http::Two => downlink,
                    Http::One => dialer.connect().await?,
                };
                let max = plan.extra.sc_max_each_post_bytes.unwrap_or(Range(1_000_000, 1_000_000)).pick().max(1) as usize;
                let gap = plan.extra.sc_min_posts_interval_ms.unwrap_or(Range(30, 30));
                tokio::spawn(async move {
                    let mut seq = 0u64;
                    let mut last = tokio::time::Instant::now() - Duration::from_secs(1);
                    let mut buf = vec![0u8; max.min(1 << 20)];
                    loop {
                        // What there is to send now, up to the limit.
                        let n = match up.read(&mut buf).await {
                            Ok(0) | Err(_) => break,
                            Ok(n) => n,
                        };
                        let mut chunk = buf[..n].to_vec();
                        let wait = Duration::from_millis(gap.pick() as u64);
                        tokio::time::sleep_until(last + wait).await;
                        // Whatever arrived while waiting goes in the same post.
                        while chunk.len() < max {
                            let mut more = vec![0u8; (max - chunk.len()).min(64 * 1024)];
                            match tokio::time::timeout(Duration::ZERO, up.read(&mut more)).await {
                                Ok(Ok(m)) if m > 0 => chunk.extend_from_slice(&more[..m]),
                                _ => break,
                            }
                        }
                        last = tokio::time::Instant::now();
                        let body = Full::new(Bytes::from(chunk)).map_err(|never| match never {}).boxed();
                        let Ok(req) = plan.request(upload_method.clone(), &session, Some(seq), body, false) else { break };
                        seq += 1;
                        match posts.send(req).await {
                            Ok(res) => {
                                let mut body = res.into_body();
                                while let Some(Ok(_)) = body.frame().await {}
                            }
                            Err(_) => break,
                        }
                    }
                });
            }
        }
    }
    Ok(mine)
}

/// Feeds what the VLESS stream writes into a streaming request body.
fn pump_stream(mut up: tokio::io::ReadHalf<tokio::io::DuplexStream>, tx: mpsc::Sender<Bytes>) {
    tokio::spawn(async move {
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            match up.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send(Bytes::copy_from_slice(&buf[..n])).await.is_err() {
                        break;
                    }
                }
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(extra: &str) -> Plan {
        Plan { https: true, authority: "cdn.example".into(), path: "/xh/".into(), query: String::new(), extra: serde_json::from_str(extra).unwrap() }
    }

    #[test]
    fn requests_carry_padding_session_and_seq_where_xray_looks() {
        let req = plan("{}").request(Method::POST, "sid", Some(7), empty(), false).unwrap();
        assert_eq!(req.uri().path(), "/xh/sid/7");
        let referer = req.headers()["referer"].to_str().unwrap();
        let pad = referer.strip_prefix("https://cdn.example/xh/?x_padding=").unwrap();
        assert!((100..=1000).contains(&pad.len()) && pad.bytes().all(|b| b == b'X'), "{referer}");

        let req = plan(r#"{"xPaddingObfsMode":true,"xPaddingPlacement":"header","xPaddingMethod":"tokenish","sessionIDPlacement":"header","seqPlacement":"query","xPaddingBytes":"300-300"}"#).request(Method::POST, "sid", Some(3), empty(), true).unwrap();
        assert_eq!(req.uri().path(), "/xh/");
        assert_eq!(req.uri().query(), Some("x_seq=3"));
        assert_eq!(req.headers()["x-session"], "sid");
        assert_eq!(req.headers()["content-type"], "application/grpc");
        let pad = req.headers()["x-padding"].to_str().unwrap();
        let bytes = pad.bytes().map(huffman_bits).sum::<usize>().div_ceil(8);
        assert!(bytes.abs_diff(300) <= 2, "{bytes}");
    }

    #[test]
    fn ranges_read_like_xray() {
        let e: Extra = serde_json::from_str(r#"{"xPaddingBytes":"200-50","scMaxEachPostBytes":500000}"#).unwrap();
        assert_eq!(e.x_padding_bytes, Some(Range(50, 200)));
        assert_eq!(e.sc_max_each_post_bytes, Some(Range(500000, 500000)));
    }
}
