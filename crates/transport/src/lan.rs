//! LAN mode: discover peers on the local network and connect to them directly, without the
//! signaling server. Discovery uses mDNS (service `_kronos-rs._tcp`); the WebRTC handshake
//! (offer/answer/ICE) is exchanged over a direct TCP connection between the two apps, then
//! the usual password handshake and media run peer-to-peer. The server is not involved.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use signaling_proto::{ClientMessage, PeerId, SessionId, SignalPayload};
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc};

use crate::SignalSender;

/// mDNS service type for Kronos Remote Support LAN discovery.
const SERVICE_TYPE: &str = "_kronos-rs._tcp.local.";

/// A peer found on the local network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanPeer {
    pub id: PeerId,
    /// Every address the peer announces (all its interfaces: LAN, VPN, virtual machines...),
    /// most likely reachable first. Not all of them may be reachable from here.
    pub addrs: Vec<SocketAddr>,
    /// Friendly host name (best effort), for the UI.
    pub name: String,
}

/// Messages on the direct LAN signaling connection (newline-delimited JSON).
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
enum LanMsg {
    /// First message from the connecting viewer: who it is.
    Hello {
        id: String,
    },
    Signal {
        payload: SignalPayload,
    },
    Bye,
}

/// A direct LAN session: drives a [`crate::PeerSession`] without the server.
pub struct LanSession {
    pub session: SessionId,
    pub peer: PeerId,
    /// Pass to `PeerSession::start` in place of the server sender.
    pub signal: SignalSender,
    /// WebRTC payloads from the other side; feed each to `PeerSession::handle_signal`.
    pub incoming: mpsc::UnboundedReceiver<SignalPayload>,
}

/// Advertises this device on the LAN and keeps a list of discovered peers.
pub struct LanDiscovery {
    daemon: mdns_sd::ServiceDaemon,
    full_name: String,
    peers: Arc<Mutex<HashMap<String, LanPeer>>>,
}

impl LanDiscovery {
    /// Starts advertising `id` at `port` and browsing for other peers.
    pub fn start(id: &PeerId, port: u16) -> Result<Self> {
        let daemon = mdns_sd::ServiceDaemon::new().context("starting mDNS")?;
        let host = hostname();
        // Instance name is the id (unique per device); TXT carries id and name.
        let props = [("id", id.as_str()), ("name", host.as_str())];
        let service = mdns_sd::ServiceInfo::new(
            SERVICE_TYPE,
            id.as_str(),
            &format!("{}.local.", id.as_str()),
            "",
            port,
            &props[..],
        )
        .context("building mDNS service")?
        .enable_addr_auto();
        let full_name = service.get_fullname().to_owned();
        daemon
            .register(service)
            .context("registering mDNS service")?;

        let peers: Arc<Mutex<HashMap<String, LanPeer>>> = Arc::default();
        let receiver = daemon.browse(SERVICE_TYPE).context("browsing mDNS")?;
        let (peers_bg, own) = (peers.clone(), id.as_str().to_owned());
        std::thread::Builder::new()
            .name("lan-discovery".into())
            .spawn(move || {
                while let Ok(event) = receiver.recv() {
                    match event {
                        mdns_sd::ServiceEvent::ServiceResolved(info) => {
                            if let Some(peer) = to_peer(&info, &own) {
                                peers_bg
                                    .lock()
                                    .unwrap()
                                    .insert(info.get_fullname().to_owned(), peer);
                            }
                        }
                        mdns_sd::ServiceEvent::ServiceRemoved(_, name) => {
                            peers_bg.lock().unwrap().remove(&name);
                        }
                        _ => {}
                    }
                }
            })
            .context("spawning discovery thread")?;
        Ok(Self {
            daemon,
            full_name,
            peers,
        })
    }

    /// Peers seen on the LAN right now.
    pub fn peers(&self) -> Vec<LanPeer> {
        let mut peers: Vec<_> = self.peers.lock().unwrap().values().cloned().collect();
        peers.sort_by(|a, b| a.name.cmp(&b.name));
        peers
    }

    /// Addresses of a discovered peer by id (never empty), if present.
    pub fn resolve(&self, id: &PeerId) -> Option<Vec<SocketAddr>> {
        self.peers
            .lock()
            .unwrap()
            .values()
            .find(|p| &p.id == id)
            .map(|p| p.addrs.clone())
    }
}

impl Drop for LanDiscovery {
    fn drop(&mut self) {
        let _ = self.daemon.unregister(&self.full_name);
        let _ = self.daemon.shutdown();
    }
}

fn to_peer(info: &mdns_sd::ServiceInfo, own: &str) -> Option<LanPeer> {
    let id = info.get_property_val_str("id")?;
    if id == own {
        return None; // our own advertisement
    }
    let peer_id: PeerId = id.parse().ok()?;
    let addrs = rank_addresses(info.get_addresses().iter().copied(), info.get_port());
    if addrs.is_empty() {
        return None;
    }
    let name = info.get_property_val_str("name").unwrap_or(id).to_owned();
    Some(LanPeer {
        id: peer_id,
        addrs,
        name,
    })
}

/// Orders announced addresses by how likely they are reachable: private IPv4 (the usual LAN)
/// first, then other IPv4, then routable IPv6. IPv6 link-local is dropped: mDNS gives no
/// scope id, so it cannot be dialled.
fn rank_addresses(addrs: impl Iterator<Item = std::net::IpAddr>, port: u16) -> Vec<SocketAddr> {
    use std::net::IpAddr;
    let mut ranked: Vec<(u8, IpAddr)> = addrs
        .filter_map(|ip| {
            let rank = match ip {
                IpAddr::V4(v4) if v4.is_loopback() || v4.is_link_local() => return None,
                IpAddr::V4(v4) if v4.is_private() => 0,
                IpAddr::V4(_) => 1,
                IpAddr::V6(v6) if v6.is_loopback() || v6.is_unicast_link_local() => return None,
                IpAddr::V6(_) => 2,
            };
            Some((rank, ip))
        })
        .collect();
    ranked.sort();
    ranked.dedup();
    ranked
        .into_iter()
        .map(|(_, ip)| SocketAddr::new(ip, port))
        .collect()
}

/// This computer's name as the OS reports it, without a domain suffix ("Mac-Studio.local"
/// becomes "Mac-Studio").
fn hostname() -> String {
    let name = gethostname::gethostname().to_string_lossy().into_owned();
    let short = name.split('.').next().unwrap_or_default().trim().to_owned();
    if short.is_empty() { "PC".into() } else { short }
}

/// Longest LAN signaling line (an SDP offer is a few KB).
const MAX_LINE: u64 = 64 * 1024;
/// Direct LAN connections open at once; more are refused (anyone on the network can
/// connect, before any password).
const MAX_LAN_CONNECTIONS: usize = 8;
/// A new LAN connection must introduce itself within this time.
const HELLO_TIMEOUT: Duration = Duration::from_secs(10);

/// Reads one line of at most [`MAX_LINE`] bytes; `Ok(0)` at the end of the stream.
async fn read_line_limited<R>(reader: &mut R, line: &mut String) -> std::io::Result<usize>
where
    R: AsyncBufRead + Unpin,
{
    let n = reader.take(MAX_LINE).read_line(line).await?;
    if n as u64 >= MAX_LINE && !line.ends_with('\n') {
        return Err(std::io::Error::other("LAN message too long"));
    }
    Ok(n)
}

/// Host side: listens for direct LAN connections. Returns the bound port (advertise it via
/// [`LanDiscovery::start`]) and a stream of incoming sessions.
pub async fn listen() -> Result<(u16, mpsc::UnboundedReceiver<LanSession>)> {
    let listener = TcpListener::bind(("0.0.0.0", 0))
        .await
        .context("binding LAN listener")?;
    let port = listener.local_addr()?.port();
    let (tx, rx) = mpsc::unbounded_channel();
    let slots = Arc::new(Semaphore::new(MAX_LAN_CONNECTIONS));
    tokio::spawn(async move {
        loop {
            let Ok((stream, addr)) = listener.accept().await else {
                break;
            };
            let Ok(slot) = slots.clone().try_acquire_owned() else {
                tracing::warn!(%addr, "too many LAN connections: refused");
                continue;
            };
            let tx = tx.clone();
            tokio::spawn(async move {
                let accepted = tokio::time::timeout(HELLO_TIMEOUT, accept_session(stream, slot));
                if let Ok(Ok(session)) = accepted.await {
                    let _ = tx.send(session);
                }
            });
        }
    });
    Ok((port, rx))
}

/// Viewer side: connect directly to a discovered peer and start a session (we are the offerer).
/// Tries each announced address in order (some may be VPN or virtual-machine interfaces not
/// reachable from here) and uses the first that answers.
pub async fn connect(addrs: &[SocketAddr], own_id: &PeerId, target: &PeerId) -> Result<LanSession> {
    let mut last_error = anyhow::anyhow!("no LAN address for the peer");
    let mut connected = None;
    for addr in addrs {
        let attempt = tokio::time::timeout(Duration::from_secs(2), TcpStream::connect(addr));
        match attempt.await {
            Ok(Ok(stream)) => {
                connected = Some(stream);
                break;
            }
            Ok(Err(e)) => last_error = anyhow::anyhow!("{addr}: {e}"),
            Err(_) => last_error = anyhow::anyhow!("{addr}: timed out"),
        }
    }
    let Some(stream) = connected else {
        return Err(last_error.context("connecting over the LAN"));
    };
    let (read, mut write) = stream.into_split();
    write_msg(
        &mut write,
        &LanMsg::Hello {
            id: own_id.as_str().to_owned(),
        },
    )
    .await?;
    // The peer on the other end is identified by the address; the real identity check is the
    // password handshake over the data channel afterwards.
    Ok(spawn_session(
        BufReader::new(read),
        write,
        target.clone(),
        next_session_id(),
        None,
    ))
}

async fn accept_session(stream: TcpStream, slot: OwnedSemaphorePermit) -> Result<LanSession> {
    let (read, write) = stream.into_split();
    let mut reader = BufReader::new(read);
    let mut line = String::new();
    read_line_limited(&mut reader, &mut line).await?;
    let peer = match serde_json::from_str::<LanMsg>(line.trim())? {
        LanMsg::Hello { id } => id.parse().unwrap_or_else(|_| placeholder_id()),
        _ => anyhow::bail!("expected hello"),
    };
    // Keep the same buffered reader: bytes after the hello (the offer) may already be in it.
    Ok(spawn_session(
        reader,
        write,
        peer,
        next_session_id(),
        Some(slot),
    ))
}

/// `slot` (host side) is held until the connection ends.
fn spawn_session<R, W>(
    mut reader: R,
    mut write: W,
    peer: PeerId,
    session: SessionId,
    slot: Option<OwnedSemaphorePermit>,
) -> LanSession
where
    R: tokio::io::AsyncBufRead + Unpin + Send + 'static,
    W: tokio::io::AsyncWrite + Unpin + Send + 'static,
{
    let (signal, mut to_peer) = SignalSender::channel();
    let (incoming_tx, incoming) = mpsc::unbounded_channel();
    // Outgoing: translate the session's ClientMessages into LAN wire messages.
    tokio::spawn(async move {
        while let Some(msg) = to_peer.recv().await {
            let wire = match msg {
                ClientMessage::Signal { payload, .. } => LanMsg::Signal { payload },
                ClientMessage::Leave { .. } => LanMsg::Bye,
                _ => continue,
            };
            if write_msg(&mut write, &wire).await.is_err() {
                break;
            }
        }
        let _ = write.shutdown().await;
    });
    // Incoming: forward payloads to the session.
    tokio::spawn(async move {
        let _slot = slot;
        let mut line = String::new();
        loop {
            line.clear();
            match read_line_limited(&mut reader, &mut line).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            match serde_json::from_str::<LanMsg>(line.trim()) {
                Ok(LanMsg::Signal { payload }) => {
                    if incoming_tx.send(payload).is_err() {
                        break;
                    }
                }
                Ok(LanMsg::Bye) | Err(_) => break,
                Ok(LanMsg::Hello { .. }) => {}
            }
        }
    });
    LanSession {
        session,
        peer,
        signal,
        incoming,
    }
}

async fn write_msg<W: AsyncWriteExt + Unpin>(write: &mut W, msg: &LanMsg) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(msg).expect("serializable");
    line.push(b'\n');
    write.write_all(&line).await?;
    write.flush().await
}

/// Local, process-unique session ids for LAN sessions (never leave this machine's maps).
fn next_session_id() -> SessionId {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(1);
    // Bit 52 set so LAN ids never collide with server-assigned ones (small counters). Must
    // stay below 2^53: ids travel to the UI as JavaScript numbers, and larger values get
    // rounded there (the approval click would then answer a session that does not exist).
    (1u64 << 52) | NEXT.fetch_add(1, Ordering::Relaxed)
}

fn placeholder_id() -> PeerId {
    "000000000".parse().expect("valid placeholder id")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn direct_signaling_roundtrip() {
        let (port, mut incoming) = listen().await.unwrap();
        let viewer_id: PeerId = "123456789".parse().unwrap();
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let host_id: PeerId = "987654321".parse().unwrap();
        let mut viewer = connect(&[addr], &viewer_id, &host_id).await.unwrap();
        let mut host = incoming.recv().await.unwrap();
        assert_eq!(host.peer, viewer_id);
        assert_eq!(viewer.peer, host_id);

        // Viewer (offerer) sends an offer; host receives it.
        viewer.signal.send(ClientMessage::Signal {
            session: viewer.session,
            payload: SignalPayload::Offer {
                sdp: "v=0 offer".into(),
            },
        });
        match host.incoming.recv().await.unwrap() {
            SignalPayload::Offer { sdp } => assert_eq!(sdp, "v=0 offer"),
            other => panic!("expected offer, got {other:?}"),
        }
        // Host (answerer) replies; viewer receives it.
        host.signal.send(ClientMessage::Signal {
            session: host.session,
            payload: SignalPayload::Answer {
                sdp: "v=0 answer".into(),
            },
        });
        match viewer.incoming.recv().await.unwrap() {
            SignalPayload::Answer { sdp } => assert_eq!(sdp, "v=0 answer"),
            other => panic!("expected answer, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod address_tests {
    use super::*;
    use std::net::IpAddr;

    #[test]
    fn private_ipv4_first_and_link_local_dropped() {
        let ips = [
            "fe80::1",
            "2001:db8::5",
            "100.64.0.7",
            "192.168.1.20",
            "169.254.3.4",
        ];
        let ips = ips.iter().map(|s| s.parse::<IpAddr>().unwrap());
        let ranked = rank_addresses(ips, 7000);
        let got: Vec<String> = ranked.iter().map(|a| a.to_string()).collect();
        assert_eq!(
            got,
            ["192.168.1.20:7000", "100.64.0.7:7000", "[2001:db8::5]:7000"]
        );
    }

    #[test]
    fn session_ids_fit_in_a_javascript_number() {
        let id = next_session_id();
        assert!(
            id < (1u64 << 53),
            "{id} is not exactly representable as an f64"
        );
        assert!(id > (1u64 << 52));
    }

    #[tokio::test]
    async fn connect_skips_unreachable_addresses() {
        let (port, mut incoming) = listen().await.unwrap();
        // A closed local port answers at once with "connection refused".
        let closed = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap();
        let good = SocketAddr::from(([127, 0, 0, 1], port));
        let viewer_id: PeerId = "123456789".parse().unwrap();
        let host_id: PeerId = "987654321".parse().unwrap();
        let viewer = connect(&[closed, good], &viewer_id, &host_id).await;
        assert!(
            viewer.is_ok(),
            "should fall through to the reachable address"
        );
        assert_eq!(incoming.recv().await.unwrap().peer, viewer_id);
    }

    #[tokio::test]
    async fn overlong_lines_are_refused() {
        let long = vec![b'a'; MAX_LINE as usize + 10];
        let mut line = String::new();
        assert!(read_line_limited(&mut &long[..], &mut line).await.is_err());
        line.clear();
        let n = read_line_limited(&mut &b"{}\n"[..], &mut line)
            .await
            .unwrap();
        assert_eq!((n, line.as_str()), (3, "{}\n"));
    }
}

#[cfg(test)]
mod discovery_tests {
    use super::*;

    // Needs a real network interface + multicast; run explicitly: `cargo test -p transport
    // -- --ignored discovers_a_peer`.
    #[test]
    #[ignore]
    fn discovers_a_peer() {
        let a_id: PeerId = "111111111".parse().unwrap();
        let b_id: PeerId = "222222222".parse().unwrap();
        let a = LanDiscovery::start(&a_id, 40001).unwrap();
        let b = LanDiscovery::start(&b_id, 40002).unwrap();
        let mut found = None;
        for _ in 0..50 {
            if let Some(addr) = a.resolve(&b_id) {
                found = Some(addr);
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        let addrs = found.expect("A should discover B on the LAN");
        assert!(addrs.iter().all(|a| a.port() == 40002));
        // A must not list itself.
        assert!(a.resolve(&a_id).is_none());
        drop((a, b));
    }
}
