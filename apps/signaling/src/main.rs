//! Signaling server. Plain HTTP/WebSocket; TLS is terminated by a reverse proxy (Caddy).

mod hub;
mod turn;

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use axum::Router;
use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::IntoResponse;
use axum::routing::get;
use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use hub::Hub;
use signaling_proto::{ClientMessage, ErrorCode, IceServer, PeerId, ServerMessage};
use tokio::sync::{Notify, mpsc};

/// Largest accepted client message; SDP offers are a few KiB.
const MAX_MESSAGE_BYTES: usize = 64 * 1024;
const REGISTER_TIMEOUT: Duration = Duration::from_secs(10);
/// At most this many `Connect` requests per connection per minute (slows id guessing).
const CONNECTS_PER_MINUTE: usize = 10;

#[derive(Parser)]
#[command(about = "Desktop_remoto signaling server")]
struct Args {
    #[arg(long, default_value = "0.0.0.0:8080")]
    listen: SocketAddr,
    /// Where device ids are persisted (ids.json). Omit to keep them in memory only.
    #[arg(long)]
    data_dir: Option<PathBuf>,
    /// STUN url handed to clients; repeatable.
    #[arg(long = "ice-server", default_value = "stun:stun.l.google.com:19302")]
    ice_servers: Vec<String>,
    /// TURN url (e.g. turn:203.0.113.10:3478?transport=udp); repeatable. Needs --turn-secret-file.
    #[arg(long = "turn-url")]
    turn_urls: Vec<String>,
    /// File holding the secret shared with coturn (`static-auth-secret`).
    #[arg(long)]
    turn_secret_file: Option<PathBuf>,
    /// Lifetime of issued TURN credentials.
    #[arg(long, default_value_t = 12)]
    turn_ttl_hours: u64,
}

#[derive(Clone)]
struct AppState {
    hub: Arc<Mutex<Hub>>,
    next_conn: Arc<AtomicU64>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let args = Args::parse();

    let store = match &args.data_dir {
        Some(dir) => {
            std::fs::create_dir_all(dir)?;
            Some(dir.join("ids.json"))
        }
        None => None,
    };
    let ice_servers = args
        .ice_servers
        .iter()
        .map(|url| IceServer {
            urls: vec![url.clone()],
            username: String::new(),
            credential: String::new(),
        })
        .collect();

    let turn = match (&args.turn_secret_file, args.turn_urls.is_empty()) {
        (Some(path), false) => {
            let secret = std::fs::read_to_string(path)?.trim().as_bytes().to_vec();
            anyhow::ensure!(
                !secret.is_empty(),
                "empty TURN secret in {}",
                path.display()
            );
            tracing::info!(urls = ?args.turn_urls, "TURN relay enabled");
            Some(turn::Turn::new(
                args.turn_urls.clone(),
                secret,
                Duration::from_secs(args.turn_ttl_hours * 3600),
            ))
        }
        (None, true) => None,
        _ => anyhow::bail!("--turn-url and --turn-secret-file must be given together"),
    };

    let state = AppState {
        hub: Arc::new(Mutex::new(Hub::new(store, ice_servers, turn)?)),
        next_conn: Arc::new(AtomicU64::new(1)),
    };
    let app = Router::new()
        .route("/health", get(|| async { "ok" }))
        .route("/ws", get(ws_handler))
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(args.listen).await?;
    tracing::info!("listening on {}", args.listen);
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.max_message_size(MAX_MESSAGE_BYTES)
        .on_upgrade(move |socket| handle_socket(socket, state))
}

async fn handle_socket(socket: WebSocket, state: AppState) {
    let conn = state.next_conn.fetch_add(1, Ordering::Relaxed);
    let (mut sink, mut stream) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel::<ServerMessage>();
    let kick = Arc::new(Notify::new());

    let writer = tokio::spawn(async move {
        while let Some(msg) = rx.recv().await {
            if sink
                .send(Message::Text(msg.to_json().into()))
                .await
                .is_err()
            {
                break;
            }
        }
    });

    let id = match tokio::time::timeout(REGISTER_TIMEOUT, next_client_message(&mut stream)).await {
        Ok(Some(ClientMessage::Register { device_key })) if !device_key.is_empty() => {
            let mut hub = state.hub.lock().unwrap();
            let id = hub.register(conn, &device_key, tx.clone(), kick.clone());
            let _ = tx.send(ServerMessage::Registered {
                id: id.clone(),
                ice_servers: hub.ice_servers_for(&id),
            });
            id
        }
        _ => {
            let _ = tx.send(ServerMessage::error(
                ErrorCode::NotRegistered,
                "first message must be register",
            ));
            drop(tx);
            let _ = writer.await;
            return;
        }
    };
    tracing::info!(conn, %id, "peer online");

    let mut recent_connects = VecDeque::new();
    loop {
        tokio::select! {
            msg = next_client_message(&mut stream) => {
                let Some(msg) = msg else { break };
                if let Err(err) = handle_message(&state, &id, msg, &mut recent_connects) {
                    let _ = tx.send(err);
                }
            }
            _ = kick.notified() => {
                tracing::info!(conn, %id, "connection replaced by a newer one");
                break;
            }
        }
    }

    state.hub.lock().unwrap().disconnect(conn, &id);
    tracing::info!(conn, %id, "peer offline");
    // Let the writer flush what is queued (e.g. the "replaced" error), then hang up.
    drop(tx);
    let _ = tokio::time::timeout(Duration::from_secs(1), writer).await;
}

fn handle_message(
    state: &AppState,
    id: &PeerId,
    msg: ClientMessage,
    recent_connects: &mut VecDeque<Instant>,
) -> Result<(), ServerMessage> {
    let mut hub = state.hub.lock().unwrap();
    match msg {
        ClientMessage::Register { .. } => Err(ServerMessage::error(
            ErrorCode::BadRequest,
            "already registered",
        )),
        ClientMessage::Connect { target } => {
            let now = Instant::now();
            while recent_connects
                .front()
                .is_some_and(|t| now.duration_since(*t) > Duration::from_secs(60))
            {
                recent_connects.pop_front();
            }
            if recent_connects.len() >= CONNECTS_PER_MINUTE {
                return Err(ServerMessage::error(
                    ErrorCode::RateLimited,
                    "too many connection attempts, retry later",
                ));
            }
            recent_connects.push_back(now);
            hub.connect(id, &target)
        }
        ClientMessage::Signal { session, payload } => hub.relay(id, session, payload),
        ClientMessage::Leave { session } => hub.leave(id, session),
    }
}

/// Next well-formed client message; `None` when the socket closes.
/// Malformed messages are logged and skipped.
async fn next_client_message(
    stream: &mut futures_util::stream::SplitStream<WebSocket>,
) -> Option<ClientMessage> {
    while let Some(Ok(frame)) = stream.next().await {
        match frame {
            Message::Text(text) => match serde_json::from_str(text.as_str()) {
                Ok(msg) => return Some(msg),
                Err(e) => tracing::warn!("invalid client message: {e}"),
            },
            Message::Close(_) => return None,
            _ => {}
        }
    }
    None
}
