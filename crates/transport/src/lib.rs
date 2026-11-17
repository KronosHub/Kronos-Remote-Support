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
    CONTROL_CHANNEL, ConnectionPath, FILES_CHANNEL, FS_CHANNEL, PeerSession, Role, SessionChannels,
    SessionInfo, sort_channels,
};
pub use signaling::{SignalSender, SignalingClient};
pub use webrtc::data_channel::{DataChannel, DataChannelEvent};
pub use webrtc::media_stream::track_remote::TrackRemote;
pub use webrtc::peer_connection::RTCPeerConnectionState as ConnectionState;

/// NACK generator of the viewer: asks for each missing video packet at most twice, 100 ms
/// apart. The library's default asks every 100 ms with no limit, so on a link with 1 s of
/// round trip every loss came back about ten times and the duplicates filled the link
/// (ADR 0076; `tests/nack_storm.rs`). The picture gives a gap up after 200 ms anyway
/// (`video::MAX_REORDER_WAIT`), so later answers are of no use.
pub fn nack_generator() -> rtc::interceptor::NackGeneratorBuilder {
    rtc::interceptor::NackGeneratorBuilder::new()
        .with_interval(std::time::Duration::from_millis(100))
        .with_max_nacks_per_packet(2)
}
