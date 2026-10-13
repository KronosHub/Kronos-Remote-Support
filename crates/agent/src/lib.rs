//! The Kronos agent (ADR 0033).
//!
//! Windows draws UAC prompts and the lock screen on a separate, secure desktop that a
//! process started by the user can neither see nor control. A Windows service running as
//! SYSTEM starts this agent in the user's session; the agent follows the desktop receiving
//! input, captures it and injects mouse and keyboard there. The app (running as the user)
//! talks to it over a local named pipe and keeps doing everything else: signaling, WebRTC,
//! encoding, approvals.
//!
//! Wire format on the pipe: `u32` LE length of what follows, then a kind byte —
//! `J` + JSON [`Request`]/[`Reply`], or `F` + width `u32` LE + height `u32` LE + BGRA pixels.

pub mod client;
pub mod server;
#[cfg(target_os = "windows")]
pub mod windows;

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

/// Bumped on incompatible protocol changes (app and agent come from the same build).
pub const PROTOCOL_VERSION: u32 = 1;
/// Largest message accepted (a 4K BGRA frame plus header).
const MAX_MESSAGE: usize = 3840 * 2160 * 4 + 64;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Request {
    Hello {
        version: u32,
    },
    StartCapture {
        display_id: Option<u32>,
        max_size: Option<(u32, u32)>,
        max_fps: u32,
        show_cursor: bool,
    },
    StartInput,
    Input {
        event: protocol::InputEvent,
    },
    SetArea {
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Reply {
    Hello {
        version: u32,
    },
    CaptureStarted {
        width: u32,
        height: u32,
        display_id: u32,
        bounds: (f64, f64, f64, f64),
    },
    InputStarted,
    Error {
        message: String,
    },
}

/// One message on the pipe.
#[derive(Debug, Clone, PartialEq)]
pub enum Wire<T> {
    Json(T),
    Frame {
        width: u32,
        height: u32,
        data: Vec<u8>,
    },
}

pub async fn write_message<W, T>(w: &mut W, message: &Wire<T>) -> std::io::Result<()>
where
    W: AsyncWrite + Unpin,
    T: Serialize,
{
    match message {
        Wire::Json(value) => {
            let json = serde_json::to_vec(value).map_err(std::io::Error::other)?;
            w.write_u32_le(json.len() as u32 + 1).await?;
            w.write_u8(b'J').await?;
            w.write_all(&json).await?;
        }
        Wire::Frame {
            width,
            height,
            data,
        } => {
            w.write_u32_le(data.len() as u32 + 9).await?;
            w.write_u8(b'F').await?;
            w.write_u32_le(*width).await?;
            w.write_u32_le(*height).await?;
            w.write_all(data).await?;
        }
    }
    w.flush().await
}

/// Next message; `Ok(None)` at a clean end of stream.
pub async fn read_message<R, T>(r: &mut R) -> std::io::Result<Option<Wire<T>>>
where
    R: AsyncRead + Unpin,
    T: for<'de> Deserialize<'de>,
{
    let len = match r.read_u32_le().await {
        Ok(len) => len as usize,
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    };
    if len == 0 || len > MAX_MESSAGE {
        return Err(std::io::Error::other(format!("bad message length {len}")));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body).await?;
    match body[0] {
        b'J' => serde_json::from_slice(&body[1..])
            .map(|v| Some(Wire::Json(v)))
            .map_err(std::io::Error::other),
        b'F' if len >= 9 => {
            let width = u32::from_le_bytes(body[1..5].try_into().expect("4 bytes"));
            let height = u32::from_le_bytes(body[5..9].try_into().expect("4 bytes"));
            if (width as usize) * (height as usize) * 4 != len - 9 {
                return Err(std::io::Error::other("frame size mismatch"));
            }
            body.drain(..9);
            Ok(Some(Wire::Frame {
                width,
                height,
                data: body,
            }))
        }
        other => Err(std::io::Error::other(format!("bad message kind {other}"))),
    }
}
