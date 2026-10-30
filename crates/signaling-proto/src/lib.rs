//! Messages exchanged over the signaling WebSocket (JSON text frames).
//!
//! The signaling server only matches peers and relays SDP/ICE between them; it never
//! sees media or input. Types here are deliberately independent from the WebRTC
//! library so the server does not need to link it.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Public 9-digit identifier of a device, shown to users ("123 456 789").
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PeerId(String);

impl PeerId {
    pub const DIGITS: usize = 9;

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for PeerId {
    type Error = String;

    /// Accepts digits with optional spaces, as users tend to type them.
    fn try_from(value: String) -> Result<Self, Self::Error> {
        let digits: String = value.chars().filter(|c| !c.is_whitespace()).collect();
        if digits.len() == Self::DIGITS
            && digits.bytes().all(|b| b.is_ascii_digit())
            && !digits.starts_with('0')
        {
            Ok(Self(digits))
        } else {
            Err(format!("invalid peer id {value:?}: expected 9 digits"))
        }
    }
}

impl std::str::FromStr for PeerId {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::try_from(s.to_owned())
    }
}

impl From<PeerId> for String {
    fn from(id: PeerId) -> Self {
        id.0
    }
}

impl fmt::Display for PeerId {
    /// Grouped for readability: `123 456 789`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = &self.0;
        write!(f, "{} {} {}", &s[0..3], &s[3..6], &s[6..9])
    }
}

/// Identifies one connection attempt between two peers.
pub type SessionId = u64;

/// STUN/TURN server handed out by the signaling server.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IceServer {
    pub urls: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub username: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub credential: String,
}

/// WebRTC negotiation data relayed verbatim between the two peers of a session.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SignalPayload {
    Offer {
        sdp: String,
    },
    Answer {
        sdp: String,
    },
    Candidate {
        candidate: String,
        sdp_mid: Option<String>,
        sdp_mline_index: Option<u16>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    /// First message on every connection. `device_key` is a random secret stored on the
    /// device; the same key always gets the same id back.
    Register {
        device_key: String,
    },
    /// Ask to open a session with another peer (the caller becomes the WebRTC offerer).
    Connect {
        target: PeerId,
    },
    Signal {
        session: SessionId,
        payload: SignalPayload,
    },
    Leave {
        session: SessionId,
    },
    /// Which of these devices (at most [`MAX_PRESENCE_IDS`]) are connected to the server
    /// now; answered with `Presence`. Only to servers that announce [`FEATURE_PRESENCE`],
    /// at most once every [`PRESENCE_INTERVAL_SECS`] per connection.
    Presence {
        ids: Vec<PeerId>,
    },
}

/// `Registered.features` entry of servers that answer `Presence`.
pub const FEATURE_PRESENCE: &str = "presence";
/// Most ids asked about in one `Presence` request.
pub const MAX_PRESENCE_IDS: usize = 20;
/// Least time between two `Presence` requests on one connection: the online state of a
/// list of known devices, not a way to scan ids.
pub const PRESENCE_INTERVAL_SECS: u64 = 10;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    BadRequest,
    NotRegistered,
    PeerOffline,
    SelfConnect,
    UnknownSession,
    RateLimited,
    /// The same device registered again on another connection; this one is closed.
    Replaced,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    Registered {
        id: PeerId,
        ice_servers: Vec<IceServer>,
        /// Optional abilities of this server (e.g. [`FEATURE_PRESENCE`]); empty from old ones.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        features: Vec<String>,
    },
    /// Sent to the caller of `Connect`.
    SessionCreated {
        session: SessionId,
        peer: PeerId,
        /// Fresh STUN/TURN servers (TURN credentials are short-lived); empty from old servers.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        ice_servers: Vec<IceServer>,
    },
    /// Sent to the target of `Connect`.
    IncomingSession {
        session: SessionId,
        from: PeerId,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        ice_servers: Vec<IceServer>,
    },
    Signal {
        session: SessionId,
        payload: SignalPayload,
    },
    SessionClosed {
        session: SessionId,
        reason: String,
    },
    /// Answer to `Presence`: the asked ids that are online.
    Presence {
        online: Vec<PeerId>,
    },
    Error {
        code: ErrorCode,
        message: String,
    },
}

impl ClientMessage {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("ClientMessage is always serializable")
    }
}

impl ServerMessage {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("ServerMessage is always serializable")
    }

    pub fn error(code: ErrorCode, message: impl Into<String>) -> Self {
        Self::Error {
            code,
            message: message.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presence_messages_and_old_servers() {
        // Servers before presence send no `features`.
        let old = r#"{"type":"registered","id":"123456789","ice_servers":[]}"#;
        let Ok(ServerMessage::Registered { features, .. }) = serde_json::from_str(old) else {
            panic!("old registered message not read");
        };
        assert!(features.is_empty());
        let ask = ClientMessage::Presence {
            ids: vec!["123 456 789".parse().unwrap()],
        };
        assert_eq!(ask.to_json(), r#"{"type":"presence","ids":["123456789"]}"#);
        let reply: ServerMessage =
            serde_json::from_str(r#"{"type":"presence","online":["123456789"]}"#).unwrap();
        assert_eq!(
            reply,
            ServerMessage::Presence {
                online: vec!["123456789".parse().unwrap()]
            }
        );
    }

    #[test]
    fn peer_id_accepts_spaces_and_formats_grouped() {
        let id: PeerId = "123 456 789".parse().unwrap();
        assert_eq!(id.as_str(), "123456789");
        assert_eq!(id.to_string(), "123 456 789");
    }

    #[test]
    fn peer_id_rejects_bad_input() {
        for bad in ["12345678", "1234567890", "12345678a", "012345678"] {
            assert!(bad.parse::<PeerId>().is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn invalid_id_in_json_is_rejected() {
        let json = r#"{"type":"connect","target":"42"}"#;
        assert!(serde_json::from_str::<ClientMessage>(json).is_err());
    }

    #[test]
    fn wire_format_is_stable() {
        let msg = ClientMessage::Signal {
            session: 7,
            payload: SignalPayload::Answer { sdp: "v=0".into() },
        };
        assert_eq!(
            msg.to_json(),
            r#"{"type":"signal","session":7,"payload":{"kind":"answer","sdp":"v=0"}}"#
        );
        let back: ClientMessage = serde_json::from_str(&msg.to_json()).unwrap();
        assert_eq!(back, msg);
    }
}
