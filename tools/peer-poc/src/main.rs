//! Command-line peer for testing without the GUI.
//!
//!   peer-poc host [--source pattern]          # prints id + password, streams its screen
//!   peer-poc connect 123456789 -p <password>  # receives the stream, reports fps and latency
//!
//! Both sides talk to the signaling server; once WebRTC is up, everything flows peer-to-peer.

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use rand::RngExt;
use signaling_proto::{ClientMessage, PeerId, ServerMessage, SessionId};
use stream::session::{self, HostEvent, ViewerCommand, ViewerEvent};
use stream::{Source, StreamConfig};
use tokio::sync::{mpsc, oneshot};
use transport::video::VideoSender;
use transport::{PeerSession, Role, SessionInfo, SignalingClient, TrackRemote, diag, handshake};

#[derive(Parser)]
#[command(about = "Connect two peers via the signaling server and ping over WebRTC")]
struct Args {
    /// Signaling server WebSocket url.
    #[arg(long, default_value = "ws://127.0.0.1:8080/ws")]
    server: String,
    /// File holding this device's secret key (created if missing).
    /// Defaults to `peer-host.key` for `host`, and a throwaway key for `connect`.
    #[arg(long)]
    key_file: Option<PathBuf>,
    /// Force traffic through the TURN relay (tests the relay path).
    #[arg(long, global = true)]
    relay_only: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Wait for incoming sessions (auto-approved) and stream to them.
    Host {
        #[arg(long, value_enum, default_value_t = SourceArg::Screen)]
        source: SourceArg,
        /// Let viewers control mouse and keyboard (macOS: needs Accessibility permission).
        #[arg(long)]
        control: bool,
        /// Also accept this personal password (unattended access).
        #[arg(long)]
        personal_password: Option<String>,
        /// After 2 s of a session, send a chat message to the viewer.
        #[arg(long)]
        chat_reply: bool,
    },
    /// Connect to a host, receive its stream, measure latency.
    Connect {
        target: PeerId,
        /// Password shown by the host (or its personal password with `--personal`).
        #[arg(long, short)]
        password: String,
        /// `--password` is the host's personal password (unattended access).
        #[arg(long)]
        personal: bool,
        #[arg(long, default_value_t = 10)]
        seconds: u64,
        /// Write the received H.264 stream here (play with `ffplay -f h264 <file>`).
        #[arg(long)]
        save: Option<PathBuf>,
        /// After half the time, ask the host to share this monitor instead.
        #[arg(long)]
        switch_to: Option<u32>,
        /// After 1 s, move the host's pointer to the centre of its screen (no click).
        #[arg(long)]
        input_test: bool,
        /// After 1 s, send a chat message to the host.
        #[arg(long)]
        chat_test: bool,
        /// Soak test: send mouse moves at 60 Hz for the whole session (a user moving the mouse).
        #[arg(long)]
        input_flood: bool,
        /// Share the clipboard with the host (reads and writes this machine's clipboard).
        #[arg(long)]
        clipboard: bool,
    },
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum SourceArg {
    /// The main display (macOS only for now).
    Screen,
    /// Synthetic moving image (any platform).
    Pattern,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                // ICE/DTLS warn about expected trickle-ICE transients; RUST_LOG overrides.
                .unwrap_or_else(|_| "info,rtc_ice=error,rtc_dtls=error".into()),
        )
        .init();
    let args = Args::parse();
    RELAY_ONLY.store(args.relay_only, Ordering::Relaxed);

    let key = match (&args.key_file, &args.command) {
        (Some(path), _) => load_or_create_key(path)?,
        (None, Command::Host { .. }) => load_or_create_key(Path::new("peer-host.key"))?,
        (None, Command::Connect { .. }) => random_key(),
    };
    let client = SignalingClient::connect(&args.server, &key).await?;
    println!("Registered as {}", client.id);

    match args.command {
        Command::Host {
            source,
            control,
            personal_password,
            chat_reply,
        } => {
            CHAT_TEST.store(chat_reply, Ordering::Relaxed);
            // The personal secret is bound to this host's id.
            let personal = personal_password
                .map(|p| transport::derive_personal_secret(&p, client.id.as_str()));
            host(client, source, control, personal).await
        }
        Command::Connect {
            target,
            password,
            personal,
            seconds,
            save,
            switch_to,
            input_test,
            chat_test,
            input_flood,
            clipboard,
        } => {
            CLIPBOARD.store(clipboard, Ordering::Relaxed);
            INPUT_TEST.store(input_test, Ordering::Relaxed);
            CHAT_TEST.store(chat_test, Ordering::Relaxed);
            INPUT_FLOOD.store(input_flood, Ordering::Relaxed);
            let (password, personal) = if personal {
                (
                    transport::derive_personal_secret(&password, target.as_str()),
                    true,
                )
            } else {
                (password, false)
            };
            connect(
                client,
                target,
                (password, personal),
                seconds,
                save,
                switch_to,
            )
            .await
        }
    }
}

/// `connect --input-flood`.
static INPUT_FLOOD: AtomicBool = AtomicBool::new(false);
/// `connect --clipboard`.
static CLIPBOARD: AtomicBool = AtomicBool::new(false);

/// `connect --chat-test`.
static CHAT_TEST: AtomicBool = AtomicBool::new(false);
/// `connect --input-test`.
static INPUT_TEST: AtomicBool = AtomicBool::new(false);

/// `--relay-only`, read wherever a session starts.
static RELAY_ONLY: AtomicBool = AtomicBool::new(false);

/// ICE servers sent with the session (fresh TURN credentials), else those from registration.
fn session_ice<'a>(
    session: &'a [signaling_proto::IceServer],
    client: &'a SignalingClient,
) -> &'a [signaling_proto::IceServer] {
    if session.is_empty() {
        &client.ice_servers
    } else {
        session
    }
}

fn random_key() -> String {
    let bytes: [u8; 32] = rand::rng().random();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn load_or_create_key(path: &Path) -> Result<String> {
    if path.exists() {
        return Ok(std::fs::read_to_string(path)?.trim().to_owned());
    }
    let key = random_key();
    std::fs::write(path, &key).with_context(|| format!("writing {}", path.display()))?;
    Ok(key)
}

async fn host(
    mut client: SignalingClient,
    source: SourceArg,
    control: bool,
    personal: Option<String>,
) -> Result<()> {
    let config = StreamConfig {
        source: match source {
            SourceArg::Screen => Source::Screen { display_id: None },
            SourceArg::Pattern => Source::TestPattern {
                width: 1280,
                height: 720,
            },
        },
        allow_control: control,
        ..StreamConfig::default()
    };
    let password = transport::generate_password();
    println!("Password: {password}");
    println!("Waiting for connections... (Ctrl+C to stop)");
    let mut sessions: HashMap<SessionId, PeerSession> = HashMap::new();

    loop {
        let msg = tokio::select! {
            msg = client.recv() => msg,
            _ = tokio::signal::ctrl_c() => break,
        };
        match msg {
            Some(ServerMessage::IncomingSession {
                session,
                from,
                ice_servers,
            }) => {
                println!("Incoming session {session} from {from}");
                let mut peer = PeerSession::start(
                    Role::Answerer,
                    session,
                    session_ice(&ice_servers, &client),
                    RELAY_ONLY.load(Ordering::Relaxed),
                    client.sender(),
                )
                .await?;
                let channels = peer.take_data_channels().expect("fresh session");
                let (info, password, personal) = (peer.info(), password.clone(), personal.clone());
                let video = peer.video_sender().expect("answerer has a video track");
                let config = config.clone();
                tokio::spawn(async move {
                    if let Some(channels) = transport::sort_channels(channels).await {
                        serve(channels, info, password, personal, video, config).await;
                    }
                });
                sessions.insert(session, peer);
            }
            Some(ServerMessage::Signal { session, payload }) => {
                if let Some(peer) = sessions.get_mut(&session)
                    && let Err(e) = peer.handle_signal(payload).await
                {
                    tracing::warn!(session, "bad signal: {e:#}");
                }
            }
            Some(ServerMessage::SessionClosed { session, reason }) => {
                println!("Session {session} closed: {reason}");
                if let Some(peer) = sessions.remove(&session) {
                    peer.shutdown().await;
                }
            }
            Some(ServerMessage::Error { code, message }) => {
                tracing::warn!("server error {code:?}: {message}")
            }
            Some(other) => tracing::debug!("ignored {other:?}"),
            None => bail!("signaling connection lost"),
        }
    }

    for peer in sessions.values() {
        peer.close().await;
    }
    Ok(())
}

/// Host side of one session: authenticate (auto-approved in the CLI), then stream.
async fn serve(
    channels: transport::SessionChannels,
    info: SessionInfo,
    password: String,
    personal: Option<String>,
    video: Arc<VideoSender>,
    config: StreamConfig,
) {
    let transport::SessionChannels {
        control: channel,
        files,
        audio,
        ..
    } = channels;
    let result = async {
        diag::wait_open(&channel).await?;
        let transcript = info
            .transcript()
            .await
            .context("missing DTLS fingerprints")?;
        let secrets = handshake::HostSecrets {
            temporary: &password,
            personal: personal.as_deref(),
        };
        let access = handshake::authenticate_host(
            &channel,
            secrets,
            transcript,
            |_| None,
            |_, _| async { Ok(()) },
            None,
        )
        .await?;
        anyhow::Ok(access)
    }
    .await;
    match result {
        Ok((access, name)) => match name {
            Some(name) => println!("Viewer authenticated ({access:?}) as {name}"),
            None => println!("Viewer authenticated ({access:?})"),
        },
        Err(e) => {
            println!("Authentication failed: {e:#}");
            return;
        }
    }
    // Kept alive for the whole session; dropping it would stop the stream.
    let (host_tx, commands) = mpsc::unbounded_channel();
    if CHAT_TEST.load(Ordering::Relaxed) {
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(2)).await;
            let _ = host_tx.send(stream::session::HostCommand::Chat("ciao dal host".into()));
        });
    }
    session::serve_host(
        channel,
        files,
        audio,
        video,
        config,
        commands,
        |event| match event {
            HostEvent::Started { width, height } => println!("Streaming {width}x{height}"),
            HostEvent::Failed(e) => println!("Cannot stream: {e}"),
            HostEvent::ControlUnavailable(e) => println!("Remote control unavailable: {e}"),
            HostEvent::InputBlocked(blocked) => println!("Local input blocked: {blocked}"),
            HostEvent::Transfer(p) => println!("Transfer: {}/{} bytes", p.done, p.total),
            HostEvent::ControlActive => println!("Remote control active"),
            HostEvent::ControlFailing(e) => println!("Remote control failing: {e}"),
            HostEvent::Chat(text) => println!("Chat from viewer: {text}"),
            HostEvent::Clipboard(message) => println!("Clipboard: {message}"),
            HostEvent::Quality { kbps, fps, .. } => println!("Quality: {kbps} kbit/s, {fps} fps"),
            HostEvent::Ended(reason) => println!("Stream ended: {reason}"),
        },
    )
    .await;
}

async fn connect(
    mut client: SignalingClient,
    target: PeerId,
    // (password or derived personal secret, personal?)
    password: (String, bool),
    seconds: u64,
    save: Option<PathBuf>,
    switch_to: Option<u32>,
) -> Result<()> {
    let started = Instant::now();
    client.send(ClientMessage::Connect { target });

    let mut peer = loop {
        match client.recv().await {
            Some(ServerMessage::SessionCreated {
                session,
                peer,
                ice_servers,
            }) => {
                println!("Session {session} created with {peer}");
                break PeerSession::start(
                    Role::Offerer,
                    session,
                    session_ice(&ice_servers, &client),
                    RELAY_ONLY.load(Ordering::Relaxed),
                    client.sender(),
                )
                .await?;
            }
            Some(ServerMessage::Error { code, message }) => bail!("{code:?}: {message}"),
            Some(other) => tracing::debug!("ignored {other:?}"),
            None => bail!("signaling connection lost"),
        }
    };

    let info = peer.info();
    let tracks = peer.take_video_tracks().expect("fresh session");
    let channels = peer.take_data_channels().expect("fresh session");
    let channels = transport::sort_channels(channels)
        .await
        .context("offerer always has a control channel")?;
    let (done_tx, mut done_rx) = oneshot::channel();
    tokio::spawn(async move {
        let _ = done_tx.send(
            view(
                channels, tracks, info, password, seconds, save, switch_to, started,
            )
            .await,
        );
    });

    let deadline = tokio::time::sleep(Duration::from_secs(30 + seconds));
    tokio::pin!(deadline);
    let result = loop {
        tokio::select! {
            msg = client.recv() => match msg {
                Some(ServerMessage::Signal { payload, .. }) => peer.handle_signal(payload).await?,
                Some(ServerMessage::SessionClosed { reason, .. }) => break Err(anyhow::anyhow!("session closed: {reason}")),
                Some(ServerMessage::Error { code, message }) => tracing::warn!("server error {code:?}: {message}"),
                Some(_) => {}
                None => break Err(anyhow::anyhow!("signaling connection lost")),
            },
            res = &mut done_rx => break res.context("viewer task panicked")?,
            _ = &mut deadline => break Err(anyhow::anyhow!("timed out (is the host reachable? NAT may need TURN)")),
        }
    };
    peer.close().await;
    result
}

/// Viewer side: authenticate, receive the stream for `seconds`, report what arrived.
#[allow(clippy::too_many_arguments)]
async fn view(
    channels: transport::SessionChannels,
    tracks: mpsc::UnboundedReceiver<Arc<dyn TrackRemote>>,
    info: SessionInfo,
    // (password or derived personal secret, personal?)
    password: (String, bool),
    seconds: u64,
    save: Option<PathBuf>,
    switch_to: Option<u32>,
    started: Instant,
) -> Result<()> {
    let transport::SessionChannels {
        control: channel,
        files,
        audio,
        ..
    } = channels;
    diag::wait_open(&channel).await?;
    println!(
        "Data channel open after {:.0} ms",
        started.elapsed().as_secs_f64() * 1000.0
    );
    let transcript = info
        .transcript()
        .await
        .context("missing DTLS fingerprints")?;
    let (password, personal) = password;
    handshake::authenticate_viewer(&channel, &password, personal, transcript, None).await?;
    println!("Authenticated");
    match info.connection_path().await {
        Some(path) => println!("Connection path: {path:?}"),
        None => println!("Connection path: unknown"),
    }

    let mut file = match &save {
        Some(path) => Some(std::fs::File::create(path)?),
        None => None,
    };
    let (stop_tx, commands) = mpsc::unbounded_channel();
    if INPUT_FLOOD.load(Ordering::Relaxed) {
        let tx = stop_tx.clone();
        tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_millis(16));
            let mut t = 0f32;
            loop {
                tick.tick().await;
                t += 0.01;
                let event = protocol::InputEvent::MouseMove {
                    x: 0.5 + 0.3 * t.cos(),
                    y: 0.5 + 0.3 * t.sin(),
                };
                if tx.send(ViewerCommand::Input(event)).is_err() {
                    break;
                }
            }
        });
    }
    if CHAT_TEST.load(Ordering::Relaxed) {
        let tx = stop_tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            println!("Chat test: sending a message to the host");
            let _ = tx.send(ViewerCommand::Chat("ciao dal viewer".into()));
        });
    }
    if INPUT_TEST.load(Ordering::Relaxed) {
        let tx = stop_tx.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(1)).await;
            println!("Input test: moving the host pointer to the centre");
            let _ = tx.send(ViewerCommand::Input(protocol::InputEvent::MouseMove {
                x: 0.5,
                y: 0.5,
            }));
        });
    }
    tokio::spawn(async move {
        if let Some(id) = switch_to {
            tokio::time::sleep(Duration::from_secs(seconds) / 2).await;
            println!("Switching to display {id}");
            let _ = stop_tx.send(ViewerCommand::SelectDisplay(id));
        }
        tokio::time::sleep(Duration::from_secs(seconds) / if switch_to.is_some() { 2 } else { 1 })
            .await;
        let _ = stop_tx.send(ViewerCommand::Stop);
    });

    let (mut frames, mut keyframes, mut bytes) = (0u64, 0u64, 0u64);
    let mut first_frame: Option<Instant> = None;
    let mut rtts = Vec::new();
    let mut resyncs = 0u32;
    let mut partner_usage = None;
    let mut failure = None;
    let share_clipboard = CLIPBOARD.load(Ordering::Relaxed);
    session::run_viewer(
        channel,
        files,
        audio,
        tracks,
        commands,
        share_clipboard,
        |event| match event {
            ViewerEvent::Started { width, height, .. } => {
                println!("Host streaming {width}x{height}")
            }
            ViewerEvent::Frame(frame) => {
                if first_frame.is_none() {
                    first_frame = Some(Instant::now());
                    println!(
                        "First frame after {:.0} ms",
                        started.elapsed().as_secs_f64() * 1000.0
                    );
                }
                frames += 1;
                keyframes += u64::from(frame.keyframe);
                bytes += frame.data.len() as u64;
                if let Some(file) = &mut file {
                    let _ = file.write_all(&frame.data);
                }
            }
            ViewerEvent::Rtt(rtt) => rtts.push(rtt),
            ViewerEvent::Resync => resyncs += 1,
            ViewerEvent::Clipboard(message) => println!("Clipboard: {message}"),
            ViewerEvent::ClipboardEnabled(enabled) => println!("Clipboard sharing: {enabled}"),
            ViewerEvent::Chat(text) => println!("Chat from host: {text}"),
            ViewerEvent::PartnerUsage { cpu, ram_mb } => partner_usage = Some((cpu, ram_mb)),
            ViewerEvent::Quality { kbps, fps, .. } => {
                println!("Host quality: {kbps} kbit/s, {fps} fps")
            }
            ViewerEvent::Error(e) => {
                println!("Host error: {e}");
                failure = Some(e);
            }
            ViewerEvent::ControlUnavailable(e) => println!("Host cannot be controlled: {e}"),
            ViewerEvent::ScreenUnavailable(e) => println!("Host has no screen yet: {e}"),
            ViewerEvent::AudioUnavailable(e) => println!("Sound unavailable: {e}"),
            ViewerEvent::ResolutionAdjustable => println!("Host can match its resolution"),
            ViewerEvent::PartnerUpdating => println!("Host is quitting to update"),
            ViewerEvent::PartnerReconnect => println!("Host asks to connect again"),
            ViewerEvent::ConnectionLost => println!("Connection lost"),
            ViewerEvent::Silent(true) => println!("Host silent, waiting"),
            ViewerEvent::Silent(false) => println!("Host answers again"),
            ViewerEvent::Transfer(p) => println!("Transfer: {}/{} bytes", p.done, p.total),
            ViewerEvent::InputBlocked { blocked, message } => {
                println!(
                    "Host input blocked: {blocked} {}",
                    message.unwrap_or_default()
                )
            }
            ViewerEvent::Displays { displays, current } => {
                for d in displays {
                    let mark = if d.id == current { "*" } else { " " };
                    println!(
                        "{mark} display {} {} {}x{}",
                        d.id, d.name, d.width, d.height
                    );
                }
            }
            ViewerEvent::Ended(reason) => println!("Session ended: {reason}"),
        },
    )
    .await;

    let secs = first_frame
        .map_or(0.0, |t| t.elapsed().as_secs_f64())
        .max(0.001);
    println!(
        "\nVideo: {frames} frames ({:.1} fps), {keyframes} keyframes, {:.0} KiB ({:.0} kbit/s)",
        frames as f64 / secs,
        bytes as f64 / 1024.0,
        bytes as f64 * 8.0 / 1000.0 / secs
    );
    println!("Keyframe requests (resync): {resyncs}");
    if let Some((cpu, ram_mb)) = partner_usage {
        println!("Host app usage: CPU {cpu:.1}%, RAM {ram_mb} MB");
    }
    if let Some((sent, received)) = info.traffic().await {
        println!(
            "Traffic: sent {} KiB, received {} KiB",
            sent / 1024,
            received / 1024
        );
    }
    let samples: Vec<String> = rtts
        .iter()
        .map(|d| format!("{:.1}", d.as_secs_f64() * 1000.0))
        .collect();
    println!("RTT samples (ms): {}", samples.join(" "));
    let stats = diag::RttStats::from_samples(&rtts);
    println!(
        "RTT over {} pings: min {:.2} / avg {:.2} / max {:.2} ms",
        stats.count, stats.min_ms, stats.avg_ms, stats.max_ms
    );
    if let Some(path) = save {
        println!(
            "Saved to {} (ffplay -f h264 {})",
            path.display(),
            path.display()
        );
    }
    if let Some(e) = failure {
        bail!("host could not stream: {e}");
    }
    if frames == 0 {
        bail!("no video received");
    }
    Ok(())
}
