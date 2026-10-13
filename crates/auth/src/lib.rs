//! Session password and mutual authentication between viewer and host.
//!
//! Both sides run symmetric SPAKE2 with the session password, then prove knowledge of the
//! derived key with an HMAC over a transcript that includes both DTLS fingerprints.
//! Consequences:
//! - the password never leaves the device, not even encrypted;
//! - an eavesdropper (or the signaling server) cannot brute-force it offline;
//! - a man in the middle who swapped DTLS certificates during signaling produces a
//!   transcript mismatch, so the confirmation fails even with the right password.

use hmac::{Hmac, Mac};
use rand::RngExt;
use sha2::Sha256;
use spake2::{Ed25519Group, Identity, Password, Spake2};

const PASSWORD_ALPHABET: &[u8] = b"abcdefghijkmnpqrstuvwxyz23456789";
/// Digits of the alphabet (0 and 1 excluded as look-alikes of o and l).
const PASSWORD_DIGITS: &[u8] = b"23456789";
/// 8 characters, at least 5 of them digits (easier to dictate and type): about 34 bits,
/// enough for online guessing only (PAKE) with rate limiting, and it changes after every
/// session.
pub const PASSWORD_LEN: usize = 8;
pub const PASSWORD_MIN_DIGITS: usize = 5;
const PROTOCOL_ID: &[u8] = b"desktop-remoto/auth/v1";

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum AuthError {
    #[error("malformed handshake message")]
    Malformed,
    #[error("wrong password or tampered connection")]
    BadProof,
}

/// Random lowercase alphanumeric password of [`PASSWORD_LEN`] characters, at least
/// [`PASSWORD_MIN_DIGITS`] of them digits, in random positions. Look-alike characters
/// (l, o, 0, 1) are excluded.
pub fn generate_password() -> String {
    let mut rng = rand::rng();
    let mut chars: Vec<u8> = (0..PASSWORD_LEN)
        .map(|i| {
            let set = if i < PASSWORD_MIN_DIGITS {
                PASSWORD_DIGITS
            } else {
                PASSWORD_ALPHABET
            };
            set[rng.random_range(0..set.len())]
        })
        .collect();
    // Fisher–Yates: spread the guaranteed digits over random positions.
    for i in (1..chars.len()).rev() {
        chars.swap(i, rng.random_range(0..=i));
    }
    chars.into_iter().map(char::from).collect()
}

/// Normalizes user input: trims, lowercases, drops inner spaces.
pub fn normalize_password(input: &str) -> String {
    input
        .chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_lowercase)
        .collect()
}

/// Minimum length of a personal (unattended access) password.
pub const PERSONAL_PASSWORD_MIN_LEN: usize = 8;
const PERSONAL_KDF_ROUNDS: u32 = 100_000;

/// Checks a personal password chosen by the user.
pub fn validate_personal_password(password: &str) -> Result<(), &'static str> {
    if password.chars().count() < PERSONAL_PASSWORD_MIN_LEN {
        return Err("la password personale deve avere almeno 8 caratteri");
    }
    if password.trim() != password {
        return Err("la password personale non può iniziare o finire con spazi");
    }
    Ok(())
}

/// Secret both sides use for SPAKE2 in unattended sessions, derived from the personal
/// password and the host's id. Only this value is stored, never the password itself, and
/// it is useless for any other device. Lowercase hex, so it survives password normalization.
pub fn derive_personal_secret(password: &str, host_id: &str) -> String {
    let salt = format!("desktop-remoto/personal/v1/{host_id}");
    let mut out = [0u8; 32];
    pbkdf2::pbkdf2_hmac::<Sha256>(
        password.as_bytes(),
        salt.as_bytes(),
        PERSONAL_KDF_ROUNDS,
        &mut out,
    );
    out.iter().map(|b| format!("{b:02x}")).collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Viewer,
    Host,
}

impl Side {
    fn label(self) -> &'static [u8] {
        match self {
            Side::Viewer => b"viewer",
            Side::Host => b"host",
        }
    }
}

/// DTLS certificate fingerprints of both ends, as seen by this peer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transcript {
    pub viewer_fingerprint: String,
    pub host_fingerprint: String,
}

/// First step: holds SPAKE2 state until the peer's message arrives.
pub struct Handshake {
    state: Spake2<Ed25519Group>,
    side: Side,
}

/// Shared key after SPAKE2. Only valid once the peer's proof has been verified.
pub struct SessionKey {
    key: Vec<u8>,
    side: Side,
    transcript: Transcript,
}

impl Handshake {
    /// Returns the handshake state and the message to send to the peer.
    pub fn start(password: &str, side: Side) -> (Self, Vec<u8>) {
        let (state, msg) = Spake2::<Ed25519Group>::start_symmetric(
            &Password::new(normalize_password(password).as_bytes()),
            &Identity::new(PROTOCOL_ID),
        );
        (Self { state, side }, msg)
    }

    /// Consumes the peer's SPAKE2 message. A wrong password still yields a key; it is the
    /// proofs exchanged afterwards that detect the mismatch.
    pub fn finish(self, peer_msg: &[u8], transcript: Transcript) -> Result<SessionKey, AuthError> {
        let key = self
            .state
            .finish(peer_msg)
            .map_err(|_| AuthError::Malformed)?;
        Ok(SessionKey {
            key,
            side: self.side,
            transcript,
        })
    }
}

impl SessionKey {
    fn mac_for(&self, side: Side) -> Hmac<Sha256> {
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.key).expect("HMAC accepts any key size");
        mac.update(PROTOCOL_ID);
        mac.update(side.label());
        for part in [
            &self.transcript.viewer_fingerprint,
            &self.transcript.host_fingerprint,
        ] {
            mac.update(&(part.len() as u32).to_be_bytes());
            mac.update(part.as_bytes());
        }
        mac
    }

    /// Proof that this side knows the key.
    pub fn proof(&self) -> Vec<u8> {
        self.mac_for(self.side).finalize().into_bytes().to_vec()
    }

    /// Checks the other side's proof in constant time.
    pub fn verify_peer(&self, proof: &[u8]) -> Result<(), AuthError> {
        let peer = match self.side {
            Side::Viewer => Side::Host,
            Side::Host => Side::Viewer,
        };
        self.mac_for(peer)
            .verify_slice(proof)
            .map_err(|_| AuthError::BadProof)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transcript() -> Transcript {
        Transcript {
            viewer_fingerprint: "sha-256 AA:BB".into(),
            host_fingerprint: "sha-256 CC:DD".into(),
        }
    }

    fn run(
        viewer_pw: &str,
        host_pw: &str,
        viewer_t: Transcript,
        host_t: Transcript,
    ) -> (bool, bool) {
        let (v, v_msg) = Handshake::start(viewer_pw, Side::Viewer);
        let (h, h_msg) = Handshake::start(host_pw, Side::Host);
        let v_key = v.finish(&h_msg, viewer_t).unwrap();
        let h_key = h.finish(&v_msg, host_t).unwrap();
        (
            h_key.verify_peer(&v_key.proof()).is_ok(),
            v_key.verify_peer(&h_key.proof()).is_ok(),
        )
    }

    #[test]
    fn same_password_authenticates_both_ways() {
        assert_eq!(
            run("abc123xy", "abc123xy", transcript(), transcript()),
            (true, true)
        );
    }

    #[test]
    fn input_is_normalized() {
        assert_eq!(
            run(" ABC1 23XY ", "abc123xy", transcript(), transcript()),
            (true, true)
        );
    }

    #[test]
    fn wrong_password_fails() {
        assert_eq!(
            run("abc123xy", "abc123xz", transcript(), transcript()),
            (false, false)
        );
    }

    #[test]
    fn swapped_certificate_fails_even_with_right_password() {
        let mut mitm = transcript();
        mitm.host_fingerprint = "sha-256 EE:FF".into();
        assert_eq!(
            run("abc123xy", "abc123xy", mitm, transcript()),
            (false, false)
        );
    }

    #[test]
    fn proof_is_side_specific() {
        // A host cannot reflect the viewer's own proof back to it.
        let (v, v_msg) = Handshake::start("pw", Side::Viewer);
        let (h, h_msg) = Handshake::start("pw", Side::Host);
        let v_key = v.finish(&h_msg, transcript()).unwrap();
        let _ = h.finish(&v_msg, transcript()).unwrap();
        assert_eq!(v_key.verify_peer(&v_key.proof()), Err(AuthError::BadProof));
    }

    #[test]
    fn personal_secret_is_bound_to_password_and_host() {
        let a = derive_personal_secret("Segreto-123", "123456789");
        assert_eq!(a.len(), 64);
        assert_eq!(a, derive_personal_secret("Segreto-123", "123456789"));
        // Case matters, and the same password on another device gives another secret.
        assert_ne!(a, derive_personal_secret("segreto-123", "123456789"));
        assert_ne!(a, derive_personal_secret("Segreto-123", "987654321"));
        // The secret works as a SPAKE2 password on both sides.
        assert_eq!(run(&a, &a, transcript(), transcript()), (true, true));
    }

    #[test]
    fn personal_password_rules() {
        assert!(validate_personal_password("corta").is_err());
        assert!(validate_personal_password(" spazio-iniziale").is_err());
        assert!(validate_personal_password("Lunga-abbastanza").is_ok());
    }

    #[test]
    fn generated_password_format() {
        for _ in 0..500 {
            let pw = generate_password();
            assert_eq!(pw.len(), PASSWORD_LEN);
            assert!(pw.bytes().all(|b| PASSWORD_ALPHABET.contains(&b)));
            let digits = pw.bytes().filter(u8::is_ascii_digit).count();
            assert!(digits >= PASSWORD_MIN_DIGITS, "{pw}: only {digits} digits");
        }
        assert_ne!(generate_password(), generate_password());
    }
}
