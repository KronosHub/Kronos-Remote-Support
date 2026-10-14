//! Background service: keeps the device registered on the signaling server, answers
//! incoming sessions (password + local user approval, then screen sharing) and opens
//! sessions towards partners (receiving their screen). Reports to the UI through the
//! `service` event; video frames go through a per-session binary channel.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use signaling_proto::{ClientMessage, ErrorCode, PeerId, ServerMessage, SessionId};
use stream::session::{self, HostCommand, HostEvent, ViewerCommand, ViewerEvent};
use stream::{Source, StreamConfig};
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::{AppHandle, Emitter, Manager};
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot;

use crate::store::{SavedDevice, Store};
use transport::handshake::{self, Access, HandshakeError, Refusal};
use transport::video::VideoSender;
use transport::{
    ConnectionState, DataChannel, PeerSession, Role, SessionChannels, SessionInfo, SignalingClient,
    TrackRemote, diag,
};

/// The only signaling server for now (see deploy/README.md).
/// Signaling on Cloudflare Workers (ADR 0035); the server address is never shown in the UI.
pub const SERVER: &str = "wss://kronos-signaling.kronos-signaling.workers.dev/ws";
const RETRY_DELAY: Duration = Duration::from_secs(3);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
/// Time allowed for ICE + DTLS before declaring the peer unreachable.
const OPEN_TIMEOUT: Duration = Duration::from_secs(20);
/// How long the host user has to accept an incoming connection.
pub const APPROVAL_TIMEOUT: Duration = Duration::from_secs(30);
/// Viewer waits for password check + host approval (temporary password).
const AUTH_TIMEOUT: Duration = Duration::from_secs(45);
/// With the personal password nobody has to click: the answer comes at once.
const PERSONAL_AUTH_TIMEOUT: Duration = Duration::from_secs(15);
/// After refusing, the host keeps the channel open briefly so the reason reaches the viewer.
const REFUSAL_LINGER: Duration = Duration::from_secs(2);
/// Wrong guesses of the temporary password before it is replaced (ADR 0039).
const MAX_TEMPORARY_FAILURES: u32 = 3;
/// Wrong guesses of the personal password that block unattended access for a while.
const MAX_PERSONAL_FAILURES: usize = 5;
const FAILED_ATTEMPTS_WINDOW: Duration = Duration::from_secs(10 * 60);

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Link {
    Connecting,
    Online,
    Offline,
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub link: Link,
    /// Formatted id ("123 456 789") once registered.
    pub id: Option<String>,
    /// Password partners must enter to connect to this device.
    pub password: String,
    /// A personal password is set: owners can connect without approval.
    pub unattended: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum UiEvent {
    Status(Status),
    Log {
        level: &'static str,
        message: String,
    },
    /// A partner with the right password is waiting for the local user's decision.
    ApprovalRequest {
        session: SessionId,
        peer: String,
        timeout_secs: u64,
    },
    ApprovalClosed {
        session: SessionId,
    },
    /// We are watching a partner's screen: the UI should open the viewer and attach a channel.
    ViewOpened {
        session: SessionId,
        peer: String,
    },
    ViewSize {
        session: SessionId,
        width: u32,
        height: u32,
    },
    ViewRtt {
        session: SessionId,
        ms: f64,
    },
    /// The picture had to be resynced (keyframe requested).
    ViewResync {
        session: SessionId,
    },
    /// Traffic of the session and CPU/memory of this app, every second while viewing.
    ViewStats {
        session: SessionId,
        rx_kbps: u32,
        tx_kbps: u32,
        cpu: Option<f32>,
        ram_mb: Option<u32>,
    },
    /// CPU/memory the partner's app uses.
    ViewPartnerUsage {
        session: SessionId,
        cpu: f32,
        ram_mb: u32,
    },
    /// Clipboard sharing turned on/off for a viewer session.
    ViewClipboard {
        session: SessionId,
        enabled: bool,
    },
    /// The partner's sound is (no longer) playing.
    ViewAudio {
        session: SessionId,
        enabled: bool,
    },
    /// Short message for the viewer window (e.g. files received on the clipboard).
    ViewNotice {
        session: SessionId,
        message: String,
    },
    /// A chat message from the partner, to show in the viewer window.
    ViewChat {
        session: SessionId,
        text: String,
    },
    /// A chat message from a partner we are sharing with, for the main window.
    HostChat {
        session: SessionId,
        peer: String,
        text: String,
    },
    /// Devices of this app found on the local network (connectable without the server).
    LanPeers {
        peers: Vec<LanPeerView>,
    },
    /// The partner adapted the stream to the available bandwidth.
    ViewQuality {
        session: SessionId,
        kbps: u32,
        fps: u32,
        limited: bool,
    },
    /// The partner's monitors and the one being shown.
    ViewDisplays {
        session: SessionId,
        displays: Vec<protocol::DisplayInfo>,
        current: u32,
    },
    /// The partner's screen can be watched but not controlled.
    ViewControlUnavailable {
        session: SessionId,
        message: String,
    },
    ViewClosed {
        session: SessionId,
        reason: String,
        /// How long the session lasted, for the end-of-session popup.
        duration_secs: u64,
    },
    /// Our screen is being shared with a partner.
    HostingStarted {
        session: SessionId,
        peer: String,
    },
    HostingEnded {
        session: SessionId,
        peer: String,
        duration_secs: u64,
    },
    /// Which network path a session uses (shown in the viewer toolbar and host banner).
    ConnectionPath {
        session: SessionId,
        path: Option<transport::ConnectionPath>,
    },
}

/// How we authenticate to a partner.
#[derive(Clone)]
pub struct Credential {
    /// Temporary password as typed, or the derived personal secret.
    secret: String,
    personal: bool,
}

enum Command {
    Connect {
        target: PeerId,
        credential: Credential,
    },
    /// Like `Connect`, but through the server only: the direct LAN attempt already failed.
    ConnectViaServer {
        target: PeerId,
        credential: Credential,
    },
    /// Abort connections still being set up (the "Annulla" button).
    CancelConnect,
}

/// A device of this app seen on the local network.
#[derive(Debug, Clone, Serialize)]
pub struct LanPeerView {
    pub id: String,
    /// Computer name the device announces.
    pub host: String,
    /// Name given by the user on this computer, if any.
    pub alias: Option<String>,
}

/// A saved device as shown in the UI (no secret).
#[derive(Debug, Clone, Serialize)]
pub struct DeviceView {
    pub id: String,
    pub name: String,
}

enum Internal {
    SessionDone(SessionId),
    /// A peer connected to us directly over the LAN (we are the host).
    LanIncoming(transport::lan::LanSession),
}

/// Control handle of a session we are viewing.
struct ViewerCtl {
    commands: UnboundedSender<ViewerCommand>,
    /// Binary channel to the UI, attached once the viewer screen is ready.
    frames: Option<Channel<InvokeResponseBody>>,
    /// Frames sent to the UI and not yet acknowledged as processed.
    in_flight: u32,
    /// The UI fell behind: drop frames until it catches up and a keyframe arrives.
    resyncing: bool,
    /// Events for the viewer window produced before it attached (it opens asynchronously).
    backlog: Vec<UiEvent>,
}

/// Viewer events kept for a window that has not attached yet; older ones are dropped.
const MAX_BACKLOG: usize = 256;

/// Frames the UI may lag behind (~0.5 s at 30 fps) before we stop feeding it. A window
/// in the background (WebView2 throttles it) would otherwise queue minutes of video.
const MAX_FRAMES_IN_FLIGHT: u32 = 15;
/// Lag at which a resyncing UI is considered caught up again.
const RESUME_FRAMES_IN_FLIGHT: u32 = 3;

/// Longest chat message shown; the rest is cut.
const MAX_CHAT_CHARS: usize = 4000;
/// Chat messages accepted from a partner per [`CHAT_WINDOW`]; more are dropped.
const MAX_CHAT_MESSAGES: u32 = 20;
const CHAT_WINDOW: Duration = Duration::from_secs(10);
/// A chat message brings the main window forward at most this often.
const CHAT_RAISE_EVERY: Duration = Duration::from_secs(30);

/// Keeps a partner from flooding the chat or stealing the focus with it.
#[derive(Default)]
struct ChatLimiter {
    window: Option<Instant>,
    count: u32,
    raised: Option<Instant>,
}

impl ChatLimiter {
    /// The message to show (cut to [`MAX_CHAT_CHARS`]), or `None` to drop it.
    fn accept(&mut self, mut text: String) -> Option<String> {
        if self.window.is_none_or(|t| t.elapsed() > CHAT_WINDOW) {
            self.window = Some(Instant::now());
            self.count = 0;
        }
        self.count += 1;
        if self.count > MAX_CHAT_MESSAGES {
            return None;
        }
        if let Some((end, _)) = text.char_indices().nth(MAX_CHAT_CHARS) {
            text.truncate(end);
        }
        Some(text)
    }

    /// True if the main window may come forward for a new message.
    fn may_raise(&mut self) -> bool {
        if self.raised.is_some_and(|t| t.elapsed() < CHAT_RAISE_EVERY) {
            return false;
        }
        self.raised = Some(Instant::now());
        true
    }
}

/// Wrong password guesses, per password (ADR 0039).
#[derive(Default)]
struct Attempts {
    /// Wrong guesses of the current temporary password.
    temporary: u32,
    /// Wrong guesses of the personal password within [`FAILED_ATTEMPTS_WINDOW`].
    personal: VecDeque<Instant>,
}

/// State shared between the service task, its spawned tasks and Tauri commands.
struct Shared {
    status: Mutex<Status>,
    approvals: Mutex<HashMap<SessionId, oneshot::Sender<bool>>>,
    attempts: Mutex<Attempts>,
    viewers: Mutex<HashMap<SessionId, ViewerCtl>>,
    hosting: Mutex<HashMap<SessionId, UnboundedSender<HostCommand>>>,
    /// Hosting sessions opened with the personal password: ended when it is removed or
    /// changed.
    unattended: Mutex<HashSet<SessionId>>,
    /// Derived personal secret of this device (unattended access), if enabled.
    personal: Mutex<Option<String>>,
    /// Outgoing sessions still connecting/authenticating: cancel them with a reason.
    connecting: Mutex<HashMap<SessionId, oneshot::Sender<String>>>,
    /// LAN discovery (peers on the local network), when this device's id is known.
    lan: Mutex<Option<Arc<transport::lan::LanDiscovery>>>,
    store: Store,
}

impl Shared {
    /// Replaces the temporary password and tells the UI.
    fn rotate_password(&self, app: &AppHandle) -> Status {
        let status = {
            let mut status = self.status.lock().unwrap();
            status.password = transport::generate_password();
            status.clone()
        };
        self.attempts.lock().unwrap().temporary = 0;
        emit(app, UiEvent::Status(status.clone()));
        status
    }

    fn password(&self) -> String {
        self.status.lock().unwrap().password.clone()
    }

    /// Records a wrong guess. A guessed-at temporary password is replaced after a few
    /// failures, so trying passwords is pointless and nobody else gets locked out.
    fn record_failure(&self, app: &AppHandle, personal: bool) {
        if personal {
            self.attempts
                .lock()
                .unwrap()
                .personal
                .push_back(Instant::now());
            return;
        }
        let replace = {
            let mut attempts = self.attempts.lock().unwrap();
            attempts.temporary += 1;
            attempts.temporary >= MAX_TEMPORARY_FAILURES
        };
        if replace {
            self.rotate_password(app);
            log(
                app,
                "warn",
                "troppe password errate: la password temporanea è stata cambiata",
            );
        }
    }

    /// Too many wrong personal passwords lately: unattended access is paused.
    fn personal_locked_out(&self) -> bool {
        let mut attempts = self.attempts.lock().unwrap();
        while attempts
            .personal
            .front()
            .is_some_and(|t| t.elapsed() > FAILED_ATTEMPTS_WINDOW)
        {
            attempts.personal.pop_front();
        }
        attempts.personal.len() >= MAX_PERSONAL_FAILURES
    }

    /// Emits an event for a viewer window, or keeps it until the window attaches.
    fn view_event(&self, app: &AppHandle, session: SessionId, event: UiEvent) {
        if let Some(viewer) = self.viewers.lock().unwrap().get_mut(&session)
            && viewer.frames.is_none()
        {
            if viewer.backlog.len() >= MAX_BACKLOG {
                viewer.backlog.remove(0);
            }
            viewer.backlog.push(event);
            return;
        }
        emit(app, event);
    }

    fn send_frame(&self, session: SessionId, frame: &transport::video::VideoFrame) {
        let mut viewers = self.viewers.lock().unwrap();
        let Some(viewer) = viewers.get_mut(&session) else {
            return;
        };
        let Some(channel) = viewer.frames.as_ref() else {
            return;
        };
        if viewer.in_flight >= MAX_FRAMES_IN_FLIGHT && !viewer.resyncing {
            tracing::info!(
                session,
                "UI behind on video, dropping frames until it catches up"
            );
            viewer.resyncing = true;
        }
        if viewer.resyncing {
            // Deltas would reference dropped frames: wait for a keyframe, and only send it
            // once the UI has drained what it already has (see `frames_consumed`).
            if !frame.keyframe || viewer.in_flight > RESUME_FRAMES_IN_FLIGHT {
                return;
            }
            viewer.resyncing = false;
        }
        viewer.in_flight += 1;
        // [keyframe: u8][timestamp µs: u64 LE][Annex B access unit]
        let timestamp_us = u64::from(frame.rtp_timestamp) * 1000 / 90;
        let mut buf = Vec::with_capacity(9 + frame.data.len());
        buf.push(u8::from(frame.keyframe));
        buf.extend_from_slice(&timestamp_us.to_le_bytes());
        buf.extend_from_slice(&frame.data);
        let _ = channel.send(InvokeResponseBody::Raw(buf));
    }

    /// The UI processed `count` frames.
    fn frames_consumed(&self, session: SessionId, count: u32) {
        let mut viewers = self.viewers.lock().unwrap();
        let Some(viewer) = viewers.get_mut(&session) else {
            return;
        };
        viewer.in_flight = viewer.in_flight.saturating_sub(count);
        // Caught up while resyncing: ask for a fresh picture now.
        if viewer.resyncing && viewer.in_flight <= RESUME_FRAMES_IN_FLIGHT {
            let _ = viewer.commands.send(ViewerCommand::RequestKeyframe);
        }
    }
}

/// Handle stored in Tauri state; commands go to the service task.
pub struct Service {
    commands: UnboundedSender<Command>,
    shared: Arc<Shared>,
    app: AppHandle,
}

impl Service {
    /// `data_dir: None` (Quick access): throwaway identity, nothing stored on disk.
    pub fn start(app: AppHandle, data_dir: Option<PathBuf>) -> anyhow::Result<Self> {
        let device_key = match &data_dir {
            Some(dir) => {
                std::fs::create_dir_all(dir)?;
                load_or_create_key(&dir.join("device.key"))?
            }
            None => new_device_key(),
        };
        let store = Store::new(data_dir);
        let personal = store.personal_secret();
        let shared = Arc::new(Shared {
            status: Mutex::new(Status {
                link: Link::Connecting,
                id: None,
                password: transport::generate_password(),
                unattended: personal.is_some(),
                error: None,
            }),
            approvals: Mutex::new(HashMap::new()),
            attempts: Mutex::new(Attempts::default()),
            viewers: Mutex::new(HashMap::new()),
            hosting: Mutex::new(HashMap::new()),
            unattended: Mutex::new(HashSet::new()),
            personal: Mutex::new(personal),
            connecting: Mutex::new(HashMap::new()),
            lan: Mutex::new(None),
            store,
        });
        let (commands, rx) = mpsc::unbounded_channel();
        let ctx = Ctx {
            app: app.clone(),
            device_key,
            shared: shared.clone(),
            commands: commands.clone(),
        };
        tauri::async_runtime::spawn(run(ctx, rx));
        Ok(Self {
            commands,
            shared,
            app,
        })
    }

    pub fn status(&self) -> Status {
        self.shared.status.lock().unwrap().clone()
    }

    /// Connects with the partner's temporary password (the partner must approve).
    pub fn connect(&self, target: PeerId, password: String) {
        let credential = Credential {
            secret: password,
            personal: false,
        };
        let _ = self.commands.send(Command::Connect { target, credential });
    }

    /// Connects with the partner's personal password (no approval); optionally remembers
    /// the device under `save_as`.
    pub fn connect_personal(
        &self,
        target: PeerId,
        password: &str,
        save_as: Option<String>,
    ) -> Result<(), String> {
        let secret = transport::derive_personal_secret(password, target.as_str());
        if let Some(name) = save_as {
            self.shared
                .store
                .save_device(SavedDevice {
                    id: target.as_str().to_owned(),
                    name,
                    secret: secret.clone(),
                })
                .map_err(|e| format!("impossibile salvare il dispositivo: {e}"))?;
        }
        let credential = Credential {
            secret,
            personal: true,
        };
        let _ = self.commands.send(Command::Connect { target, credential });
        Ok(())
    }

    pub fn connect_device(&self, id: &str) -> Result<(), String> {
        let id: String = id.chars().filter(|c| !c.is_whitespace()).collect();
        let device = self
            .shared
            .store
            .devices()
            .into_iter()
            .find(|d| d.id == id)
            .ok_or("dispositivo non trovato")?;
        let target: PeerId = device.id.parse()?;
        let credential = Credential {
            secret: device.secret,
            personal: true,
        };
        let _ = self.commands.send(Command::Connect { target, credential });
        Ok(())
    }

    pub fn cancel_connect(&self) {
        let _ = self.commands.send(Command::CancelConnect);
    }

    /// Devices of this app currently seen on the local network.
    pub fn lan_peers(&self) -> Vec<LanPeerView> {
        let names = self.shared.store.lan_names();
        let mut peers: Vec<LanPeerView> = self
            .shared
            .lan
            .lock()
            .unwrap()
            .as_ref()
            .map(|d| {
                d.peers()
                    .into_iter()
                    .map(|p| LanPeerView {
                        alias: names.get(p.id.as_str()).cloned(),
                        id: p.id.to_string(),
                        host: p.name,
                    })
                    .collect()
            })
            .unwrap_or_default();
        // Named devices first, then by name.
        peers.sort_by_key(|p| {
            (
                p.alias.is_none(),
                p.alias
                    .clone()
                    .unwrap_or_else(|| p.host.clone())
                    .to_lowercase(),
            )
        });
        peers
    }

    /// Names a LAN device on this computer (empty name: back to its computer name).
    pub fn set_lan_name(&self, id: &str, name: &str) -> Result<Vec<LanPeerView>, String> {
        let id: String = id.chars().filter(|c| !c.is_whitespace()).collect();
        let name = name.trim();
        if name.chars().count() > 40 {
            return Err("nome troppo lungo (massimo 40 caratteri)".into());
        }
        self.shared
            .store
            .set_lan_name(&id, name)
            .map_err(|e| format!("impossibile salvare: {e}"))?;
        Ok(self.lan_peers())
    }

    /// Windows: the user declined the admin-windows agent, so startup must not re-ask.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn uac_agent_declined(&self) -> bool {
        self.shared.store.uac_agent_declined()
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn set_uac_agent_declined(&self, declined: bool) {
        let _ = self.shared.store.set_uac_agent_declined(declined);
    }

    pub fn devices(&self) -> Vec<DeviceView> {
        self.shared
            .store
            .devices()
            .into_iter()
            .map(|d| DeviceView {
                id: d.id.parse::<PeerId>().map_or(d.id, |id| id.to_string()),
                name: d.name,
            })
            .collect()
    }

    pub fn remove_device(&self, id: &str) -> Result<(), String> {
        let id: String = id.chars().filter(|c| !c.is_whitespace()).collect();
        self.shared
            .store
            .remove_device(&id)
            .map_err(|e| format!("impossibile rimuovere il dispositivo: {e}"))
    }

    /// Enables unattended access with `password`, or disables it with `None`.
    pub fn set_personal_password(&self, password: Option<&str>) -> Result<Status, String> {
        if crate::QUICK {
            return Err("l'accesso non presidiato non è disponibile in Quick access".into());
        }
        let secret = match password {
            Some(password) => {
                transport::validate_personal_password(password)?;
                // The secret is bound to this device's id, known once registered.
                let id = self
                    .status()
                    .id
                    .ok_or("attendi la connessione al server, poi riprova")?;
                let id: PeerId = id.parse()?;
                Some(transport::derive_personal_secret(password, id.as_str()))
            }
            None => None,
        };
        self.shared
            .store
            .set_personal_secret(secret.as_deref())
            .map_err(|e| format!("impossibile salvare: {e}"))?;
        let enabled = secret.is_some();
        *self.shared.personal.lock().unwrap() = secret;
        // Whoever got in with the previous personal password loses access now.
        let ended: Vec<SessionId> = self.shared.unattended.lock().unwrap().drain().collect();
        let hosting = self.shared.hosting.lock().unwrap();
        for session in &ended {
            if let Some(tx) = hosting.get(session) {
                let _ = tx.send(HostCommand::Stop);
            }
        }
        drop(hosting);
        if !ended.is_empty() {
            log(
                &self.app,
                "info",
                "password personale cambiata: sessioni senza conferma terminate",
            );
        }
        let status = {
            let mut status = self.shared.status.lock().unwrap();
            status.unattended = enabled;
            status.clone()
        };
        emit(&self.app, UiEvent::Status(status.clone()));
        Ok(status)
    }

    pub fn new_password(&self) -> Status {
        self.shared.rotate_password(&self.app)
    }

    /// Delivers the local user's decision for a pending incoming connection.
    pub fn respond(&self, session: SessionId, accept: bool) {
        if let Some(tx) = self.shared.approvals.lock().unwrap().remove(&session) {
            let _ = tx.send(accept);
        }
    }

    /// The viewer screen is ready: start delivering frames (beginning with a keyframe).
    /// The viewer window is ready: frames start flowing and the events it missed are
    /// returned. `None` if the session is already over.
    pub fn attach_view(
        &self,
        session: SessionId,
        frames: Channel<InvokeResponseBody>,
    ) -> Option<Vec<UiEvent>> {
        let mut viewers = self.shared.viewers.lock().unwrap();
        let viewer = viewers.get_mut(&session)?;
        viewer.frames = Some(frames);
        viewer.in_flight = 0;
        viewer.resyncing = false;
        let _ = viewer.commands.send(ViewerCommand::RequestKeyframe);
        Some(std::mem::take(&mut viewer.backlog))
    }

    /// Turns clipboard sharing on or off for a viewer session.
    pub fn set_clipboard(&self, session: SessionId, enabled: bool) {
        if let Some(viewer) = self.shared.viewers.lock().unwrap().get(&session) {
            let _ = viewer.commands.send(ViewerCommand::Clipboard(enabled));
        }
    }

    /// Sends a chat message on a session we view (viewer window) or share (main window).
    pub fn send_chat(&self, session: SessionId, text: String) -> bool {
        if let Some(viewer) = self.shared.viewers.lock().unwrap().get(&session) {
            return viewer.commands.send(ViewerCommand::Chat(text)).is_ok();
        }
        if let Some(host) = self.shared.hosting.lock().unwrap().get(&session) {
            return host.send(HostCommand::Chat(text)).is_ok();
        }
        false
    }

    pub fn frames_consumed(&self, session: SessionId, count: u32) {
        self.shared.frames_consumed(session, count);
    }

    pub fn request_keyframe(&self, session: SessionId) {
        if let Some(viewer) = self.shared.viewers.lock().unwrap().get(&session) {
            let _ = viewer.commands.send(ViewerCommand::RequestKeyframe);
        }
    }

    pub fn send_input(&self, session: SessionId, event: protocol::InputEvent) {
        if let Some(viewer) = self.shared.viewers.lock().unwrap().get(&session) {
            let _ = viewer.commands.send(ViewerCommand::Input(event));
        }
    }

    pub fn set_audio(&self, session: SessionId, enabled: bool) {
        if let Some(viewer) = self.shared.viewers.lock().unwrap().get(&session) {
            let _ = viewer.commands.send(ViewerCommand::Audio(enabled));
        }
    }

    pub fn set_high_resolution(&self, session: SessionId, enabled: bool) {
        if let Some(viewer) = self.shared.viewers.lock().unwrap().get(&session) {
            let _ = viewer.commands.send(ViewerCommand::HighResolution(enabled));
        }
    }

    pub fn select_display(&self, session: SessionId, id: u32) {
        if let Some(viewer) = self.shared.viewers.lock().unwrap().get(&session) {
            let _ = viewer.commands.send(ViewerCommand::SelectDisplay(id));
        }
    }

    /// True while we view or share this session.
    pub fn is_session_active(&self, session: SessionId) -> bool {
        self.shared.viewers.lock().unwrap().contains_key(&session)
            || self.shared.hosting.lock().unwrap().contains_key(&session)
    }

    pub fn close_view(&self, session: SessionId) {
        if let Some(viewer) = self.shared.viewers.lock().unwrap().get(&session) {
            let _ = viewer.commands.send(ViewerCommand::Stop);
        }
    }

    pub fn stop_hosting(&self, session: SessionId) {
        if let Some(tx) = self.shared.hosting.lock().unwrap().get(&session) {
            let _ = tx.send(HostCommand::Stop);
        }
    }
}

struct Ctx {
    app: AppHandle,
    device_key: String,
    shared: Arc<Shared>,
    /// Back into the service loop (a failed LAN attempt retries through the server).
    commands: UnboundedSender<Command>,
}

impl Ctx {
    fn log(&self, level: &'static str, message: impl Into<String>) {
        log(&self.app, level, message);
    }

    fn set_status(&self, link: Link, id: Option<String>, error: Option<String>) {
        let status = {
            let mut status = self.shared.status.lock().unwrap();
            status.link = link;
            status.id = id;
            status.error = error.map(|e| crate::privacy::redact(&e));
            status.clone()
        };
        emit(&self.app, UiEvent::Status(status));
    }
}

fn emit(app: &AppHandle, event: UiEvent) {
    let _ = app.emit("service", event);
}

fn log(app: &AppHandle, level: &'static str, message: impl Into<String>) {
    let message = message.into();
    tracing::info!("[{level}] {message}");
    // Never show server addresses in the UI (the app may be distributed).
    let message = crate::privacy::redact(&message);
    emit(app, UiEvent::Log { level, message });
}

fn load_or_create_key(path: &std::path::Path) -> anyhow::Result<String> {
    if let Ok(key) = std::fs::read_to_string(path) {
        // Keys written before 1.2.0 were readable by the other users of the computer.
        crate::store::restrict_permissions(path);
        return Ok(key.trim().to_owned());
    }
    let key = new_device_key();
    std::io::Write::write_all(&mut crate::store::private_file(path)?, key.as_bytes())?;
    Ok(key)
}

fn new_device_key() -> String {
    use rand::RngExt;
    let bytes: [u8; 32] = rand::rng().random();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

async fn run(ctx: Ctx, mut commands: UnboundedReceiver<Command>) {
    // Sessions outlive signaling connections: once WebRTC is up, the server is no longer
    // needed, so a dropped/reconnected signaling socket must not end a remote session.
    let (internal_tx, mut internal) = mpsc::unbounded_channel::<Internal>();
    let mut sessions = Sessions {
        active: HashMap::new(),
        pending_connects: HashMap::new(),
        internal: internal_tx.clone(),
    };

    // LAN: listen for direct connections and, once our id is known, advertise + browse.
    // Works with or without the server, so it is set up before the server loop.
    let lan_port = match transport::lan::listen().await {
        Ok((port, mut incoming)) => {
            let internal = internal_tx.clone();
            tauri::async_runtime::spawn(async move {
                while let Some(session) = incoming.recv().await {
                    let _ = internal.send(Internal::LanIncoming(session));
                }
            });
            Some(port)
        }
        Err(e) => {
            tracing::warn!("LAN listener unavailable: {e:#}");
            None
        }
    };
    if let (Some(port), Some(id)) = (lan_port, ctx.shared.store.cached_id()) {
        start_lan_discovery(&ctx, &id, port);
    }

    loop {
        ctx.set_status(Link::Connecting, None, None);
        let attempt = tokio::time::timeout(
            CONNECT_TIMEOUT,
            SignalingClient::connect(SERVER, &ctx.device_key),
        )
        .await;
        let error = match attempt {
            Ok(Ok(client)) => {
                let id = client.id.to_string();
                ctx.set_status(Link::Online, Some(id.clone()), None);
                ctx.log("info", format!("registrato come {}", client.id));
                // Remember the id and (re)start LAN discovery with it.
                // A different id than the one advertised (e.g. new server): re-advertise.
                let previous = ctx.shared.store.cached_id();
                let changed = previous.as_deref() != Some(id.as_str());
                if changed {
                    let _ = ctx.shared.store.set_cached_id(&id);
                }
                // The personal password is bound to the device id: after an id change (e.g.
                // a new signaling server) the stored secret can never match again.
                // No previous id at all means the secret predates this server too (ids were
                // only cached from 0.11): it can never match either.
                if changed && ctx.shared.personal.lock().unwrap().is_some() {
                    let _ = ctx.shared.store.set_personal_secret(None);
                    *ctx.shared.personal.lock().unwrap() = None;
                    let status = {
                        let mut status = ctx.shared.status.lock().unwrap();
                        status.unattended = false;
                        status.clone()
                    };
                    emit(&ctx.app, UiEvent::Status(status));
                    ctx.log(
                        "warn",
                        "il codice di questo PC è cambiato: imposta di nuovo la password personale",
                    );
                }
                if let Some(port) = lan_port
                    && (changed || ctx.shared.lan.lock().unwrap().is_none())
                {
                    // Drop the old advertisement first (its Drop unregisters it).
                    ctx.shared.lan.lock().unwrap().take();
                    start_lan_discovery(&ctx, &id, port);
                }
                online(&ctx, client, &mut commands, &mut sessions, &mut internal).await
            }
            Ok(Err(e)) => {
                tracing::warn!("signaling connection failed: {e:#}");
                crate::privacy::connect_error(&e)
            }
            Err(_) => "timeout di connessione al server".into(),
        };
        ctx.set_status(Link::Offline, None, Some(error.clone()));
        ctx.log("warn", format!("offline: {error}"));

        let retry = tokio::time::sleep(RETRY_DELAY);
        tokio::pin!(retry);
        loop {
            tokio::select! {
                _ = &mut retry => break,
                cmd = commands.recv() => match cmd {
                    Some(Command::Connect { .. }) if crate::QUICK => {
                        ctx.log("error", "Quick access non permette di controllare altri dispositivi");
                    }
                    // The server is down, but a LAN peer may still be reachable directly.
                    Some(Command::Connect { target, credential }) => {
                        if !try_lan_connect(&ctx, &target, &credential) {
                            ctx.log("error", "non raggiungibile in rete locale e server non connesso");
                            cancel_connecting(&ctx);
                        }
                    }
                    Some(Command::ConnectViaServer { .. }) => {
                        ctx.log("error", "non raggiungibile in rete locale e server non connesso");
                        cancel_connecting(&ctx);
                    }
                    Some(Command::CancelConnect) => cancel_connecting(&ctx),
                    None => return,
                },
                Some(msg) = internal.recv() => handle_internal(&ctx, &mut sessions, msg).await,
            }
        }
    }
}

/// Cancels all connections still being set up (the "Annulla" button / failure cleanup).
fn cancel_connecting(ctx: &Ctx) {
    let cancelled: Vec<_> = ctx.shared.connecting.lock().unwrap().drain().collect();
    for (_, cancel) in cancelled {
        let _ = cancel.send("connessione annullata".into());
    }
}

/// Handles an internal event (a finished session, or an incoming LAN connection).
async fn handle_internal(ctx: &Ctx, sessions: &mut Sessions, msg: Internal) {
    match msg {
        Internal::SessionDone(session) => {
            if let Some(peer) = sessions.active.remove(&session) {
                peer.close().await;
            }
        }
        Internal::LanIncoming(lan) => lan_serve(ctx, lan),
    }
}

struct Sessions {
    active: HashMap<SessionId, PeerSession>,
    /// Credentials of connections waiting for the server to create the session.
    pending_connects: HashMap<PeerId, Credential>,
    internal: UnboundedSender<Internal>,
}

/// Serves one signaling connection. Returns the reason it ended.
async fn online(
    ctx: &Ctx,
    mut client: SignalingClient,
    commands: &mut UnboundedReceiver<Command>,
    sessions: &mut Sessions,
    internal: &mut UnboundedReceiver<Internal>,
) -> String {
    let reason = loop {
        tokio::select! {
            msg = client.recv() => {
                let Some(msg) = msg else { break "connessione al server persa".to_owned() };
                if let ServerMessage::Error { code: ErrorCode::Replaced, .. } = msg {
                    break "questo ID è stato collegato da un'altra installazione".to_owned();
                }
                handle_server_message(ctx, &client, sessions, msg).await;
            }
            cmd = commands.recv() => match cmd {
                // Quick access only shares this screen (the UI offers no way to connect).
                Some(Command::Connect { .. }) if crate::QUICK => {
                    ctx.log("error", "Quick access non permette di controllare altri dispositivi");
                }
                Some(Command::Connect { target, credential }) => {
                    // Same network: connect directly, no server. Otherwise go through it.
                    if try_lan_connect(ctx, &target, &credential) {
                        ctx.log("info", format!("connessione a {target} in rete locale..."));
                    } else {
                        ctx.log("info", format!("connessione a {target}..."));
                        sessions.pending_connects.insert(target.clone(), credential);
                        client.send(ClientMessage::Connect { target });
                    }
                }
                Some(Command::ConnectViaServer { target, credential }) => {
                    ctx.log("info", format!("connessione a {target} tramite server..."));
                    sessions.pending_connects.insert(target.clone(), credential);
                    client.send(ClientMessage::Connect { target });
                }
                Some(Command::CancelConnect) => {
                    sessions.pending_connects.clear();
                    let cancelled: Vec<_> = ctx.shared.connecting.lock().unwrap().drain().collect();
                    for (_, cancel) in cancelled {
                        let _ = cancel.send("connessione annullata".into());
                    }
                    ctx.log("error", "connessione annullata");
                }
                None => break "chiusura".to_owned(),
            },
            Some(msg) = internal.recv() => handle_internal(ctx, sessions, msg).await,
        }
    };

    // Connections requested on this server connection will never get their session.
    sessions.pending_connects.clear();
    reason
}

async fn handle_server_message(
    ctx: &Ctx,
    client: &SignalingClient,
    sessions: &mut Sessions,
    msg: ServerMessage,
) {
    match msg {
        ServerMessage::IncomingSession {
            session,
            from,
            ice_servers,
        } => {
            match PeerSession::start(
                Role::Answerer,
                session,
                session_ice(&ice_servers, client),
                false,
                client.sender(),
            )
            .await
            {
                Ok(mut peer) => {
                    watch_state(&ctx.app, &peer, session);
                    let channels = peer.take_data_channels().expect("fresh session");
                    let video = peer.video_sender().expect("answerer has a video track");
                    let (app, shared, info) = (ctx.app.clone(), ctx.shared.clone(), peer.info());
                    let internal = sessions.internal.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Some(channels) = transport::sort_channels(channels).await {
                            serve_partner(&app, &shared, session, from, info, channels, video)
                                .await;
                        }
                        let _ = internal.send(Internal::SessionDone(session));
                    });
                    sessions.active.insert(session, peer);
                }
                Err(e) => ctx.log("error", format!("sessione in arrivo fallita: {e:#}")),
            }
        }
        ServerMessage::SessionCreated {
            session,
            peer: partner,
            ice_servers,
        } => {
            let Some(credential) = sessions.pending_connects.remove(&partner) else {
                tracing::warn!(session, "session created without pending connect");
                client.send(ClientMessage::Leave { session });
                return;
            };
            match PeerSession::start(
                Role::Offerer,
                session,
                session_ice(&ice_servers, client),
                false,
                client.sender(),
            )
            .await
            {
                Ok(mut peer) => {
                    watch_state(&ctx.app, &peer, session);
                    let channels = peer.take_data_channels().expect("fresh session");
                    let tracks = peer.take_video_tracks().expect("fresh session");
                    let (app, shared, info) = (ctx.app.clone(), ctx.shared.clone(), peer.info());
                    let internal = sessions.internal.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Some(channels) = transport::sort_channels(channels).await {
                            view_partner(
                                &app,
                                &shared,
                                session,
                                partner,
                                info,
                                channels,
                                tracks,
                                &credential,
                            )
                            .await;
                        }
                        let _ = internal.send(Internal::SessionDone(session));
                    });
                    sessions.active.insert(session, peer);
                }
                Err(e) => ctx.log("error", format!("avvio sessione fallito: {e:#}")),
            }
        }
        ServerMessage::Signal { session, payload } => {
            if let Some(peer) = sessions.active.get_mut(&session)
                && let Err(e) = peer.handle_signal(payload).await
            {
                tracing::warn!(session, "bad signal: {e:#}");
            }
        }
        ServerMessage::SessionClosed { session, reason } => {
            // An outgoing session closed while still authenticating: stop waiting now.
            if let Some(cancel) = ctx.shared.connecting.lock().unwrap().remove(&session) {
                let _ = cancel.send("il partner ha chiuso la connessione".into());
            }
            ctx.shared.approvals.lock().unwrap().remove(&session);
            emit(&ctx.app, UiEvent::ApprovalClosed { session });
            if let Some(peer) = sessions.active.remove(&session) {
                peer.shutdown().await;
                tracing::info!(session, "session closed by server: {reason}");
            }
            // Closing the connection ourselves raises no state event: end the viewer window /
            // the sharing explicitly, or the picture would stay frozen with no end popup.
            if let Some(viewer) = ctx.shared.viewers.lock().unwrap().get(&session) {
                let _ = viewer.commands.send(ViewerCommand::Stop);
            }
            if let Some(host) = ctx.shared.hosting.lock().unwrap().get(&session) {
                let _ = host.send(HostCommand::Stop);
            }
        }
        ServerMessage::Error { message, .. } => {
            // A failed Connect never yields SessionCreated; forget its password.
            sessions.pending_connects.clear();
            ctx.log("error", message);
        }
        ServerMessage::Registered { .. } => {}
    }
}

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

// ---- LAN mode (same network, no server) ---------------------------------------------

/// Starts advertising this device and browsing for peers on the LAN.
fn start_lan_discovery(ctx: &Ctx, id: &str, port: u16) {
    let Ok(id) = id.parse::<PeerId>() else { return };
    match transport::lan::LanDiscovery::start(&id, port) {
        Ok(discovery) => {
            *ctx.shared.lan.lock().unwrap() = Some(std::sync::Arc::new(discovery));
            emit(&ctx.app, UiEvent::LanPeers { peers: Vec::new() });
        }
        Err(e) => tracing::warn!("LAN discovery unavailable: {e:#}"),
    }
}

/// This device's id (from the server, or cached for offline LAN), if known.
fn own_id(shared: &Shared) -> Option<PeerId> {
    shared
        .status
        .lock()
        .unwrap()
        .id
        .as_deref()
        .or(shared.store.cached_id().as_deref())
        .and_then(|s| s.parse().ok())
}

/// If `target` is a peer on the local network, connects to it directly (no server) and
/// returns true. Otherwise returns false (caller falls back to the server).
fn try_lan_connect(ctx: &Ctx, target: &PeerId, credential: &Credential) -> bool {
    let addrs = {
        let lan = ctx.shared.lan.lock().unwrap();
        match lan.as_ref().and_then(|d| d.resolve(target)) {
            Some(addrs) => addrs,
            None => return false,
        }
    };
    let Some(id) = own_id(&ctx.shared) else {
        return false;
    };
    let (app, shared, commands, target, credential) = (
        ctx.app.clone(),
        ctx.shared.clone(),
        ctx.commands.clone(),
        target.clone(),
        credential.clone(),
    );
    tauri::async_runtime::spawn(async move {
        lan_view(app, shared, commands, &addrs, id, target, credential).await;
    });
    true
}

/// Viewer side of a LAN session: connect directly, run the WebRTC handshake over the LAN
/// link, then view the partner exactly as with the server. If the peer cannot be reached
/// directly (firewall, other subnet, macOS local network permission...), retries through
/// the server.
async fn lan_view(
    app: AppHandle,
    shared: std::sync::Arc<Shared>,
    commands: UnboundedSender<Command>,
    addrs: &[std::net::SocketAddr],
    own_id: PeerId,
    target: PeerId,
    credential: Credential,
) {
    let lan = match transport::lan::connect(addrs, &own_id, &target).await {
        Ok(lan) => lan,
        Err(e) => {
            log(
                &app,
                "warn",
                format!("rete locale non raggiungibile ({e:#}): provo tramite server"),
            );
            let _ = commands.send(Command::ConnectViaServer { target, credential });
            return;
        }
    };
    // No ICE servers: on the same LAN the host candidate is reached directly.
    match PeerSession::start(Role::Offerer, lan.session, &[], false, lan.signal).await {
        Ok(mut peer) => {
            watch_state(&app, &peer, lan.session);
            let tracks = peer.take_video_tracks().expect("fresh session");
            let channels = peer.take_data_channels().expect("fresh session");
            let info = peer.info();
            let body = {
                let (app, shared) = (app.clone(), shared.clone());
                tauri::async_runtime::spawn(async move {
                    if let Some(channels) = transport::sort_channels(channels).await {
                        view_partner(
                            &app,
                            &shared,
                            lan.session,
                            target,
                            info,
                            channels,
                            tracks,
                            &credential,
                        )
                        .await;
                    }
                })
            };
            pump_lan_signals(peer, lan.incoming, body).await;
        }
        Err(e) => log(
            &app,
            "error",
            format!("avvio sessione in rete locale fallito: {e:#}"),
        ),
    }
}

/// Host side of an incoming LAN session: run the handshake over the LAN link, then share
/// the screen exactly as with the server.
fn lan_serve(ctx: &Ctx, lan: transport::lan::LanSession) {
    let (app, shared) = (ctx.app.clone(), ctx.shared.clone());
    tauri::async_runtime::spawn(async move {
        let (session, from) = (lan.session, lan.peer.clone());
        match PeerSession::start(Role::Answerer, session, &[], false, lan.signal).await {
            Ok(mut peer) => {
                watch_state(&app, &peer, session);
                let video = peer.video_sender().expect("answerer has a video track");
                let channels = peer.take_data_channels().expect("fresh session");
                let info = peer.info();
                let body = {
                    let (app, shared) = (app.clone(), shared.clone());
                    tauri::async_runtime::spawn(async move {
                        if let Some(channels) = transport::sort_channels(channels).await {
                            serve_partner(&app, &shared, session, from, info, channels, video)
                                .await;
                        }
                    })
                };
                pump_lan_signals(peer, lan.incoming, body).await;
            }
            Err(e) => log(
                &app,
                "warn",
                format!("sessione in rete locale fallita: {e:#}"),
            ),
        }
    });
}

/// Drives trickle-ICE over the LAN link: feed every incoming payload to the session until
/// the body (view/serve) finishes or the LAN link closes, then close the session.
async fn pump_lan_signals(
    mut peer: PeerSession,
    mut incoming: tokio::sync::mpsc::UnboundedReceiver<signaling_proto::SignalPayload>,
    body: tauri::async_runtime::JoinHandle<()>,
) {
    tokio::pin!(body);
    loop {
        tokio::select! {
            payload = incoming.recv() => match payload {
                Some(payload) => {
                    if let Err(e) = peer.handle_signal(payload).await {
                        tracing::debug!("LAN signal failed: {e:#}");
                    }
                }
                None => break,
            },
            _ = &mut body => break,
        }
    }
    body.abort();
    peer.close().await;
}

/// Reports the network path (LAN / direct / TURN relay) of an established session to the
/// UI, and again whenever it changes, until the session ends.
/// Follows a session's network path; when `viewing`, also reports every second the traffic
/// and the CPU/memory this app uses, for the viewer toolbar.
fn watch_path(app: &AppHandle, info: SessionInfo, session: SessionId, viewing: bool) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut last = None;
        let mut tick = 0u64;
        let mut traffic: Option<((u64, u64), Instant)> = None;
        let sampler = Arc::new(Mutex::new(None::<stream::usage::UsageSampler>));
        let mut usage: Option<stream::usage::Usage> = None;
        loop {
            if tick.is_multiple_of(3) {
                let path = info.connection_path().await;
                if path.is_some() && path != last {
                    last = path;
                    let service = app.state::<Service>();
                    service.shared.view_event(
                        &app,
                        session,
                        UiEvent::ConnectionPath { session, path },
                    );
                }
            }
            if viewing {
                if tick.is_multiple_of(2) {
                    let sampler = sampler.clone();
                    usage = tauri::async_runtime::spawn_blocking(move || {
                        let mut sampler = sampler.lock().unwrap();
                        sampler
                            .get_or_insert_with(stream::usage::UsageSampler::new)
                            .sample()
                    })
                    .await
                    .ok()
                    .flatten();
                }
                if let Some(now) = info.traffic().await {
                    let at = Instant::now();
                    if let Some(((sent, received), then)) = traffic {
                        let secs = at.duration_since(then).as_secs_f64().max(0.001);
                        let kbps = |bytes: u64| (bytes as f64 * 8.0 / 1000.0 / secs) as u32;
                        app.state::<Service>().shared.view_event(
                            &app,
                            session,
                            UiEvent::ViewStats {
                                session,
                                rx_kbps: kbps(now.1.saturating_sub(received)),
                                tx_kbps: kbps(now.0.saturating_sub(sent)),
                                cpu: usage.map(|u| u.cpu_percent),
                                ram_mb: usage.map(|u| (u.memory_bytes / 1_000_000) as u32),
                            },
                        );
                    }
                    traffic = Some((now, at));
                }
            }
            tokio::time::sleep(Duration::from_secs(1)).await;
            tick += 1;
            let shared = app.state::<crate::service::Service>();
            if !shared.is_session_active(session) {
                break;
            }
        }
    });
}

/// Logs WebRTC connection failures of a session.
fn watch_state(app: &AppHandle, peer: &PeerSession, session: SessionId) {
    let mut state = peer.state();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        while state.changed().await.is_ok() {
            let current = *state.borrow_and_update();
            tracing::info!(session, %current, "peer connection state");
            match current {
                ConnectionState::Failed => {
                    log(&app, "error", "connessione con il partner persa");
                    // Do not leave a frozen picture or a stale "shared" banner behind.
                    let service = app.state::<Service>();
                    service.close_view(session);
                    service.stop_hosting(session);
                    break;
                }
                ConnectionState::Closed => {
                    // The peer link is gone: close the viewer / sharing banner if still open.
                    let service = app.state::<Service>();
                    service.close_view(session);
                    service.stop_hosting(session);
                    break;
                }
                _ => {}
            }
        }
    });
}

async fn wait_open(channel: &Arc<dyn DataChannel>) -> bool {
    matches!(
        tokio::time::timeout(OPEN_TIMEOUT, diag::wait_open(channel)).await,
        Ok(Ok(()))
    )
}

/// Host side: verify the password, ask the local user, then share the screen.
async fn serve_partner(
    app: &AppHandle,
    shared: &Arc<Shared>,
    session: SessionId,
    from: PeerId,
    info: SessionInfo,
    channels: SessionChannels,
    video: Arc<VideoSender>,
) {
    let SessionChannels {
        control: channel,
        files,
        audio,
    } = channels;
    if !wait_open(&channel).await {
        log(
            app,
            "warn",
            format!("{from}: connessione diretta non riuscita"),
        );
        return;
    }
    let Some(transcript) = info.transcript().await else {
        log(
            app,
            "error",
            "impossibile leggere i certificati della connessione",
        );
        return;
    };

    let temporary = shared.password();
    // Which password the viewer tries, for counting wrong guesses.
    let personal_attempt = AtomicBool::new(false);
    let refuse_early = |personal: bool| {
        personal_attempt.store(personal, Ordering::Relaxed);
        let locked = personal && shared.personal_locked_out();
        locked.then_some(Refusal::TooManyAttempts)
    };
    // Guesses made in parallel all check the same password: one that is right but was
    // replaced meanwhile (after wrong guesses) no longer counts.
    let approve = async {
        if shared.password() != temporary {
            return false;
        }
        ask_user(app, shared, session, &from).await
    };
    let personal = shared.personal.lock().unwrap().clone();
    let secrets = handshake::HostSecrets {
        temporary: &temporary,
        personal: personal.as_deref(),
    };
    let result =
        handshake::authenticate_host(&channel, secrets, transcript, refuse_early, approve).await;
    let unattended = matches!(result, Ok(Access::Unattended));
    match result {
        Ok(Access::Approved) => {}
        // Same for parallel guesses of the personal password.
        Ok(Access::Unattended) if shared.personal_locked_out() => {
            log(app, "warn", format!("{from}: troppi tentativi falliti"));
            return;
        }
        Ok(Access::Unattended) => {
            crate::show_main_window(app);
            log(
                app,
                "info",
                format!("{from} connesso con la password personale (accesso non presidiato)"),
            );
        }
        Err(HandshakeError::BadPassword) => {
            let personal = personal_attempt.load(Ordering::Relaxed);
            shared.record_failure(app, personal);
            log(app, "warn", format!("{from}: password errata"));
            linger(&channel).await;
            return;
        }
        Err(e) => {
            log(app, "warn", format!("{from}: {e}"));
            linger(&channel).await;
            return;
        }
    }

    let (stop_tx, stop_rx) = mpsc::unbounded_channel();
    if unattended {
        // The personal password may have been removed while the handshake ran.
        if shared.personal.lock().unwrap().as_ref() != personal.as_ref() {
            log(
                app,
                "warn",
                format!("{from}: password personale non più valida"),
            );
            return;
        }
        shared.unattended.lock().unwrap().insert(session);
    }
    shared.hosting.lock().unwrap().insert(session, stop_tx);
    watch_path(app, info.clone(), session, false);
    emit(
        app,
        UiEvent::HostingStarted {
            session,
            peer: from.to_string(),
        },
    );
    let hosting_since = Instant::now();
    let config = StreamConfig {
        source: Source::Screen { display_id: None },
        // Windows: use the SYSTEM agent (if running) so UAC/lock screen work.
        agent_product: Some(crate::agent_product().to_owned()),
        ..StreamConfig::default()
    };
    let mut chat = ChatLimiter::default();
    session::serve_host(
        channel,
        files,
        audio,
        video,
        config,
        stop_rx,
        |event| match event {
            HostEvent::Started { width, height } => log(
                app,
                "info",
                format!("schermo condiviso con {from} ({width}x{height})"),
            ),
            HostEvent::Failed(e) => log(app, "error", capture_error_message(&e)),
            HostEvent::ControlUnavailable(e) => log(
                app,
                "warn",
                format!("{from} può vedere ma non controllare: {e}"),
            ),
            HostEvent::ControlActive => log(
                app,
                "info",
                format!("{from} sta controllando mouse e tastiera"),
            ),
            HostEvent::ControlFailing(e) => log(
                app,
                "warn",
                format!("il controllo remoto da {from} non funziona: {e}"),
            ),
            HostEvent::Chat(text) => {
                let Some(text) = chat.accept(text) else {
                    return;
                };
                if chat.may_raise() {
                    crate::show_main_window(app);
                }
                emit(
                    app,
                    UiEvent::HostChat {
                        session,
                        peer: from.to_string(),
                        text,
                    },
                );
            }
            // Logged to file by the stream crate; too frequent for the activity list.
            HostEvent::Quality { .. } => {}
            HostEvent::Clipboard(message) => {
                log(app, "info", format!("appunti con {from}: {message}"))
            }
            HostEvent::Ended(reason) => log(
                app,
                "info",
                format!("condivisione con {from} terminata: {reason}"),
            ),
        },
    )
    .await;
    shared.hosting.lock().unwrap().remove(&session);
    shared.unattended.lock().unwrap().remove(&session);
    emit(
        app,
        UiEvent::HostingEnded {
            session,
            peer: from.to_string(),
            duration_secs: hosting_since.elapsed().as_secs(),
        },
    );
    // The popup with the duration lives in the main window, which may be in the tray.
    crate::show_main_window(app);
    // A password that served one connection is not reused for the next one.
    shared.rotate_password(app);
    log(
        app,
        "info",
        "connessione terminata: nuova password generata",
    );
}

/// Turns capture errors into something the user can act on.
fn capture_error_message(error: &str) -> String {
    let lower = error.to_lowercase();
    if lower.contains("permission") || lower.contains("tcc") || lower.contains("permessi") {
        "impossibile catturare lo schermo: concedi a Kronos Remote Support il permesso \
         \"Registrazione schermo\" (Impostazioni di Sistema > Privacy e sicurezza) e riavvia l'app"
            .into()
    } else if lower.contains("not supported") {
        "la condivisione dello schermo non è ancora disponibile su questo sistema".into()
    } else {
        format!("impossibile condividere lo schermo: {error}")
    }
}

/// Keeps a refused session open briefly so the "denied" message reaches the viewer
/// (closing at once could drop it, leaving the viewer waiting).
async fn linger(channel: &Arc<dyn DataChannel>) {
    let _ = tokio::time::timeout(REFUSAL_LINGER, async {
        while channel.poll().await.is_some() {}
    })
    .await;
}

/// Shows the approval popup and waits for the answer (rejects on timeout).
async fn ask_user(
    app: &AppHandle,
    shared: &Arc<Shared>,
    session: SessionId,
    from: &PeerId,
) -> bool {
    let (tx, rx) = oneshot::channel();
    shared.approvals.lock().unwrap().insert(session, tx);
    crate::show_main_window(app);
    emit(
        app,
        UiEvent::ApprovalRequest {
            session,
            peer: from.to_string(),
            timeout_secs: APPROVAL_TIMEOUT.as_secs(),
        },
    );
    let accepted = matches!(
        tokio::time::timeout(APPROVAL_TIMEOUT, rx).await,
        Ok(Ok(true))
    );
    shared.approvals.lock().unwrap().remove(&session);
    emit(app, UiEvent::ApprovalClosed { session });
    if !accepted {
        log(app, "info", format!("connessione da {from} rifiutata"));
    }
    accepted
}

/// Viewer side: authenticate with the partner's password, then receive the screen.
#[allow(clippy::too_many_arguments)]
async fn view_partner(
    app: &AppHandle,
    shared: &Arc<Shared>,
    session: SessionId,
    partner: PeerId,
    info: SessionInfo,
    channels: SessionChannels,
    tracks: UnboundedReceiver<Arc<dyn TrackRemote>>,
    credential: &Credential,
) {
    let SessionChannels {
        control: channel,
        files,
        audio,
    } = channels;
    let (cancel_tx, cancel_rx) = oneshot::channel::<String>();
    shared.connecting.lock().unwrap().insert(session, cancel_tx);
    let timeout = if credential.personal {
        PERSONAL_AUTH_TIMEOUT
    } else {
        AUTH_TIMEOUT
    };
    let setup = async {
        if !wait_open(&channel).await {
            return Err("impossibile stabilire la connessione con il partner".to_owned());
        }
        let transcript = info
            .transcript()
            .await
            .ok_or("impossibile leggere i certificati della connessione")?;
        let waiting = if credential.personal {
            "verifica della password personale..."
        } else {
            "verifica password, in attesa di conferma dal partner..."
        };
        log(app, "info", waiting);
        match tokio::time::timeout(
            timeout,
            handshake::authenticate_viewer(
                &channel,
                &credential.secret,
                credential.personal,
                transcript,
            ),
        )
        .await
        {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => Err(e.to_string()),
            Err(_) => Err("nessuna risposta dal partner".to_owned()),
        }
    };
    let outcome = tokio::select! {
        result = setup => result,
        reason = cancel_rx => Err(reason.unwrap_or_else(|_| "connessione interrotta".into())),
    };
    shared.connecting.lock().unwrap().remove(&session);
    if let Err(message) = outcome {
        log(app, "error", message);
        return;
    }

    let (commands_tx, commands) = mpsc::unbounded_channel();
    shared.viewers.lock().unwrap().insert(
        session,
        ViewerCtl {
            commands: commands_tx,
            frames: None,
            in_flight: 0,
            resyncing: false,
            backlog: Vec::new(),
        },
    );
    let viewing_since = Instant::now();
    let opened = UiEvent::ViewOpened {
        session,
        peer: partner.to_string(),
    };
    // The main window stops its "connecting" spinner; the viewer window gets it on attach.
    emit(app, opened.clone());
    shared.view_event(app, session, opened);
    // Each remote screen gets its own window; the main one (ID, password, approvals)
    // stays usable, e.g. while others connect to this computer.
    crate::open_view_window(app, session, &partner.to_string());
    watch_path(app, info.clone(), session, true);

    let mut last_rtt_emit: Option<Instant> = None;
    let mut ended = String::from("sessione terminata");
    let mut chat = ChatLimiter::default();
    session::run_viewer(
        channel,
        files,
        audio,
        tracks,
        commands,
        true,
        |event| match event {
            ViewerEvent::Started { width, height } => shared.view_event(
                app,
                session,
                UiEvent::ViewSize {
                    session,
                    width,
                    height,
                },
            ),
            ViewerEvent::Frame(frame) => shared.send_frame(session, &frame),
            ViewerEvent::Rtt(rtt) => {
                if last_rtt_emit.is_none_or(|t| t.elapsed() >= Duration::from_secs(1)) {
                    last_rtt_emit = Some(Instant::now());
                    shared.view_event(
                        app,
                        session,
                        UiEvent::ViewRtt {
                            session,
                            ms: rtt.as_secs_f64() * 1000.0,
                        },
                    );
                }
            }
            ViewerEvent::Resync => shared.view_event(app, session, UiEvent::ViewResync { session }),
            ViewerEvent::PartnerUsage { cpu, ram_mb } => shared.view_event(
                app,
                session,
                UiEvent::ViewPartnerUsage {
                    session,
                    cpu,
                    ram_mb,
                },
            ),
            ViewerEvent::Quality { kbps, fps, limited } => shared.view_event(
                app,
                session,
                UiEvent::ViewQuality {
                    session,
                    kbps,
                    fps,
                    limited,
                },
            ),
            ViewerEvent::Displays { displays, current } => shared.view_event(
                app,
                session,
                UiEvent::ViewDisplays {
                    session,
                    displays,
                    current,
                },
            ),
            ViewerEvent::ControlUnavailable(message) => {
                log(app, "warn", format!("controllo non disponibile: {message}"));
                shared.view_event(
                    app,
                    session,
                    UiEvent::ViewControlUnavailable { session, message },
                );
            }
            ViewerEvent::Clipboard(message) => {
                log(app, "info", format!("appunti con {partner}: {message}"));
                shared.view_event(app, session, UiEvent::ViewNotice { session, message });
            }
            ViewerEvent::AudioUnavailable(message) => {
                log(app, "warn", format!("audio non disponibile: {message}"));
                shared.view_event(
                    app,
                    session,
                    UiEvent::ViewAudio {
                        session,
                        enabled: false,
                    },
                );
                shared.view_event(app, session, UiEvent::ViewNotice { session, message });
            }
            ViewerEvent::ClipboardEnabled(enabled) => {
                shared.view_event(app, session, UiEvent::ViewClipboard { session, enabled })
            }
            ViewerEvent::Chat(text) => {
                if let Some(text) = chat.accept(text) {
                    shared.view_event(app, session, UiEvent::ViewChat { session, text });
                }
            }
            ViewerEvent::Error(e) => {
                log(
                    app,
                    "error",
                    format!("il partner non può condividere lo schermo: {e}"),
                );
                ended = e;
            }
            ViewerEvent::Ended(reason) => {
                if ended == "sessione terminata" {
                    ended = reason;
                }
            }
        },
    )
    .await;

    shared.viewers.lock().unwrap().remove(&session);
    log(
        app,
        "info",
        format!("sessione con {partner} chiusa: {ended}"),
    );
    emit(
        app,
        UiEvent::ViewClosed {
            session,
            reason: ended,
            duration_secs: viewing_since.elapsed().as_secs(),
        },
    );
    crate::show_main_window(app);
}
