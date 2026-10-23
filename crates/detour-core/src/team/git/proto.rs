//! The Git smart protocol (version 0), client side, over any byte stream:
//! `git-upload-pack` for fetching and `git-receive-pack` for pushing.

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use super::object::Oid;
use super::{bad, pack};
use crate::{Error, Result};

const AGENT: &str = concat!("agent=detour/", env!("CARGO_PKG_VERSION"));
/// A pushed or fetched pack bigger than this is refused.
const MAX_PACK: usize = 512 << 20;

pub trait Stream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> Stream for T {}

/* ---------- pkt-line ---------- */

async fn flush(s: &mut (impl AsyncWrite + Unpin)) -> Result<()> {
    s.write_all(b"0000").await?;
    s.flush().await?;
    Ok(())
}

/// One packet; `None` for a flush packet.
async fn read_pkt(s: &mut (impl AsyncRead + Unpin)) -> Result<Option<Vec<u8>>> {
    let mut len = [0u8; 4];
    s.read_exact(&mut len).await.map_err(|_| bad("connection closed"))?;
    let len = usize::from_str_radix(std::str::from_utf8(&len).map_err(|_| bad("pkt-line"))?, 16).map_err(|_| bad("pkt-line"))?;
    if len == 0 {
        return Ok(None);
    }
    if !(4..=65520).contains(&len) {
        return Err(bad("pkt-line length"));
    }
    let mut data = vec![0u8; len - 4];
    s.read_exact(&mut data).await.map_err(|_| bad("connection closed"))?;
    Ok(Some(data))
}

fn trim_nl(mut d: &[u8]) -> &[u8] {
    if d.last() == Some(&b'\n') {
        d = &d[..d.len() - 1];
    }
    d
}

/* ---------- reference advertisement ---------- */

#[derive(Debug, Default)]
pub struct Advert {
    pub refs: Vec<(String, Oid)>,
    pub caps: Vec<String>,
}

impl Advert {
    /// The branch the remote's HEAD points to, else `main`, else `master`, else the first branch.
    pub fn default_branch(&self) -> Option<String> {
        let from_head = self.caps.iter().find_map(|c| c.strip_prefix("symref=HEAD:")).map(String::from);
        let has = |r: &str| self.refs.iter().any(|(n, _)| n == r);
        from_head
            .filter(|r| has(r))
            .or_else(|| ["refs/heads/main", "refs/heads/master"].into_iter().find(|r| has(r)).map(String::from))
            .or_else(|| self.refs.iter().map(|(n, _)| n).find(|n| n.starts_with("refs/heads/")).cloned())
    }

    pub fn get(&self, name: &str) -> Option<Oid> {
        self.refs.iter().find(|(n, _)| n == name).map(|(_, id)| *id)
    }
}

pub async fn read_advert(s: &mut (impl AsyncRead + Unpin)) -> Result<Advert> {
    let mut a = Advert::default();
    let mut first = true;
    while let Some(line) = read_pkt(s).await? {
        let line = trim_nl(&line);
        if line.starts_with(b"ERR ") {
            return Err(Error::Git { message: String::from_utf8_lossy(&line[4..]).into() });
        }
        let (refpart, caps) = match line.iter().position(|&b| b == 0) {
            Some(i) => (&line[..i], Some(&line[i + 1..])),
            None => (line, None),
        };
        if first {
            if let Some(caps) = caps {
                a.caps = String::from_utf8_lossy(caps).split(' ').filter(|c| !c.is_empty()).map(String::from).collect();
            }
            first = false;
        }
        let text = std::str::from_utf8(refpart).map_err(|_| bad("ref advertisement"))?;
        let (id, name) = text.split_once(' ').ok_or_else(|| bad("ref advertisement"))?;
        let id = Oid::from_hex(id).ok_or_else(|| bad("ref advertisement"))?;
        // An empty repository advertises only its capabilities.
        if name != "capabilities^{}" && !name.ends_with("^{}") {
            a.refs.push((name.to_string(), id));
        }
    }
    Ok(a)
}

/* ---------- side band ---------- */

/// Reads side-band packets until a flush, returning channel 1 and failing on channel 3.
async fn read_sideband(s: &mut (impl AsyncRead + Unpin), first: Option<Vec<u8>>) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let mut next = first;
    loop {
        let pkt = match next.take() {
            Some(p) => p,
            None => match read_pkt(s).await? {
                Some(p) => p,
                None => return Ok(out),
            },
        };
        match pkt.first() {
            Some(1) => {
                out.extend_from_slice(&pkt[1..]);
                if out.len() > MAX_PACK {
                    return Err(bad("pack too large"));
                }
            }
            Some(2) => {}
            Some(3) => return Err(Error::Git { message: String::from_utf8_lossy(trim_nl(&pkt[1..])).into() }),
            _ => return Err(bad("side band")),
        }
    }
}

/* ---------- fetch ---------- */

pub struct Fetched {
    pub branch: Option<String>,
    pub head: Option<Oid>,
    /// Empty when the head was already known.
    pub objects: pack::Objects,
}

/// What to ask the server for, given its advertisement; `request` is `None` when there is nothing to fetch.
pub struct FetchPlan {
    pub branch: Option<String>,
    pub head: Option<Oid>,
    pub request: Option<Vec<u8>>,
}

fn pkt(out: &mut Vec<u8>, data: &[u8]) {
    out.extend_from_slice(format!("{:04x}", data.len() + 4).as_bytes());
    out.extend_from_slice(data);
}

pub fn plan_fetch(advert: &Advert, haves: &[Oid]) -> Result<FetchPlan> {
    let branch = advert.default_branch();
    let head = branch.as_deref().and_then(|b| advert.get(b));
    let Some(want) = head.filter(|h| !haves.contains(h)) else {
        return Ok(FetchPlan { branch, head, request: None });
    };
    if !advert.caps.iter().any(|c| c == "side-band-64k") {
        return Err(bad("server without side-band-64k"));
    }
    let mut req = Vec::new();
    let caps = ["side-band-64k", "ofs-delta", AGENT].join(" ");
    pkt(&mut req, format!("want {want} {caps}\n").as_bytes());
    req.extend_from_slice(b"0000");
    for h in haves.iter().take(64) {
        pkt(&mut req, format!("have {h}\n").as_bytes());
    }
    pkt(&mut req, b"done\n");
    Ok(FetchPlan { branch, head, request: Some(req) })
}

/// ACK / NAK lines, then the pack on side band 1.
pub async fn read_fetch_response(s: &mut (impl AsyncRead + Unpin)) -> Result<pack::Objects> {
    let first = loop {
        match read_pkt(s).await? {
            Some(p) if p.starts_with(b"ACK ") || p.starts_with(b"NAK") => continue,
            Some(p) if p.starts_with(b"ERR ") => return Err(Error::Git { message: String::from_utf8_lossy(trim_nl(&p[4..])).into() }),
            Some(p) => break p,
            None => return Err(bad("no pack from server")),
        }
    };
    let data = read_sideband(s, Some(first)).await?;
    pack::read(&data)
}

/// Fetches the default branch over a two-way stream (SSH or a local process).
pub async fn fetch(s: &mut impl Stream, haves: &[Oid]) -> Result<Fetched> {
    let advert = read_advert(s).await?;
    let plan = plan_fetch(&advert, haves)?;
    let Some(req) = plan.request else {
        flush(s).await?;
        return Ok(Fetched { branch: plan.branch, head: plan.head, objects: Default::default() });
    };
    s.write_all(&req).await?;
    s.flush().await?;
    Ok(Fetched { branch: plan.branch, head: plan.head, objects: read_fetch_response(s).await? })
}

/// The refs a server advertises, ending the conversation right after (two-way stream).
pub async fn list_refs(s: &mut impl Stream) -> Result<Advert> {
    let advert = read_advert(s).await?;
    flush(s).await?;
    Ok(advert)
}

/* ---------- push ---------- */

pub enum Pushed {
    Done,
    /// The branch moved on the server since `old`: fetch and try again.
    Stale,
}

pub enum PushPlan {
    Stale,
    /// The request, and whether the report comes on the side band.
    Send(Vec<u8>, bool),
}

/// The update command and pack for moving `branch` from `old` (zero for a new branch) to `new`.
pub fn plan_push(advert: &Advert, branch: &str, old: Oid, new: Oid, pack: &[u8]) -> Result<PushPlan> {
    if advert.get(branch).unwrap_or(Oid::ZERO) != old {
        return Ok(PushPlan::Stale);
    }
    let sideband = advert.caps.iter().any(|c| c == "side-band-64k");
    if !advert.caps.iter().any(|c| c == "report-status") {
        return Err(bad("server without report-status"));
    }
    let caps = if sideband { format!("report-status side-band-64k {AGENT}") } else { format!("report-status {AGENT}") };
    let mut cmd = format!("{old} {new} {branch}").into_bytes();
    cmd.push(0);
    cmd.extend_from_slice(caps.as_bytes());
    cmd.push(b'\n');
    let mut req = Vec::with_capacity(pack.len() + cmd.len() + 8);
    pkt(&mut req, &cmd);
    req.extend_from_slice(b"0000");
    req.extend_from_slice(pack);
    Ok(PushPlan::Send(req, sideband))
}

pub async fn read_push_response(s: &mut (impl AsyncRead + Unpin), sideband: bool) -> Result<Pushed> {
    let report = if sideband {
        read_sideband(s, None).await?
    } else {
        let mut all = Vec::new();
        while let Some(p) = read_pkt(s).await? {
            all.extend_from_slice(format!("{:04x}", p.len() + 4).as_bytes());
            all.extend_from_slice(&p);
        }
        all
    };
    // The report is itself pkt-lines: "unpack ok", then "ok <ref>" or "ng <ref> <reason>".
    let mut r = &report[..];
    let mut lines = Vec::new();
    while let Some(p) = read_pkt(&mut r).await? {
        lines.push(String::from_utf8_lossy(trim_nl(&p)).to_string());
    }
    if !lines.iter().any(|l| l == "unpack ok") {
        return Err(Error::Git { message: lines.join("; ") });
    }
    match lines.iter().find(|l| l.starts_with("ok ") || l.starts_with("ng ")) {
        Some(l) if l.starts_with("ok ") => Ok(Pushed::Done),
        Some(l) if l.contains("non-fast-forward") || l.contains("fetch first") || l.contains("stale") => Ok(Pushed::Stale),
        Some(l) => Err(Error::Git { message: l.clone() }),
        None => Err(bad("no push report")),
    }
}

/// Pushes over a two-way stream (SSH or a local process).
pub async fn push(s: &mut impl Stream, branch: &str, old: Oid, new: Oid, pack: &[u8]) -> Result<Pushed> {
    let advert = read_advert(s).await?;
    match plan_push(&advert, branch, old, new, pack)? {
        PushPlan::Stale => {
            flush(s).await?;
            Ok(Pushed::Stale)
        }
        PushPlan::Send(req, sideband) => {
            s.write_all(&req).await?;
            s.flush().await?;
            read_push_response(s, sideband).await
        }
    }
}

/* ---------- smart HTTP ---------- */

/// The advertisement of `GET info/refs?service=…`: a service line, a flush, then the usual refs.
pub async fn read_http_advert(body: &[u8], service: &str) -> Result<Advert> {
    let mut r = body;
    let first = read_pkt(&mut r).await?.ok_or_else(|| bad("not a smart Git server"))?;
    if trim_nl(&first) != format!("# service={service}").as_bytes() {
        return Err(bad("not a smart Git server"));
    }
    // Some servers skip the flush after the service line.
    let mut rest = r;
    if rest.starts_with(b"0000") {
        rest = &rest[4..];
    }
    read_advert(&mut rest).await
}
