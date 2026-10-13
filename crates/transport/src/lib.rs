//! Networking: signaling client and WebRTC sessions.
//!
//! All use of the `webrtc` crate is confined to this crate (ADR 0004), so the WebRTC
//! library can be swapped without touching the rest of the code.

pub mod diag;
pub mod handshake;
pub mod lan;
mod session;
mod signaling;
pub mod video;

pub use auth::{
    Transcript, derive_personal_secret, generate_password, normalize_password,
    validate_personal_password,
};
pub use session::{
    CONTROL_CHANNEL, ConnectionPath, FILES_CHANNEL, PeerSession, Role, SessionChannels,
    SessionInfo, sort_channels,
};
pub use signaling::{SignalSender, SignalingClient};
pub use webrtc::data_channel::{DataChannel, DataChannelEvent};
pub use webrtc::media_stream::track_remote::TrackRemote;
pub use webrtc::peer_connection::RTCPeerConnectionState as ConnectionState;
