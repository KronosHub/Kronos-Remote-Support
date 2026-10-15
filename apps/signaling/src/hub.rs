//! In-memory routing state: who is online, which sessions exist, which device owns which id.
//!
//! All methods are synchronous and called under a mutex; nothing here awaits.

use std::collections::HashMap;
use std::path::PathBuf;

use rand::RngExt;
use sha2::{Digest, Sha256};
use signaling_proto::{ErrorCode, IceServer, PeerId, ServerMessage, SessionId, SignalPayload};
use std::sync::Arc;

use tokio::sync::Notify;
use tokio::sync::mpsc::UnboundedSender;

use crate::turn::Turn;

/// Identifies one WebSocket connection; a device that reconnects gets a new one.
pub type ConnId = u64;

pub type Outbox = UnboundedSender<ServerMessage>;

struct Online {
    conn: ConnId,
    tx: Outbox,
    /// Tells the connection's task to hang up (used when the device connects again).
    kick: Arc<Notify>,
}

pub struct Hub {
    online: HashMap<PeerId, Online>,
    sessions: HashMap<SessionId, (PeerId, PeerId)>,
    /// SHA-256(device key) -> id. The raw key never touches disk.
    ids: HashMap<String, PeerId>,
    next_session: SessionId,
    store: Option<PathBuf>,
    ice_servers: Vec<IceServer>,
    turn: Option<Turn>,
}

fn key_hash(device_key: &str) -> String {
    Sha256::digest(device_key.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

impl Hub {
    pub fn new(
        store: Option<PathBuf>,
        ice_servers: Vec<IceServer>,
        turn: Option<Turn>,
    ) -> anyhow::Result<Self> {
        let ids = match &store {
            Some(path) if path.exists() => serde_json::from_slice(&std::fs::read(path)?)?,
            _ => HashMap::new(),
        };
        Ok(Self {
            online: HashMap::new(),
            sessions: HashMap::new(),
            ids,
            next_session: 1,
            store,
            ice_servers,
            turn,
        })
    }

    /// STUN servers plus fresh TURN credentials for `peer`.
    pub fn ice_servers_for(&self, peer: &PeerId) -> Vec<IceServer> {
        let mut servers = self.ice_servers.clone();
        if let Some(turn) = &self.turn {
            servers.push(turn.credentials(peer, std::time::SystemTime::now()));
        }
        servers
    }

    fn persist(&self) {
        let Some(path) = &self.store else { return };
        let json = serde_json::to_vec_pretty(&self.ids).expect("ids map is serializable");
        // Write-then-rename so a crash never leaves a truncated file.
        let tmp = path.with_extension("tmp");
        if let Err(e) = std::fs::write(&tmp, json).and_then(|()| std::fs::rename(&tmp, path)) {
            tracing::error!("failed to persist ids to {}: {e}", path.display());
        }
    }

    fn allocate_id(&self) -> PeerId {
        let mut rng = rand::rng();
        loop {
            let id = PeerId::from_number(rng.random_range(100_000_000..=999_999_999))
                .expect("range matches PeerId");
            if !self.ids.values().any(|existing| *existing == id) {
                return id;
            }
        }
    }

    /// Registers a connection, returning the device's stable id.
    /// A previous connection of the same device is told why and hung up, and its sessions closed;
    /// otherwise it would keep believing it is online while unreachable.
    pub fn register(
        &mut self,
        conn: ConnId,
        device_key: &str,
        tx: Outbox,
        kick: Arc<Notify>,
    ) -> PeerId {
        let hash = key_hash(device_key);
        let id = match self.ids.get(&hash) {
            Some(id) => id.clone(),
            None => {
                let id = self.allocate_id();
                self.ids.insert(hash, id.clone());
                self.persist();
                id
            }
        };
        if let Some(old) = self.online.get(&id) {
            tracing::info!(%id, old_conn = old.conn, "device reconnected, replacing connection");
            let _ = old.tx.send(ServerMessage::error(
                ErrorCode::Replaced,
                "this device connected again from another session",
            ));
            old.kick.notify_one();
            self.close_sessions_of(&id, "peer reconnected");
        }
        self.online.insert(id.clone(), Online { conn, tx, kick });
        id
    }

    /// Drops a connection. No-op if the device already re-registered on a newer connection.
    pub fn disconnect(&mut self, conn: ConnId, id: &PeerId) {
        if self.online.get(id).is_some_and(|o| o.conn == conn) {
            self.online.remove(id);
            self.close_sessions_of(id, "peer disconnected");
        }
    }

    fn send(&self, to: &PeerId, msg: ServerMessage) {
        if let Some(peer) = self.online.get(to) {
            // A closed outbox means the connection is going away; disconnect() will clean up.
            let _ = peer.tx.send(msg);
        }
    }

    pub fn is_online(&self, peer: &PeerId) -> bool {
        self.online.contains_key(peer)
    }

    pub fn connect(&mut self, from: &PeerId, target: &PeerId) -> Result<(), ServerMessage> {
        if from == target {
            return Err(ServerMessage::error(
                ErrorCode::SelfConnect,
                "cannot connect to yourself",
            ));
        }
        if !self.online.contains_key(target) {
            return Err(ServerMessage::error(
                ErrorCode::PeerOffline,
                format!("peer {target} is offline"),
            ));
        }
        let session = self.next_session;
        self.next_session += 1;
        self.sessions
            .insert(session, (from.clone(), target.clone()));
        tracing::info!(session, %from, %target, "session created");

        self.send(
            target,
            ServerMessage::IncomingSession {
                session,
                from: from.clone(),
                ice_servers: self.ice_servers_for(target),
            },
        );
        self.send(
            from,
            ServerMessage::SessionCreated {
                session,
                peer: target.clone(),
                ice_servers: self.ice_servers_for(from),
            },
        );
        Ok(())
    }

    fn other_party(&self, session: SessionId, me: &PeerId) -> Option<&PeerId> {
        let (a, b) = self.sessions.get(&session)?;
        if a == me {
            Some(b)
        } else if b == me {
            Some(a)
        } else {
            None
        }
    }

    pub fn relay(
        &self,
        from: &PeerId,
        session: SessionId,
        payload: SignalPayload,
    ) -> Result<(), ServerMessage> {
        let to = self
            .other_party(session, from)
            .ok_or_else(|| unknown_session(session))?;
        self.send(to, ServerMessage::Signal { session, payload });
        Ok(())
    }

    pub fn leave(&mut self, from: &PeerId, session: SessionId) -> Result<(), ServerMessage> {
        let to = self
            .other_party(session, from)
            .ok_or_else(|| unknown_session(session))?
            .clone();
        self.sessions.remove(&session);
        self.send(
            &to,
            ServerMessage::SessionClosed {
                session,
                reason: "peer left".into(),
            },
        );
        Ok(())
    }

    fn close_sessions_of(&mut self, id: &PeerId, reason: &str) {
        let ended: Vec<_> = self
            .sessions
            .iter()
            .filter(|(_, (a, b))| a == id || b == id)
            .map(|(s, (a, b))| (*s, if a == id { b.clone() } else { a.clone() }))
            .collect();
        for (session, other) in ended {
            self.sessions.remove(&session);
            self.send(
                &other,
                ServerMessage::SessionClosed {
                    session,
                    reason: reason.into(),
                },
            );
        }
    }
}

fn unknown_session(session: SessionId) -> ServerMessage {
    ServerMessage::error(
        ErrorCode::UnknownSession,
        format!("unknown session {session}"),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::FutureExt;
    use tokio::sync::mpsc::{UnboundedReceiver, unbounded_channel};

    fn peer(hub: &mut Hub, conn: ConnId, key: &str) -> (PeerId, UnboundedReceiver<ServerMessage>) {
        let (tx, rx) = unbounded_channel();
        (hub.register(conn, key, tx, Arc::new(Notify::new())), rx)
    }

    #[test]
    fn reconnect_kicks_previous_connection() {
        let mut hub = Hub::new(None, vec![], None).unwrap();
        let (tx1, mut rx1) = unbounded_channel();
        let kick1 = Arc::new(Notify::new());
        let id = hub.register(1, "dev", tx1, kick1.clone());
        let (id2, _rx2) = peer(&mut hub, 2, "dev");
        assert_eq!(id, id2);
        assert!(matches!(
            rx1.try_recv().unwrap(),
            ServerMessage::Error {
                code: ErrorCode::Replaced,
                ..
            }
        ));
        // notify_one stores a permit, so the old task wakes even if not yet waiting.
        assert!(kick1.notified().now_or_never().is_some());
        // The stale connection hanging up must not unregister the new one.
        hub.disconnect(1, &id);
        let (b, _rx_b) = peer(&mut hub, 3, "other");
        assert!(hub.connect(&b, &id).is_ok());
    }

    #[test]
    fn same_key_same_id() {
        let mut hub = Hub::new(None, vec![], None).unwrap();
        let (a, _rx1) = peer(&mut hub, 1, "key-a");
        let (a2, _rx2) = peer(&mut hub, 2, "key-a");
        let (b, _rx3) = peer(&mut hub, 3, "key-b");
        assert_eq!(a, a2);
        assert_ne!(a, b);
    }

    #[test]
    fn session_relays_only_between_parties() {
        let mut hub = Hub::new(None, vec![], None).unwrap();
        let (a, mut rx_a) = peer(&mut hub, 1, "a");
        let (b, mut rx_b) = peer(&mut hub, 2, "b");
        let (c, _rx_c) = peer(&mut hub, 3, "c");

        hub.connect(&a, &b).unwrap();
        let ServerMessage::SessionCreated { session, .. } = rx_a.try_recv().unwrap() else {
            panic!("expected SessionCreated");
        };
        assert!(matches!(
            rx_b.try_recv().unwrap(),
            ServerMessage::IncomingSession { .. }
        ));

        let payload = SignalPayload::Offer { sdp: "x".into() };
        hub.relay(&a, session, payload.clone()).unwrap();
        assert_eq!(
            rx_b.try_recv().unwrap(),
            ServerMessage::Signal {
                session,
                payload: payload.clone()
            }
        );
        // An outsider cannot inject into the session.
        assert!(hub.relay(&c, session, payload).is_err());
    }

    #[test]
    fn offline_and_self_connect_rejected() {
        let mut hub = Hub::new(None, vec![], None).unwrap();
        let (a, _rx) = peer(&mut hub, 1, "a");
        let offline = PeerId::from_number(123_456_789).unwrap();
        assert!(hub.connect(&a, &offline).is_err());
        assert!(hub.connect(&a, &a).is_err());
    }

    #[test]
    fn disconnect_closes_sessions_but_ignores_stale_connection() {
        let mut hub = Hub::new(None, vec![], None).unwrap();
        let (a, _rx_a) = peer(&mut hub, 1, "a");
        let (b, mut rx_b) = peer(&mut hub, 2, "b");
        hub.connect(&a, &b).unwrap();
        rx_b.try_recv().unwrap(); // IncomingSession

        // Stale connection id: nothing happens.
        hub.disconnect(99, &a);
        assert!(rx_b.try_recv().is_err());

        hub.disconnect(1, &a);
        assert!(matches!(
            rx_b.try_recv().unwrap(),
            ServerMessage::SessionClosed { .. }
        ));
    }

    #[test]
    fn ids_survive_restart() {
        let dir = std::env::temp_dir().join(format!("signaling-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = dir.join("ids.json");

        let mut hub = Hub::new(Some(store.clone()), vec![], None).unwrap();
        let (id, _rx) = peer(&mut hub, 1, "persistent");
        drop(hub);

        let mut hub = Hub::new(Some(store), vec![], None).unwrap();
        let (again, _rx) = peer(&mut hub, 1, "persistent");
        assert_eq!(id, again);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
