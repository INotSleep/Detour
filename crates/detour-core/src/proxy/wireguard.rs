//! A WireGuard peer in user space: only Detour's own connections go through it.
//!
//! One tunnel per proxy runs as a task: a UDP socket to the peer, boringtun for
//! the WireGuard protocol, and smoltcp as the TCP/IP stack inside the tunnel.
//! Connections through it are handed out as byte streams; the task moves their
//! bytes between those streams and smoltcp's sockets. A tunnel with no
//! connections for a while closes.

use std::collections::{HashMap, VecDeque};
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use boringtun::noise::{Tunn, TunnResult};
use boringtun::x25519::{PublicKey, StaticSecret};
use smoltcp::iface::{Config, Interface, SocketHandle, SocketSet};
use smoltcp::phy::{Device, DeviceCapabilities, Medium, RxToken, TxToken};
use smoltcp::socket::{dns, tcp};
use smoltcp::time::Instant;
use smoltcp::wire::{HardwareAddress, IpAddress, IpCidr};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::UdpSocket;
use tokio::sync::{Notify, mpsc, oneshot};

use super::{Stream, failed};
use crate::model::{ProxyEntry, ProxyKind};
use crate::{Error, Result};

/// Bytes a connection holds each way before the other side has to wait.
const BUFFER: usize = 256 * 1024;
/// How long a tunnel with no connections stays up.
const IDLE: Duration = Duration::from_secs(120);
/// How long the first handshake may take before connections through it fail.
const HANDSHAKE: Duration = Duration::from_secs(10);
const MTU: usize = 1420;
/// Room for a WireGuard packet around the largest IP packet.
const PACKET: usize = 65536 + 64;

/// What a tunnel needs, read from a proxy.
struct Settings {
    private: StaticSecret,
    peer: PublicKey,
    preshared: Option<[u8; 32]>,
    endpoint: String,
    addresses: Vec<IpCidr>,
    dns: Vec<IpAddress>,
    mtu: usize,
    keepalive: Option<u16>,
}

fn key(text: &str) -> Result<[u8; 32]> {
    B64.decode(text.trim()).ok().and_then(|k| <[u8; 32]>::try_from(k).ok()).ok_or_else(|| failed("protocol"))
}

fn settings(p: &ProxyEntry) -> Result<Settings> {
    let ProxyKind::Wireguard { endpoint, public_key, addresses, dns, mtu, keepalive, .. } = &p.kind else { return Err(failed("protocol")) };
    let private = key(p.private_key.as_deref().ok_or_else(|| failed("auth"))?)?;
    let addresses: Vec<IpCidr> = addresses
        .iter()
        .map(|a| {
            let a = a.trim();
            let (ip, len) = a.split_once('/').map(|(ip, l)| (ip, l.parse().ok())).unwrap_or((a, None));
            let ip: IpAddr = ip.parse().map_err(|_| failed("protocol"))?;
            let len = len.unwrap_or(if ip.is_ipv4() { 32 } else { 128 });
            Ok(IpCidr::new(ip.into(), len))
        })
        .collect::<Result<_>>()?;
    if addresses.is_empty() {
        return Err(failed("protocol"));
    }
    Ok(Settings {
        private: StaticSecret::from(private),
        peer: PublicKey::from(key(public_key)?),
        preshared: p.preshared_key.as_deref().map(key).transpose()?,
        endpoint: endpoint.clone(),
        addresses,
        dns: dns.iter().filter_map(|d| d.trim().parse::<IpAddr>().ok()).map(Into::into).collect(),
        mtu: mtu.map(usize::from).filter(|m| *m >= 1280).unwrap_or(MTU),
        // Connections sit idle between commands: keep the peer's NAT entry open.
        keepalive: keepalive.or(Some(25)),
    })
}

/* ---------- the tunnels running ---------- */

enum Cmd {
    Connect { target: String, port: u16, reply: oneshot::Sender<Result<Conn>> },
    /// Answers once the handshake is done.
    Handshake { reply: oneshot::Sender<Result<()>> },
}

/// Tunnels by proxy and settings: a changed proxy gets a new one.
static TUNNELS: Mutex<Option<HashMap<String, mpsc::UnboundedSender<Cmd>>>> = Mutex::new(None);

fn tunnel_key(p: &ProxyEntry) -> String {
    use sha2::Digest;
    let h = sha2::Sha256::digest(serde_json::to_vec(p).unwrap_or_default());
    h.iter().take(16).map(|b| format!("{b:02x}")).collect()
}

/// The running tunnel of a proxy, started if needed.
async fn tunnel(p: &ProxyEntry) -> Result<mpsc::UnboundedSender<Cmd>> {
    let k = tunnel_key(p);
    if let Some(tx) = TUNNELS.lock().unwrap().get_or_insert_with(HashMap::new).get(&k).filter(|tx| !tx.is_closed()) {
        return Ok(tx.clone());
    }
    let s = settings(p)?;
    let peer = tokio::net::lookup_host(&s.endpoint).await.ok().and_then(|mut a| a.next()).ok_or_else(|| failed("unreachable"))?;
    let udp = udp_socket(peer).map_err(|_| failed("unreachable"))?;
    udp.connect(peer).await.map_err(|_| failed("unreachable"))?;
    let (tx, rx) = mpsc::unbounded_channel();
    let mut map = TUNNELS.lock().unwrap();
    let map = map.get_or_insert_with(HashMap::new);
    // Another connection may have started it meanwhile.
    if let Some(tx) = map.get(&k).filter(|tx| !tx.is_closed()) {
        return Ok(tx.clone());
    }
    map.insert(k.clone(), tx.clone());
    tokio::spawn(async move {
        Pump::new(s, udp).run(rx).await;
        let mut map = TUNNELS.lock().unwrap();
        if let Some(m) = map.as_mut() {
            m.remove(&k);
        }
    });
    Ok(tx)
}

/// A UDP socket with room for bursts: a full TCP window arrives at once, and a
/// dropped datagram costs a retransmission.
fn udp_socket(peer: SocketAddr) -> io::Result<UdpSocket> {
    use socket2::{Domain, Protocol, Socket, Type};
    let s = Socket::new(Domain::for_address(peer), Type::DGRAM, Some(Protocol::UDP))?;
    let _ = s.set_recv_buffer_size(4 << 20);
    let _ = s.set_send_buffer_size(4 << 20);
    s.set_nonblocking(true)?;
    let any: SocketAddr = if peer.is_ipv4() { ([0, 0, 0, 0], 0).into() } else { ([0u16; 8], 0).into() };
    s.bind(&any.into())?;
    UdpSocket::from_std(s.into())
}

/// A TCP connection to `target:port` through the proxy's tunnel.
pub async fn connect(p: &ProxyEntry, target: &str, port: u16) -> Result<Stream> {
    let tx = tunnel(p).await?;
    let (reply, rx) = oneshot::channel();
    tx.send(Cmd::Connect { target: target.to_string(), port, reply }).map_err(|_| failed("unreachable"))?;
    let conn = rx.await.map_err(|_| failed("unreachable"))??;
    Ok(Box::new(conn))
}

/// Waits for the proxy's handshake: whether its server answers to these keys.
pub async fn handshake(p: &ProxyEntry) -> Result<()> {
    let tx = tunnel(p).await?;
    let (reply, rx) = oneshot::channel();
    tx.send(Cmd::Handshake { reply }).map_err(|_| failed("unreachable"))?;
    rx.await.map_err(|_| failed("unreachable"))?
}

/* ---------- a connection's two ends ---------- */

#[derive(Default)]
struct Shared {
    /// To the host, waiting for room in smoltcp's socket.
    out: VecDeque<u8>,
    /// The stream is shut down or dropped: send FIN once `out` is empty.
    out_closed: bool,
    /// From the host, waiting to be read.
    inbuf: VecDeque<u8>,
    in_eof: bool,
    dropped: bool,
    reader: Option<Waker>,
    writer: Option<Waker>,
}

/// The end of a connection a client reads and writes.
struct Conn {
    shared: Arc<Mutex<Shared>>,
    pump: Arc<Notify>,
}

impl AsyncRead for Conn {
    fn poll_read(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &mut ReadBuf<'_>) -> Poll<io::Result<()>> {
        let mut s = self.shared.lock().unwrap();
        if s.inbuf.is_empty() {
            if s.in_eof {
                return Poll::Ready(Ok(()));
            }
            s.reader = Some(cx.waker().clone());
            return Poll::Pending;
        }
        let full = s.inbuf.len() >= BUFFER;
        let n = buf.remaining().min(s.inbuf.len());
        let (a, b) = s.inbuf.as_slices();
        let from_a = n.min(a.len());
        buf.put_slice(&a[..from_a]);
        buf.put_slice(&b[..n - from_a]);
        s.inbuf.drain(..n);
        drop(s);
        // There is room again: the tunnel can take more from the socket.
        if full {
            self.pump.notify_one();
        }
        Poll::Ready(Ok(()))
    }
}

impl AsyncWrite for Conn {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, data: &[u8]) -> Poll<io::Result<usize>> {
        let mut s = self.shared.lock().unwrap();
        if s.in_eof && s.out_closed {
            return Poll::Ready(Err(io::ErrorKind::BrokenPipe.into()));
        }
        let room = BUFFER.saturating_sub(s.out.len());
        if room == 0 {
            s.writer = Some(cx.waker().clone());
            return Poll::Pending;
        }
        let n = room.min(data.len());
        s.out.extend(&data[..n]);
        drop(s);
        self.pump.notify_one();
        Poll::Ready(Ok(n))
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        self.shared.lock().unwrap().out_closed = true;
        self.pump.notify_one();
        Poll::Ready(Ok(()))
    }
}

impl Drop for Conn {
    fn drop(&mut self) {
        let mut s = self.shared.lock().unwrap();
        s.out_closed = true;
        s.dropped = true;
        drop(s);
        self.pump.notify_one();
    }
}

/* ---------- smoltcp's side of the tunnel ---------- */

/// IP packets between smoltcp and WireGuard.
#[derive(Default)]
struct Packets {
    rx: VecDeque<Vec<u8>>,
    tx: VecDeque<Vec<u8>>,
    mtu: usize,
}

struct Rx(Vec<u8>);
struct Tx<'a>(&'a mut VecDeque<Vec<u8>>);

impl RxToken for Rx {
    fn consume<R, F: FnOnce(&[u8]) -> R>(self, f: F) -> R {
        f(&self.0)
    }
}

impl TxToken for Tx<'_> {
    fn consume<R, F: FnOnce(&mut [u8]) -> R>(self, len: usize, f: F) -> R {
        let mut packet = vec![0; len];
        let r = f(&mut packet);
        self.0.push_back(packet);
        r
    }
}

impl Device for Packets {
    type RxToken<'a> = Rx;
    type TxToken<'a> = Tx<'a>;

    fn receive(&mut self, _: Instant) -> Option<(Rx, Tx<'_>)> {
        let p = self.rx.pop_front()?;
        Some((Rx(p), Tx(&mut self.tx)))
    }

    fn transmit(&mut self, _: Instant) -> Option<Tx<'_>> {
        Some(Tx(&mut self.tx))
    }

    fn capabilities(&self) -> DeviceCapabilities {
        let mut c = DeviceCapabilities::default();
        c.medium = Medium::Ip;
        c.max_transmission_unit = self.mtu;
        c
    }
}

/// A connection being made or made.
struct Link {
    socket: SocketHandle,
    shared: Arc<Mutex<Shared>>,
    /// Until it is established.
    reply: Option<oneshot::Sender<Result<Conn>>>,
    fin_sent: bool,
}

/// A connection waiting for this computer to resolve its host's name.
struct PendingLocal {
    rx: oneshot::Receiver<Option<IpAddr>>,
    port: u16,
    reply: oneshot::Sender<Result<Conn>>,
}

/// A connection waiting for its host's name to resolve in the tunnel.
struct Lookup {
    query: dns::QueryHandle,
    port: u16,
    reply: oneshot::Sender<Result<Conn>>,
}

struct Pump {
    tunn: Tunn,
    udp: UdpSocket,
    dev: Packets,
    iface: Interface,
    sockets: SocketSet<'static>,
    dns: Option<SocketHandle>,
    links: Vec<Link>,
    lookups: Vec<Lookup>,
    local: Vec<PendingLocal>,
    handshake_waiting: Vec<oneshot::Sender<Result<()>>>,
    shaken: bool,
    started: std::time::Instant,
    notify: Arc<Notify>,
    next_port: u16,
    buf: Vec<u8>,
    addresses: Vec<IpCidr>,
}

impl Pump {
    fn new(s: Settings, udp: UdpSocket) -> Self {
        let tunn = Tunn::new(s.private, s.peer, s.preshared, s.keepalive, rand::random(), None);
        let mut dev = Packets { mtu: s.mtu, ..Default::default() };
        let mut iface = Interface::new(Config::new(HardwareAddress::Ip), &mut dev, Instant::now());
        iface.update_ip_addrs(|ips| {
            for a in &s.addresses {
                let _ = ips.push(*a);
            }
        });
        // Everything goes into the tunnel: the peer routes it.
        for a in &s.addresses {
            match a.address() {
                IpAddress::Ipv4(v4) => {
                    let _ = iface.routes_mut().add_default_ipv4_route(v4);
                }
                IpAddress::Ipv6(v6) => {
                    let _ = iface.routes_mut().add_default_ipv6_route(v6);
                }
            }
        }
        let mut sockets = SocketSet::new(vec![]);
        let dns = (!s.dns.is_empty()).then(|| sockets.add(dns::Socket::new(&s.dns, vec![])));
        Pump {
            tunn,
            udp,
            dev,
            iface,
            sockets,
            dns,
            links: vec![],
            lookups: vec![],
            local: vec![],
            handshake_waiting: vec![],
            shaken: false,
            started: std::time::Instant::now(),
            notify: Arc::new(Notify::new()),
            next_port: 49152 + rand::random::<u16>() % 16000,
            buf: vec![0; PACKET],
            addresses: s.addresses,
        }
    }

    async fn run(mut self, mut cmds: mpsc::UnboundedReceiver<Cmd>) {
        let mut datagram = vec![0u8; PACKET];
        self.start_handshake().await;
        let mut timers = tokio::time::interval(Duration::from_millis(250));
        let mut idle_since = Some(std::time::Instant::now());
        loop {
            let delay = self.iface.poll_delay(Instant::now(), &self.sockets).map(|d| Duration::from_micros(d.total_micros())).unwrap_or(Duration::from_secs(1));
            tokio::select! {
                r = self.udp.recv(&mut datagram) => {
                    // An error is an ICMP "unreachable" on some systems: the handshake retries.
                    if let Ok(n) = r {
                        self.incoming(&datagram[..n]).await;
                        // Everything else that arrived, before smoltcp answers.
                        for _ in 0..512 {
                            match self.udp.try_recv(&mut datagram) {
                                Ok(n) => self.incoming(&datagram[..n]).await,
                                Err(_) => break,
                            }
                        }
                    }
                }
                cmd = cmds.recv() => match cmd {
                    Some(c) => self.command(c),
                    None => break,
                },
                _ = self.notify.notified() => {}
                _ = timers.tick() => self.timers().await,
                _ = tokio::time::sleep(delay) => {}
            }
            self.step().await;
            if self.links.is_empty() && self.lookups.is_empty() && self.local.is_empty() && self.handshake_waiting.is_empty() {
                let since = *idle_since.get_or_insert_with(std::time::Instant::now);
                if since.elapsed() > IDLE {
                    break;
                }
            } else {
                idle_since = None;
            }
            if !self.shaken && self.started.elapsed() > HANDSHAKE {
                self.fail_all("handshake");
                break;
            }
        }
    }

    async fn start_handshake(&mut self) {
        if let TunnResult::WriteToNetwork(p) = self.tunn.format_handshake_initiation(&mut self.buf, false) {
            let _ = self.udp.send(p).await;
        }
    }

    /// Everything waiting gets the error.
    fn fail_all(&mut self, reason: &str) {
        for w in self.handshake_waiting.drain(..) {
            let _ = w.send(Err(failed(reason)));
        }
        for l in self.lookups.drain(..) {
            let _ = l.reply.send(Err(failed(reason)));
        }
        for l in self.local.drain(..) {
            let _ = l.reply.send(Err(failed(reason)));
        }
        for l in &mut self.links {
            if let Some(r) = l.reply.take() {
                let _ = r.send(Err(failed(reason)));
            }
            let mut s = l.shared.lock().unwrap();
            s.in_eof = true;
            if let Some(w) = s.reader.take() {
                w.wake();
            }
        }
    }

    async fn incoming(&mut self, datagram: &[u8]) {
        let mut out = Vec::new();
        match self.tunn.decapsulate(None, datagram, &mut self.buf) {
            TunnResult::WriteToNetwork(p) => {
                out.push(p.to_vec());
                // Packets queued until the handshake was done go out now.
                while let TunnResult::WriteToNetwork(p) = self.tunn.decapsulate(None, &[], &mut self.buf) {
                    out.push(p.to_vec());
                }
            }
            TunnResult::WriteToTunnelV4(p, _) | TunnResult::WriteToTunnelV6(p, _) => self.dev.rx.push_back(p.to_vec()),
            TunnResult::Done | TunnResult::Err(_) => {}
        }
        for p in out {
            let _ = self.udp.send(&p).await;
        }
        if !self.shaken && self.tunn.time_since_last_handshake().is_some() {
            self.shaken = true;
            for w in self.handshake_waiting.drain(..) {
                let _ = w.send(Ok(()));
            }
        }
    }

    async fn timers(&mut self) {
        match self.tunn.update_timers(&mut self.buf) {
            TunnResult::WriteToNetwork(p) => {
                let _ = self.udp.send(p).await;
            }
            TunnResult::Err(boringtun::noise::errors::WireGuardError::ConnectionExpired) => self.fail_all("handshake"),
            _ => {}
        }
    }

    fn command(&mut self, cmd: Cmd) {
        match cmd {
            Cmd::Handshake { reply } => {
                if self.shaken {
                    let _ = reply.send(Ok(()));
                } else {
                    self.handshake_waiting.push(reply);
                }
            }
            Cmd::Connect { target, port, reply } => {
                let bare = target.trim_start_matches('[').trim_end_matches(']');
                if let Ok(ip) = bare.parse::<IpAddr>() {
                    return self.open(ip.into(), port, reply);
                }
                match self.dns {
                    Some(h) => {
                        let socket = self.sockets.get_mut::<dns::Socket>(h);
                        let v6 = self.addresses.iter().all(|a| matches!(a.address(), IpAddress::Ipv6(_)));
                        let kind = if v6 { smoltcp::wire::DnsQueryType::Aaaa } else { smoltcp::wire::DnsQueryType::A };
                        match socket.start_query(self.iface.context(), &target, kind) {
                            Ok(query) => self.lookups.push(Lookup { query, port, reply }),
                            Err(_) => {
                                let _ = reply.send(Err(Error::Unreachable { endpoint: format!("{target}:{port}") }));
                            }
                        }
                    }
                    // No name servers in the tunnel: the name is this computer's to resolve.
                    None => {
                        let notify = self.notify.clone();
                        let (tx, rx) = oneshot::channel();
                        let wait = PendingLocal { rx, port, reply };
                        tokio::spawn(async move {
                            let ip = tokio::net::lookup_host((target.as_str(), port)).await.ok().and_then(|mut a| a.next()).map(|a| a.ip());
                            let _ = tx.send(ip);
                            notify.notify_one();
                        });
                        self.local.push(wait);
                    }
                }
            }
        }
    }

    fn open(&mut self, ip: IpAddress, port: u16, reply: oneshot::Sender<Result<Conn>>) {
        let mut socket = tcp::Socket::new(tcp::SocketBuffer::new(vec![0; BUFFER]), tcp::SocketBuffer::new(vec![0; BUFFER]));
        socket.set_nagle_enabled(false);
        socket.set_congestion_control(tcp::CongestionControl::Cubic);
        socket.set_keep_alive(Some(smoltcp::time::Duration::from_secs(30)));
        socket.set_timeout(Some(smoltcp::time::Duration::from_secs(120)));
        self.next_port = if self.next_port >= 65000 { 49152 } else { self.next_port + 1 };
        if socket.connect(self.iface.context(), (ip, port), self.next_port).is_err() {
            let _ = reply.send(Err(Error::Unreachable { endpoint: format!("{ip}:{port}") }));
            return;
        }
        let socket = self.sockets.add(socket);
        self.links.push(Link { socket, shared: Arc::default(), reply: Some(reply), fin_sent: false });
    }

    /// Lets smoltcp run, moves bytes between it and the streams, and sends its packets out.
    async fn step(&mut self) {
        for _ in 0..4 {
            self.iface.poll(Instant::now(), &mut self.dev, &mut self.sockets);
            let moved = self.shuttle();
            if !moved {
                break;
            }
        }
        self.resolve();
        while let Some(p) = self.dev.tx.pop_front() {
            if let TunnResult::WriteToNetwork(w) = self.tunn.encapsulate(&p, &mut self.buf) {
                let _ = self.udp.send(w).await;
            }
        }
    }

    /// Moves bytes between the streams and smoltcp's sockets. Returns whether any moved.
    fn shuttle(&mut self) -> bool {
        let mut moved = false;
        let notify = self.notify.clone();
        let mut gone = Vec::new();
        for (i, l) in self.links.iter_mut().enumerate() {
            let socket = self.sockets.get_mut::<tcp::Socket>(l.socket);
            if let Some(reply) = l.reply.take() {
                match socket.state() {
                    tcp::State::Established => {
                        if reply.send(Ok(Conn { shared: l.shared.clone(), pump: notify.clone() })).is_err() {
                            // Nobody waits any more (timed out): drop it.
                            socket.abort();
                            gone.push(i);
                        }
                    }
                    tcp::State::Closed => {
                        let _ = reply.send(Err(Error::Unreachable { endpoint: socket.remote_endpoint().map(|e| e.to_string()).unwrap_or_default() }));
                        gone.push(i);
                    }
                    _ if reply.is_closed() => {
                        socket.abort();
                        gone.push(i);
                    }
                    _ => l.reply = Some(reply),
                }
                continue;
            }
            let mut s = l.shared.lock().unwrap();
            // To the host.
            while socket.can_send() && !s.out.is_empty() {
                let (a, _) = s.out.as_slices();
                let n = socket.send_slice(a).unwrap_or(0);
                if n == 0 {
                    break;
                }
                s.out.drain(..n);
                moved = true;
                if let Some(w) = s.writer.take() {
                    w.wake();
                }
            }
            if s.out_closed && s.out.is_empty() && !l.fin_sent {
                socket.close();
                l.fin_sent = true;
                moved = true;
            }
            // From the host.
            let mut chunk = [0u8; 16 * 1024];
            while socket.can_recv() && s.inbuf.len() < BUFFER {
                let room = (BUFFER - s.inbuf.len()).min(chunk.len());
                let n = socket.recv_slice(&mut chunk[..room]).unwrap_or(0);
                if n == 0 {
                    break;
                }
                s.inbuf.extend(&chunk[..n]);
                moved = true;
                if let Some(w) = s.reader.take() {
                    w.wake();
                }
            }
            if !socket.may_recv() && !s.in_eof && !socket.can_recv() {
                s.in_eof = true;
                if let Some(w) = s.reader.take() {
                    w.wake();
                }
            }
            // Dropped, and its FIN went out (or the host reset it).
            if s.dropped && !socket.is_active() {
                gone.push(i);
            }
        }
        for i in gone.into_iter().rev() {
            let l = self.links.remove(i);
            self.sockets.remove(l.socket);
        }
        moved
    }

    /// Connections whose host's name was being looked up.
    fn resolve(&mut self) {
        let Some(h) = self.dns else { return self.resolve_local() };
        let mut i = 0;
        while i < self.lookups.len() {
            let socket = self.sockets.get_mut::<dns::Socket>(h);
            match socket.get_query_result(self.lookups[i].query) {
                Err(dns::GetQueryResultError::Pending) => i += 1,
                Ok(ips) if !ips.is_empty() => {
                    let l = self.lookups.remove(i);
                    self.open(ips[0], l.port, l.reply);
                }
                _ => {
                    let l = self.lookups.remove(i);
                    let _ = l.reply.send(Err(Error::Unreachable { endpoint: format!(":{}", l.port) }));
                }
            }
        }
        self.resolve_local();
    }

    fn resolve_local(&mut self) {
        let mut i = 0;
        while i < self.local.len() {
            match self.local[i].rx.try_recv() {
                Err(oneshot::error::TryRecvError::Empty) => i += 1,
                Ok(Some(ip)) => {
                    let l = self.local.remove(i);
                    self.open(ip.into(), l.port, l.reply);
                }
                _ => {
                    let l = self.local.remove(i);
                    let _ = l.reply.send(Err(Error::Unreachable { endpoint: format!(":{}", l.port) }));
                }
            }
        }
    }
}
