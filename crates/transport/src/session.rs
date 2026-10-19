//! One WebRTC peer connection, negotiated through the signaling server with trickle ICE.

use std::sync::Arc;

use anyhow::{Context, Result};
use signaling_proto::{ClientMessage, IceServer, SessionId, SignalPayload};
use tokio::sync::{mpsc, watch};
use webrtc::data_channel::{DataChannel, RTCDataChannelInit};
use webrtc::peer_connection::{
    MediaEngine, PeerConnection, PeerConnectionBuilder, PeerConnectionEventHandler,
    RTCConfigurationBuilder, RTCIceCandidateInit, RTCIceServer, RTCIceTransportPolicy,
    RTCPeerConnectionIceEvent, RTCPeerConnectionState, RTCSessionDescription, Registry,
    register_default_interceptors,
};

use rtc::rtp_transceiver::rtp_sender::RtpCodecKind;
use webrtc::media_stream::track_remote::TrackRemote;
use webrtc::rtp_transceiver::{RTCRtpTransceiverDirection, RTCRtpTransceiverInit};

use crate::SignalSender;
use crate::handshake::sdp_fingerprint;
use crate::video::VideoSender;

/// Label of the reliable, ordered channel used for control messages and input.
pub const CONTROL_CHANNEL: &str = "control";
/// Label of the channel carrying clipboard file transfers, kept apart so bulk data never
/// delays input and control messages (each channel is its own SCTP stream).
pub const FILES_CHANNEL: &str = "files";

/// Label of the channel of the file manager (browsing the host's folders, copying files
/// both ways); older hosts ignore it.
pub const FS_CHANNEL: &str = "fs";

/// Label of the unordered, unreliable channel carrying the host's sound (a late audio
/// packet is worthless; a lost one is concealed by the decoder).
pub const AUDIO_CHANNEL: &str = "audio";

/// The data channels of an authenticated session.
pub struct SessionChannels {
    pub control: Arc<dyn DataChannel>,
    /// Resolves once the files channel exists; never (error) with peers that predate it.
    pub files: tokio::sync::oneshot::Receiver<Arc<dyn DataChannel>>,
    /// Same for the audio channel (peers before 1.3.0 do not open it).
    pub audio: tokio::sync::oneshot::Receiver<Arc<dyn DataChannel>>,
    /// Same for the file manager channel (peers before 1.8.1 do not open it).
    pub fs: tokio::sync::oneshot::Receiver<Arc<dyn DataChannel>>,
}

/// Picks the control and files channels out of a session's channel stream (the remote
/// peer's channels may arrive in any order). `None` if the control channel never comes.
pub async fn sort_channels(
    mut channels: mpsc::UnboundedReceiver<Arc<dyn DataChannel>>,
) -> Option<SessionChannels> {
    let mut others = Vec::new();
    let control = loop {
        let channel = channels.recv().await?;
        match channel.label().await.as_deref() {
            Ok(CONTROL_CHANNEL) => break channel,
            _ => others.push(channel),
        }
    };
    let (files_tx, files) = tokio::sync::oneshot::channel();
    let (audio_tx, audio) = tokio::sync::oneshot::channel();
    let (fs_tx, fs) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let (mut files_tx, mut audio_tx, mut fs_tx) = (Some(files_tx), Some(audio_tx), Some(fs_tx));
        let mut pending = others.into_iter();
        while files_tx.is_some() || audio_tx.is_some() || fs_tx.is_some() {
            let channel = match pending.next() {
                Some(channel) => channel,
                None => match channels.recv().await {
                    Some(channel) => channel,
                    None => return,
                },
            };
            let slot = match channel.label().await.as_deref() {
                Ok(FILES_CHANNEL) => &mut files_tx,
                Ok(AUDIO_CHANNEL) => &mut audio_tx,
                Ok(FS_CHANNEL) => &mut fs_tx,
                _ => continue,
            };
            if let Some(tx) = slot.take() {
                let _ = tx.send(channel);
            }
        }
    });
    Some(SessionChannels {
        control,
        files,
        audio,
        fs,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// Creates the offer and the data channel (the viewer).
    Offerer,
    /// Answers (the host).
    Answerer,
}

pub struct PeerSession {
    pub id: SessionId,
    role: Role,
    pc: Arc<dyn PeerConnection>,
    /// Host: outgoing screen track (added before answering so it is negotiated).
    video_sender: Option<Arc<VideoSender>>,
    /// Viewer: incoming tracks, taken once by the video task.
    tracks: Option<mpsc::UnboundedReceiver<Arc<dyn TrackRemote>>>,
    signal: SignalSender,
    has_remote_description: bool,
    /// Remote candidates that arrived before the remote description.
    pending_candidates: Vec<RTCIceCandidateInit>,
    channels: Option<mpsc::UnboundedReceiver<Arc<dyn DataChannel>>>,
    state: watch::Receiver<RTCPeerConnectionState>,
}

#[derive(Clone)]
pub struct SessionInfo {
    pc: Arc<dyn PeerConnection>,
    role: Role,
}

/// Network path a session actually uses (from the selected ICE candidate pair).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionPath {
    /// Both ends on the same network (host candidates).
    Lan,
    /// Peer-to-peer across the Internet (NAT traversed with STUN).
    Direct,
    /// Through the TURN relay server.
    Relay,
}

impl SessionInfo {
    /// Which path the media and data currently take; `None` until ICE has selected a pair.
    pub async fn connection_path(&self) -> Option<ConnectionPath> {
        use webrtc::peer_connection::{RTCIceCandidateType, RTCStatsReportEntry, StatsSelector};

        let report = self
            .pc
            .get_stats(std::time::Instant::now(), StatsSelector::None)
            .await;
        let pair_id = report
            .transport()
            .map(|t| t.selected_candidate_pair_id.clone())
            .filter(|id| !id.is_empty())?;
        let Some(RTCStatsReportEntry::IceCandidatePair(pair)) = report.get(&pair_id) else {
            return None;
        };
        // The pair refers to candidates as "candidate:…", while their stats entries are
        // keyed "RTCLocalIceCandidate_candidate:…" / "RTCRemoteIceCandidate_candidate:…".
        let kind = |id: &str| {
            report.iter().find_map(|e| match e {
                RTCStatsReportEntry::LocalCandidate(c)
                | RTCStatsReportEntry::RemoteCandidate(c)
                    if e.id() == id || e.id().ends_with(&format!("_{id}")) =>
                {
                    Some(c.candidate_type)
                }
                _ => None,
            })
        };
        let (local, remote) = (
            kind(&pair.local_candidate_id)?,
            kind(&pair.remote_candidate_id)?,
        );
        Some(match (local, remote) {
            (RTCIceCandidateType::Relay, _) | (_, RTCIceCandidateType::Relay) => {
                ConnectionPath::Relay
            }
            (RTCIceCandidateType::Host, RTCIceCandidateType::Host) => ConnectionPath::Lan,
            _ => ConnectionPath::Direct,
        })
    }

    /// Bytes sent and received so far on this connection (all media and data).
    pub async fn traffic(&self) -> Option<(u64, u64)> {
        let report = self
            .pc
            .get_stats(
                std::time::Instant::now(),
                webrtc::peer_connection::StatsSelector::None,
            )
            .await;
        let transport = report.transport()?;
        Some((transport.bytes_sent, transport.bytes_received))
    }

    /// DTLS fingerprints of both ends from the negotiated SDPs, binding the password
    /// handshake to this exact connection. Available once negotiation completed.
    pub async fn transcript(&self) -> Option<auth::Transcript> {
        let local = sdp_fingerprint(&self.pc.local_description().await?.sdp)?;
        let remote = sdp_fingerprint(&self.pc.remote_description().await?.sdp)?;
        let (viewer_fingerprint, host_fingerprint) = match self.role {
            Role::Offerer => (local, remote),
            Role::Answerer => (remote, local),
        };
        Some(auth::Transcript {
            viewer_fingerprint,
            host_fingerprint,
        })
    }
}

struct Events {
    session: SessionId,
    signal: SignalSender,
    channels: mpsc::UnboundedSender<Arc<dyn DataChannel>>,
    tracks: mpsc::UnboundedSender<Arc<dyn TrackRemote>>,
    state: watch::Sender<RTCPeerConnectionState>,
}

#[async_trait::async_trait]
impl PeerConnectionEventHandler for Events {
    async fn on_ice_candidate(&self, event: RTCPeerConnectionIceEvent) {
        match event.candidate.to_json() {
            Ok(c) => self.signal.send(ClientMessage::Signal {
                session: self.session,
                payload: SignalPayload::Candidate {
                    candidate: c.candidate,
                    sdp_mid: c.sdp_mid,
                    sdp_mline_index: c.sdp_mline_index,
                },
            }),
            Err(e) => tracing::warn!("cannot serialize local candidate: {e}"),
        }
    }

    async fn on_connection_state_change(&self, state: RTCPeerConnectionState) {
        tracing::info!(session = self.session, %state, "peer connection state");
        let _ = self.state.send(state);
    }

    async fn on_data_channel(&self, channel: Arc<dyn DataChannel>) {
        let _ = self.channels.send(channel);
    }

    async fn on_track(&self, track: Arc<dyn TrackRemote>) {
        let _ = self.tracks.send(track);
    }
}

fn to_rtc_ice_servers(servers: &[IceServer]) -> Vec<RTCIceServer> {
    servers
        .iter()
        .map(|s| RTCIceServer {
            urls: s.urls.clone(),
            username: s.username.clone(),
            credential: s.credential.clone(),
        })
        .collect()
}

/// Interface names (lowercased substrings) of VPN/overlay tunnels. A path through them
/// (e.g. Tailscale via its relays) measured 12–150 ms between two apps on the same Mac,
/// against <1 ms on the LAN, and ICE may nominate it first. Real networks are reached
/// through physical interfaces, srflx or (later) TURN.
/// Longest wait for a peer connection to close.
const CLOSE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

const TUNNEL_PREFIXES: &[&str] = &[
    "utun",
    "tun",
    "tap",
    "wg",
    "tailscale",
    "zt",
    "zerotier",
    "ppp",
    "ipsec",
    "wireguard",
    "openvpn",
];

fn is_tunnel(interface: &str) -> bool {
    let name = interface.to_ascii_lowercase();
    TUNNEL_PREFIXES
        .iter()
        .any(|p| name.starts_with(p) || name.contains("vpn"))
}

/// One UDP socket per usable physical IPv4 interface; the wildcard if none qualifies
/// (the connection can still use srflx/relay candidates).
fn local_bind_addrs() -> Vec<String> {
    let addrs: Vec<String> = if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|i| !i.is_loopback() && !is_tunnel(&i.name))
        .filter_map(|i| match i.ip() {
            std::net::IpAddr::V4(ip) if !ip.is_link_local() => Some(format!("{ip}:0")),
            _ => None,
        })
        .collect();
    if addrs.is_empty() {
        vec!["0.0.0.0:0".to_owned()]
    } else {
        addrs
    }
}

impl PeerSession {
    /// Builds the peer connection. The offerer immediately creates the control channel
    /// and sends its offer; the answerer waits for [`handle_signal`](Self::handle_signal).
    /// `relay_only` forces traffic through TURN (testing the relay path).
    pub async fn start(
        role: Role,
        id: SessionId,
        ice_servers: &[IceServer],
        relay_only: bool,
        signal: SignalSender,
    ) -> Result<Self> {
        let mut media = MediaEngine::default();
        media.register_default_codecs()?;
        let registry = register_default_interceptors(Registry::new(), &mut media)?;

        let (channels_tx, channels) = mpsc::unbounded_channel();
        let (tracks_tx, tracks) = mpsc::unbounded_channel();
        let (state_tx, state) = watch::channel(RTCPeerConnectionState::New);

        let pc = PeerConnectionBuilder::new()
            .with_configuration(
                RTCConfigurationBuilder::new()
                    .with_ice_servers(to_rtc_ice_servers(ice_servers))
                    .with_ice_transport_policy(if relay_only {
                        RTCIceTransportPolicy::Relay
                    } else {
                        RTCIceTransportPolicy::All
                    })
                    .build(),
            )
            .with_media_engine(media)
            .with_interceptor_registry(registry)
            .with_handler(Arc::new(Events {
                session: id,
                signal: signal.clone(),
                channels: channels_tx.clone(),
                tracks: tracks_tx,
                state: state_tx,
            }))
            .with_udp_addrs(local_bind_addrs())
            .build()
            .await
            .context("building peer connection")?;
        let pc: Arc<dyn PeerConnection> = Arc::new(pc);

        let mut video_sender = None;
        if role == Role::Answerer {
            video_sender = Some(Arc::new(VideoSender::attach(&pc).await?));
        }
        if role == Role::Offerer {
            // The viewer only receives video; the host's track is matched to this m-line.
            pc.add_transceiver_from_kind(
                RtpCodecKind::Video,
                Some(RTCRtpTransceiverInit {
                    direction: RTCRtpTransceiverDirection::Recvonly,
                    ..Default::default()
                }),
            )
            .await?;
            let channel = pc.create_data_channel(CONTROL_CHANNEL, None).await?;
            let _ = channels_tx.send(channel);
            let files = pc.create_data_channel(FILES_CHANNEL, None).await?;
            let _ = channels_tx.send(files);
            let audio = pc
                .create_data_channel(
                    AUDIO_CHANNEL,
                    Some(RTCDataChannelInit {
                        ordered: false,
                        max_retransmits: Some(0),
                        ..Default::default()
                    }),
                )
                .await?;
            let _ = channels_tx.send(audio);
            let fs = pc.create_data_channel(FS_CHANNEL, None).await?;
            let _ = channels_tx.send(fs);

            let offer = pc.create_offer(None).await?;
            let sdp = offer.sdp.clone();
            pc.set_local_description(offer).await?;
            signal.send(ClientMessage::Signal {
                session: id,
                payload: SignalPayload::Offer { sdp },
            });
        }

        Ok(Self {
            id,
            role,
            pc,
            video_sender,
            tracks: Some(tracks),
            signal,
            has_remote_description: false,
            pending_candidates: Vec::new(),
            channels: Some(channels),
            state,
        })
    }

    /// Applies a negotiation message relayed from the other peer.
    pub async fn handle_signal(&mut self, payload: SignalPayload) -> Result<()> {
        match payload {
            SignalPayload::Offer { sdp } => {
                self.set_remote(RTCSessionDescription::offer(sdp)?).await?;
                let answer = self.pc.create_answer(None).await?;
                let sdp = answer.sdp.clone();
                self.pc.set_local_description(answer).await?;
                self.signal.send(ClientMessage::Signal {
                    session: self.id,
                    payload: SignalPayload::Answer { sdp },
                });
            }
            SignalPayload::Answer { sdp } => {
                self.set_remote(RTCSessionDescription::answer(sdp)?).await?;
            }
            SignalPayload::Candidate {
                candidate,
                sdp_mid,
                sdp_mline_index,
            } => {
                let init = RTCIceCandidateInit {
                    candidate,
                    sdp_mid,
                    sdp_mline_index,
                    ..Default::default()
                };
                if self.has_remote_description {
                    self.pc.add_ice_candidate(init).await?;
                } else {
                    self.pending_candidates.push(init);
                }
            }
        }
        Ok(())
    }

    async fn set_remote(&mut self, desc: RTCSessionDescription) -> Result<()> {
        self.pc.set_remote_description(desc).await?;
        self.has_remote_description = true;
        for candidate in self.pending_candidates.drain(..) {
            self.pc.add_ice_candidate(candidate).await?;
        }
        Ok(())
    }

    /// Stream of data channels: the offerer's own control channel, or those opened by the
    /// remote peer. Can be taken once, so a separate task can own the channels while the
    /// session keeps processing signaling.
    pub fn take_data_channels(&mut self) -> Option<mpsc::UnboundedReceiver<Arc<dyn DataChannel>>> {
        self.channels.take()
    }

    /// Host: the outgoing screen track.
    pub fn video_sender(&self) -> Option<Arc<VideoSender>> {
        self.video_sender.clone()
    }

    /// Viewer: stream of incoming media tracks (the host's screen). Can be taken once.
    pub fn take_video_tracks(&mut self) -> Option<mpsc::UnboundedReceiver<Arc<dyn TrackRemote>>> {
        self.tracks.take()
    }

    /// Cheap handle for tasks that need connection details (e.g. the auth handshake).
    pub fn info(&self) -> SessionInfo {
        SessionInfo {
            pc: self.pc.clone(),
            role: self.role,
        }
    }

    /// Watch connection state changes (connected, failed, ...).
    pub fn state(&self) -> watch::Receiver<RTCPeerConnectionState> {
        self.state.clone()
    }

    /// Ends the session: tells the other peer (via the server) and closes the connection.
    pub async fn close(&self) {
        self.signal.send(ClientMessage::Leave { session: self.id });
        self.shutdown().await;
    }

    /// Closes the connection without notifying anyone, e.g. after the server reported
    /// the session as already closed.
    pub async fn shutdown(&self) {
        // A connection that went stale (e.g. this computer slept) must not hold up whoever
        // closes it: the service loop waits here.
        match tokio::time::timeout(CLOSE_TIMEOUT, self.pc.close()).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => tracing::warn!("closing peer connection: {e}"),
            Err(_) => tracing::warn!(session = self.id, "closing peer connection timed out"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::is_tunnel;

    #[test]
    fn tunnel_interfaces_are_recognized() {
        for name in [
            "utun8",
            "tun0",
            "wg0",
            "Tailscale",
            "ZeroTier One",
            "OpenVPN TAP-Windows6",
            "ppp0",
        ] {
            assert!(is_tunnel(name), "{name}");
        }
        for name in ["en0", "eth0", "wlan0", "Ethernet", "Wi-Fi", "enp3s0"] {
            assert!(!is_tunnel(name), "{name}");
        }
    }
}
