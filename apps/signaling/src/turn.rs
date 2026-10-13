//! Short-lived TURN credentials ("TURN REST API", coturn `use-auth-secret`).
//!
//! username = "<expiry unix time>:<peer id>", password = base64(HMAC-SHA1(secret, username)).
//! coturn recomputes the HMAC with the same shared secret and rejects expired usernames,
//! so no per-user state is needed and nothing long-lived ships inside the apps.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use hmac::{Hmac, Mac};
use sha1::Sha1;
use signaling_proto::{IceServer, PeerId};

pub struct Turn {
    urls: Vec<String>,
    secret: Vec<u8>,
    ttl: Duration,
}

impl Turn {
    pub fn new(urls: Vec<String>, secret: Vec<u8>, ttl: Duration) -> Self {
        Self { urls, secret, ttl }
    }

    pub fn credentials(&self, peer: &PeerId, now: SystemTime) -> IceServer {
        let expiry = now.duration_since(UNIX_EPOCH).unwrap_or_default() + self.ttl;
        let username = format!("{}:{}", expiry.as_secs(), peer.as_str());
        let mut mac = Hmac::<Sha1>::new_from_slice(&self.secret).expect("any key size");
        mac.update(username.as_bytes());
        let credential =
            base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes());
        IceServer {
            urls: self.urls.clone(),
            username,
            credential,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_coturn_rest_api_format() {
        let turn = Turn::new(
            vec!["turn:example.org:3478".into()],
            b"secret".to_vec(),
            Duration::from_secs(3600),
        );
        let peer = PeerId::from_number(123_456_789).unwrap();
        let now = UNIX_EPOCH + Duration::from_secs(1_000_000);
        let c = turn.credentials(&peer, now);
        assert_eq!(c.username, "1003600:123456789");
        // echo -n "1003600:123456789" | openssl dgst -sha1 -hmac secret -binary | base64
        assert_eq!(c.credential, "NNZnsRIZq8DSK+C822I7sIarsTc=");
    }
}
