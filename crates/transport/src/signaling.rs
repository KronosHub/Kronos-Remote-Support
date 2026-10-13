//! WebSocket client for the signaling server.

use anyhow::{Context, Result, bail};
use futures_util::{SinkExt, StreamExt};
use signaling_proto::{ClientMessage, IceServer, PeerId, ServerMessage};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio_tungstenite::tungstenite::Message;

/// Interval of WebSocket pings to the signaling server.
const KEEPALIVE: std::time::Duration = std::time::Duration::from_secs(20);

/// Cloneable handle to send messages to the signaling server.
#[derive(Clone)]
pub struct SignalSender(UnboundedSender<ClientMessage>);

impl SignalSender {
    /// Queues a message. Errors are ignored: a dead connection surfaces on the receive side.
    pub fn send(&self, msg: ClientMessage) {
        let _ = self.0.send(msg);
    }

    /// A sender backed by a plain channel, so a session can be driven by a transport other
    /// than the signaling server (e.g. a direct LAN connection). The receiver gets every
    /// [`ClientMessage`] the session produces.
    pub fn channel() -> (Self, UnboundedReceiver<ClientMessage>) {
        let (tx, rx) = mpsc::unbounded_channel();
        (Self(tx), rx)
    }
}

pub struct SignalingClient {
    pub id: PeerId,
    pub ice_servers: Vec<IceServer>,
    sender: SignalSender,
    inbox: UnboundedReceiver<ServerMessage>,
}

impl SignalingClient {
    /// Connects to `url` (`ws://` or `wss://`) and registers with `device_key`.
    pub async fn connect(url: &str, device_key: &str) -> Result<Self> {
        let (socket, _) = tokio_tungstenite::connect_async(url)
            .await
            .with_context(|| format!("connecting to signaling server {url}"))?;
        let (mut sink, mut stream) = socket.split();

        let (out_tx, mut out_rx) = mpsc::unbounded_channel::<ClientMessage>();
        tokio::spawn(async move {
            // Keepalive: an idle WebSocket was cut after exactly 60 s by a NAT on the path
            // (seen with Parallels); a ping every 20 s keeps it open.
            let mut keepalive = tokio::time::interval(KEEPALIVE);
            keepalive.tick().await;
            loop {
                let frame = tokio::select! {
                    msg = out_rx.recv() => match msg {
                        Some(msg) => Message::text(msg.to_json()),
                        None => break,
                    },
                    _ = keepalive.tick() => Message::Ping(Default::default()),
                };
                if sink.send(frame).await.is_err() {
                    break;
                }
            }
            let _ = sink.close().await;
        });

        let (in_tx, mut inbox) = mpsc::unbounded_channel::<ServerMessage>();
        tokio::spawn(async move {
            while let Some(Ok(frame)) = stream.next().await {
                match frame {
                    Message::Text(text) => match serde_json::from_str(text.as_str()) {
                        Ok(msg) => {
                            if in_tx.send(msg).is_err() {
                                break;
                            }
                        }
                        Err(e) => tracing::warn!("invalid server message: {e}"),
                    },
                    Message::Close(_) => break,
                    _ => {}
                }
            }
            tracing::debug!("signaling connection closed");
        });

        let sender = SignalSender(out_tx);
        sender.send(ClientMessage::Register {
            device_key: device_key.to_owned(),
        });
        match inbox.recv().await {
            Some(ServerMessage::Registered { id, ice_servers }) => Ok(Self {
                id,
                ice_servers,
                sender,
                inbox,
            }),
            Some(ServerMessage::Error { code, message }) => {
                bail!("registration refused ({code:?}): {message}")
            }
            Some(other) => bail!("unexpected reply to register: {other:?}"),
            None => bail!("signaling server closed the connection during registration"),
        }
    }

    pub fn sender(&self) -> SignalSender {
        self.sender.clone()
    }

    pub fn send(&self, msg: ClientMessage) {
        self.sender.send(msg);
    }

    /// Next message from the server; `None` once the connection is closed.
    pub async fn recv(&mut self) -> Option<ServerMessage> {
        self.inbox.recv().await
    }
}
