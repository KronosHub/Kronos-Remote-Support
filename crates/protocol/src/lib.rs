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
    /// Host → viewer: screen sharing started with this output size; `hardware` tells
    /// whether the video chip (true) or the processor (false) encodes it. Older hosts omit it.
    StreamStarted {
        width: u32,
        height: u32,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        hardware: Option<bool>,
    },
    /// Host → viewer: screen sharing could not start or stopped (e.g. missing permission).
    StreamError {
        message: String,
    },
    /// Host → viewer: there is no screen to share yet (e.g. no monitor connected); the
    /// session stays open (chat, files) and `StreamStarted` follows once one appears.
    ScreenUnavailable {
        message: String,
    },
    /// Viewer → host: decoder lost sync (start, packet loss, decode error); send an IDR.
    RequestKeyframe,
    /// Either side ends the session. `updating`: the host quits to install an update (the
    /// viewer closes its window); older peers ignore it.
    Bye {
        reason: String,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        updating: bool,
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
    /// Viewer → host: send the host's sound (on the audio channel) or stop. Off by default.
    Audio {
        enabled: bool,
    },
    /// Host → viewer: the sound cannot be sent (unsupported system, capture failed).
    AudioUnavailable {
        message: String,
    },
    /// Either side: the clipboard file transfer under way was cancelled; the sender stops,
    /// the receiver drops what it got. Older peers ignore it.
    TransferCancelled,
    /// Viewer → host: block the host's own mouse and keyboard during the session, or not.
    BlockInput {
        enabled: bool,
    },
    /// Host → viewer: this computer may set its screen to the viewer's window size (option
    /// "used only remotely", ADR 0061). Older viewers ignore it.
    ResolutionAdjustable,
    /// Viewer → host: set the shared screen to the mode closest to `width`×`height` device
    /// pixels (`scale` of them per point); 0×0 brings the host's own resolution back. Hosts
    /// without the option ignore it.
    MatchResolution {
        width: u32,
        height: u32,
        scale: f32,
    },
    /// Host → viewer: whether the host's own input is blocked now (after a request, or when
    /// the person at the host released it with Esc ×3); `message` explains a failure.
    InputBlocked {
        blocked: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        message: Option<String>,
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
            hardware: Some(true),
        };
        assert_eq!(
            msg.to_json(),
            r#"{"t":"stream_started","width":1920,"height":1080,"hardware":true}"#
        );
        assert_eq!(ControlMsg::from_json(msg.to_json().as_bytes()), Some(msg));
        // From a host before 1.9.1.
        assert_eq!(
            ControlMsg::from_json(br#"{"t":"stream_started","width":1920,"height":1080}"#),
            Some(ControlMsg::StreamStarted {
                width: 1920,
                height: 1080,
                hardware: None,
            })
        );
        // Older peers send and expect no `updating`.
        let bye = ControlMsg::Bye {
            reason: "x".into(),
            updating: false,
        };
        assert_eq!(bye.to_json(), r#"{"t":"bye","reason":"x"}"#);
        assert_eq!(
            ControlMsg::from_json(br#"{"t":"bye","reason":"x","updating":true}"#),
            Some(ControlMsg::Bye {
                reason: "x".into(),
                updating: true,
            })
        );
        assert_eq!(
            ControlMsg::ResolutionAdjustable.to_json(),
            r#"{"t":"resolution_adjustable"}"#
        );
        let msg = ControlMsg::MatchResolution {
            width: 2560,
            height: 1440,
            scale: 2.0,
        };
        assert_eq!(
            msg.to_json(),
            r#"{"t":"match_resolution","width":2560,"height":1440,"scale":2.0}"#
        );
        assert_eq!(ControlMsg::from_json(msg.to_json().as_bytes()), Some(msg));
        assert_eq!(
            ControlMsg::RequestKeyframe.to_json(),
            r#"{"t":"request_keyframe"}"#
        );
        let hd = ControlMsg::HighResolution { enabled: false };
        assert_eq!(hd.to_json(), r#"{"t":"high_resolution","enabled":false}"#);
        assert_eq!(ControlMsg::from_json(hd.to_json().as_bytes()), Some(hd));
        let audio = ControlMsg::Audio { enabled: true };
        assert_eq!(audio.to_json(), r#"{"t":"audio","enabled":true}"#);
        assert_eq!(
            ControlMsg::from_json(audio.to_json().as_bytes()),
            Some(audio)
        );
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
