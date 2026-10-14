//! Password authentication + host approval, run on the control channel right after it opens.
//!
//! ```text
//! viewer                              host
//!   spake{msg, personal}  ──────────▶
//!                         ◀──────────   spake{msg}
//!   proof{mac}            ──────────▶   verify, then ask the local user (temporary password only)
//!                         ◀──────────   accepted{mac} | denied{reason}
//! ```
//!
//! `personal = true`: the viewer uses the host's personal password (unattended access).
//! SPAKE2 then runs on the secret derived from it, and no approval is asked.

use std::future::Future;
use std::sync::Arc;

use auth::{AuthError, Handshake, Side, Transcript};
use serde::{Deserialize, Serialize};
use webrtc::data_channel::{DataChannel, DataChannelEvent};

#[derive(Debug, thiserror::Error)]
pub enum HandshakeError {
    #[error("password errata")]
    BadPassword,
    #[error("connessione rifiutata: {0}")]
    Denied(String),
    #[error("risposta dell'host non autentica (possibile intercettazione)")]
    BadHostProof,
    #[error("protocollo non valido: {0}")]
    Protocol(String),
    #[error("canale chiuso durante l'autenticazione")]
    Closed,
    #[error("errore di invio: {0}")]
    Send(String),
}

/// Why the host refused a connection; sent to the viewer as text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    Rejected,
    TooManyAttempts,
}

impl Refusal {
    fn reason(self) -> &'static str {
        match self {
            Refusal::Rejected => "l'utente remoto ha rifiutato",
            Refusal::TooManyAttempts => "troppi tentativi falliti, riprova più tardi",
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
enum AuthMsg {
    Spake {
        msg: String,
        /// Viewer side only: authenticating with the personal password.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        personal: bool,
    },
    Proof {
        mac: String,
        /// The viewer's user name (1.4+): sent only once the password checked out, so a
        /// stranger guessing passwords never learns it. Older hosts ignore it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    Accepted {
        mac: String,
        /// The host's user name (1.4+).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    Denied {
        reason: String,
    },
}

/// Longest user name shown to the partner.
pub const MAX_NAME_CHARS: usize = 40;

/// A user name fit to show: trimmed, without control characters, at most
/// [`MAX_NAME_CHARS`]; `None` if nothing is left. Applied to our own name before sending and
/// to the partner's on arrival (it comes from the other side).
pub fn clean_name(name: &str) -> Option<String> {
    let name: String = name
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_NAME_CHARS)
        .collect();
    (!name.is_empty()).then_some(name)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(s: &str) -> Result<Vec<u8>, HandshakeError> {
    // Bytes, not string slices: a multi-byte character from a hostile peer must not panic.
    let bytes = s.as_bytes();
    if !bytes.len().is_multiple_of(2) {
        return Err(HandshakeError::Protocol("hex length".into()));
    }
    bytes
        .chunks(2)
        .map(|pair| {
            let digit = |b: u8| char::from(b).to_digit(16);
            match (digit(pair[0]), digit(pair[1])) {
                (Some(hi), Some(lo)) => Ok((hi * 16 + lo) as u8),
                _ => Err(HandshakeError::Protocol("hex digit".into())),
            }
        })
        .collect()
}

async fn send(channel: &Arc<dyn DataChannel>, msg: &AuthMsg) -> Result<(), HandshakeError> {
    let text = serde_json::to_string(msg).expect("AuthMsg is serializable");
    channel
        .send_text(&text)
        .await
        .map_err(|e| HandshakeError::Send(e.to_string()))
}

async fn recv(channel: &Arc<dyn DataChannel>) -> Result<AuthMsg, HandshakeError> {
    loop {
        match channel.poll().await {
            Some(DataChannelEvent::OnMessage(msg)) => {
                return serde_json::from_slice(&msg.data)
                    .map_err(|e| HandshakeError::Protocol(e.to_string()));
            }
            Some(DataChannelEvent::OnClose) | None => return Err(HandshakeError::Closed),
            _ => {}
        }
    }
}

fn map_auth(e: AuthError) -> HandshakeError {
    match e {
        AuthError::Malformed => HandshakeError::Protocol("spake message".into()),
        AuthError::BadProof => HandshakeError::BadPassword,
    }
}

/// Which password the host accepted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    /// Temporary password + explicit approval by the local user.
    Approved,
    /// Personal password: unattended access, no approval.
    Unattended,
}

/// Passwords the host accepts.
pub struct HostSecrets<'a> {
    pub temporary: &'a str,
    /// Derived personal secret (`auth::derive_personal_secret`), if unattended access is on.
    pub personal: Option<&'a str>,
}

fn unexpected(what: &str, got: &AuthMsg) -> HandshakeError {
    HandshakeError::Protocol(format!("expected {what}, got {got:?}"))
}

/// Viewer side. Resolves once the host verified the password and, for the temporary
/// password, its user accepted. With `personal`, `password` is the derived personal secret.
/// Returns the host's user name, if it set one. `name` is ours, sent once the password checked out.
pub async fn authenticate_viewer(
    channel: &Arc<dyn DataChannel>,
    password: &str,
    personal: bool,
    transcript: Transcript,
    name: Option<&str>,
) -> Result<Option<String>, HandshakeError> {
    let (handshake, msg) = Handshake::start(password, Side::Viewer);
    send(
        channel,
        &AuthMsg::Spake {
            msg: hex(&msg),
            personal,
        },
    )
    .await?;

    let key = match recv(channel).await? {
        AuthMsg::Spake { msg, .. } => handshake
            .finish(&unhex(&msg)?, transcript)
            .map_err(map_auth)?,
        AuthMsg::Denied { reason } => return Err(HandshakeError::Denied(reason)),
        other => return Err(unexpected("spake", &other)),
    };
    send(
        channel,
        &AuthMsg::Proof {
            mac: hex(&key.proof()),
            name: name.and_then(clean_name),
        },
    )
    .await?;

    match recv(channel).await? {
        AuthMsg::Accepted { mac, name } => {
            key.verify_peer(&unhex(&mac)?)
                .map_err(|_| HandshakeError::BadHostProof)?;
            Ok(name.as_deref().and_then(clean_name))
        }
        AuthMsg::Denied { reason } if reason == "password errata" => {
            Err(HandshakeError::BadPassword)
        }
        AuthMsg::Denied { reason } => Err(HandshakeError::Denied(reason)),
        other => Err(unexpected("result", &other)),
    }
}

/// Host side. `refuse_early` is told whether the viewer uses the personal password and may
/// block before any crypto (rate limiting); `approve` (given the viewer's user name) is
/// awaited only for the temporary password, after it checked out. `name` is ours, sent
/// with the acceptance. Returns the access granted and the viewer's user name.
pub async fn authenticate_host<R, A, F>(
    channel: &Arc<dyn DataChannel>,
    secrets: HostSecrets<'_>,
    transcript: Transcript,
    refuse_early: R,
    approve: A,
    name: Option<&str>,
) -> Result<(Access, Option<String>), HandshakeError>
where
    R: FnOnce(bool) -> Option<Refusal>,
    A: FnOnce(Option<String>) -> F,
    F: Future<Output = bool>,
{
    let (peer_msg, personal) = match recv(channel).await? {
        AuthMsg::Spake { msg, personal } => (unhex(&msg)?, personal),
        other => return Err(unexpected("spake", &other)),
    };
    if let Some(refusal) = refuse_early(personal) {
        deny(channel, refusal.reason()).await;
        return Err(HandshakeError::Denied(refusal.reason().into()));
    }
    let password = match (personal, secrets.personal) {
        (false, _) => secrets.temporary,
        (true, Some(secret)) => secret,
        (true, None) => {
            let reason = "accesso non presidiato non attivo su questo dispositivo";
            deny(channel, reason).await;
            return Err(HandshakeError::Denied(reason.into()));
        }
    };

    let (handshake, msg) = Handshake::start(password, Side::Host);
    let key = handshake.finish(&peer_msg, transcript).map_err(map_auth)?;
    send(
        channel,
        &AuthMsg::Spake {
            msg: hex(&msg),
            personal: false,
        },
    )
    .await?;

    let (proof, peer_name) = match recv(channel).await? {
        AuthMsg::Proof { mac, name } => (unhex(&mac)?, name.as_deref().and_then(clean_name)),
        other => return Err(unexpected("proof", &other)),
    };
    if key.verify_peer(&proof).is_err() {
        deny(channel, "password errata").await;
        return Err(HandshakeError::BadPassword);
    }

    let access = if personal {
        Access::Unattended
    } else if approve(peer_name.clone()).await {
        Access::Approved
    } else {
        deny(channel, Refusal::Rejected.reason()).await;
        return Err(HandshakeError::Denied(Refusal::Rejected.reason().into()));
    };
    send(
        channel,
        &AuthMsg::Accepted {
            mac: hex(&key.proof()),
            name: name.and_then(clean_name),
        },
    )
    .await?;
    Ok((access, peer_name))
}

async fn deny(channel: &Arc<dyn DataChannel>, reason: &str) {
    let _ = send(
        channel,
        &AuthMsg::Denied {
            reason: reason.into(),
        },
    )
    .await;
}

/// Extracts the DTLS certificate fingerprint (`a=fingerprint:...`) from an SDP.
pub(crate) fn sdp_fingerprint(sdp: &str) -> Option<String> {
    sdp.lines()
        .find_map(|line| line.trim().strip_prefix("a=fingerprint:"))
        .map(|fp| fp.trim().to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_roundtrip() {
        let bytes = [0u8, 1, 0xab, 0xff];
        assert_eq!(unhex(&hex(&bytes)).unwrap(), bytes);
        assert!(unhex("abc").is_err());
        assert!(unhex("zz").is_err());
        // Multi-byte characters are rejected, not a panic on a char boundary.
        assert!(unhex("éé").is_err());
        assert!(unhex("aé").is_err());
    }

    #[test]
    fn fingerprint_from_sdp() {
        let sdp = "v=0\r\no=- 1 2 IN IP4 0.0.0.0\r\nm=application 9 UDP/DTLS/SCTP webrtc-datachannel\r\na=fingerprint:sha-256 AB:CD:EF\r\n";
        assert_eq!(sdp_fingerprint(sdp).as_deref(), Some("sha-256 ab:cd:ef"));
        assert_eq!(sdp_fingerprint("v=0\r\n"), None);
    }

    #[test]
    fn wire_format() {
        let json = serde_json::to_string(&AuthMsg::Denied { reason: "x".into() }).unwrap();
        assert_eq!(json, r#"{"t":"denied","reason":"x"}"#);
        // Older peers omit `personal`; it defaults to the temporary password.
        let old: AuthMsg = serde_json::from_str(r#"{"t":"spake","msg":"00"}"#).unwrap();
        assert!(matches!(
            old,
            AuthMsg::Spake {
                personal: false,
                ..
            }
        ));
        let personal = serde_json::to_string(&AuthMsg::Spake {
            msg: "00".into(),
            personal: true,
        })
        .unwrap();
        assert_eq!(personal, r#"{"t":"spake","msg":"00","personal":true}"#);
        // Names are optional both ways: older peers neither send nor expect them.
        let old: AuthMsg = serde_json::from_str(r#"{"t":"proof","mac":"00"}"#).unwrap();
        assert!(matches!(old, AuthMsg::Proof { name: None, .. }));
        let unnamed = AuthMsg::Accepted {
            mac: "00".into(),
            name: None,
        };
        assert_eq!(
            serde_json::to_string(&unnamed).unwrap(),
            r#"{"t":"accepted","mac":"00"}"#
        );
    }

    #[test]
    fn names_are_cleaned() {
        assert_eq!(
            clean_name("  Alberto \n Cioffi\t").as_deref(),
            Some("Alberto Cioffi")
        );
        assert_eq!(clean_name(" \u{7}  "), None);
        assert_eq!(
            clean_name(&"x".repeat(100)).map(|n| n.len()),
            Some(MAX_NAME_CHARS)
        );
    }
}
