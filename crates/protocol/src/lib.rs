//! Messages exchanged on the `control` data channel once the password handshake succeeded.
//!
//! JSON text frames, tagged by `t`. Video itself travels on a WebRTC video track, not here.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ControlMsg {
    /// Latency probe; the other side answers with `Pong` carrying the same values.
    Ping {
        seq: u32,
        sent_us: u64,
    },
    Pong {
        seq: u32,
        sent_us: u64,
    },
    /// Host → viewer: screen sharing started with this output size.
    StreamStarted {
        width: u32,
        height: u32,
    },
    /// Host → viewer: screen sharing could not start or stopped (e.g. missing permission).
    StreamError {
        message: String,
    },
    /// Viewer → host: decoder lost sync (start, packet loss, decode error); send an IDR.
    RequestKeyframe,
    /// Either side ends the session.
    Bye {
        reason: String,
    },
    /// Viewer → host: mouse/keyboard event to replay on the host.
    Input {
        event: InputEvent,
    },
    /// Host → viewer: remote control is not possible (e.g. missing permission); viewing still works.
    InputUnavailable {
        message: String,
    },
    /// Host → viewer: the host's monitors and which one is being shared.
    Displays {
        displays: Vec<DisplayInfo>,
        current: u32,
    },
    /// Viewer → host: share another monitor.
    SelectDisplay {
        id: u32,
    },
    /// Viewer → host, every second: video packets received and lost since the previous
    /// report and the latest round-trip time (0 = not measured yet). Drives adaptive quality.
    Report {
        received: u32,
        lost: u32,
        rtt_ms: u32,
    },
    /// Either side: text copied on that machine, to put on the other one's clipboard.
    Clipboard {
        text: String,
    },
    /// Viewer → host: share clipboards in this session or not (sent at start and on change;
    /// the host only shares its clipboard after `enabled: true`).
    ClipboardMode {
        enabled: bool,
    },
    /// Host → viewer, every 2 s: CPU (share of the whole machine, %) and memory (MB) the
    /// host app uses, shown in the viewer next to the stream statistics.
    Usage {
        cpu: f32,
        ram_mb: u32,
    },
    /// Either side: a chat message typed by the user, shown to the other.
    Chat {
        text: String,
    },
    /// Host → viewer: the stream quality changed (bitrate and frame rate now targeted;
    /// `limited` = below the best quality because of the available bandwidth).
    Quality {
        kbps: u32,
        fps: u32,
        limited: bool,
    },
    /// Viewer → host: send up to the full size (on, the default) or at most 1920×1080
    /// (off). Older hosts ignore it.
    HighResolution {
        enabled: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DisplayInfo {
    pub id: u32,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub primary: bool,
}

/// A user input event, independent of the viewer's OS and screen size.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "k", rename_all = "snake_case")]
pub enum InputEvent {
    /// Absolute position on the shared screen, normalized to 0.0..=1.0.
    MouseMove {
        x: f32,
        y: f32,
    },
    MouseButton {
        button: MouseButton,
        down: bool,
    },
    /// Scroll in wheel notches (positive = right / down).
    Wheel {
        dx: i32,
        dy: i32,
    },
    /// Physical key, as a W3C `KeyboardEvent.code` ("KeyA", "ShiftLeft", "ArrowUp"...).
    /// The host's keyboard layout decides the resulting character.
    Key {
        code: String,
        down: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Back,
    Forward,
}

impl ControlMsg {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("ControlMsg is always serializable")
    }

    pub fn from_json(bytes: &[u8]) -> Option<Self> {
        serde_json::from_slice(bytes).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_and_wire_format() {
        let msg = ControlMsg::StreamStarted {
            width: 1920,
            height: 1080,
        };
        assert_eq!(
            msg.to_json(),
            r#"{"t":"stream_started","width":1920,"height":1080}"#
        );
        assert_eq!(ControlMsg::from_json(msg.to_json().as_bytes()), Some(msg));
        assert_eq!(
            ControlMsg::RequestKeyframe.to_json(),
            r#"{"t":"request_keyframe"}"#
        );
        let hd = ControlMsg::HighResolution { enabled: false };
        assert_eq!(hd.to_json(), r#"{"t":"high_resolution","enabled":false}"#);
        assert_eq!(ControlMsg::from_json(hd.to_json().as_bytes()), Some(hd));
    }

    #[test]
    fn input_wire_format() {
        let msg = ControlMsg::Input {
            event: InputEvent::Key {
                code: "KeyA".into(),
                down: true,
            },
        };
        assert_eq!(
            msg.to_json(),
            r#"{"t":"input","event":{"k":"key","code":"KeyA","down":true}}"#
        );
        let click = ControlMsg::Input {
            event: InputEvent::MouseButton {
                button: MouseButton::Left,
                down: false,
            },
        };
        assert_eq!(
            ControlMsg::from_json(click.to_json().as_bytes()),
            Some(click)
        );
    }

    #[test]
    fn report_wire_format() {
        let msg = ControlMsg::Report {
            received: 300,
            lost: 2,
            rtt_ms: 41,
        };
        assert_eq!(
            msg.to_json(),
            r#"{"t":"report","received":300,"lost":2,"rtt_ms":41}"#
        );
        assert_eq!(ControlMsg::from_json(msg.to_json().as_bytes()), Some(msg));
    }

    #[test]
    fn garbage_is_ignored() {
        assert_eq!(ControlMsg::from_json(b"ping 3"), None);
    }
}
