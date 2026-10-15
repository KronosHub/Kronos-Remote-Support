//! What happens on an authenticated session: the host streams its screen and answers
//! control messages; the viewer receives frames, measures latency and asks for keyframes.

use std::sync::Arc;
use std::time::{Duration, Instant};

use protocol::ControlMsg;
use tokio::sync::mpsc;
use transport::video::{Assembled, SharedPacketStats, VideoFrame, VideoReceiver, VideoSender};
use transport::{DataChannel, DataChannelEvent, TrackRemote};

use crate::rate::{self, RateController};
use crate::sharing::ClipboardShare;
use crate::{StreamChange, StreamConfig, StreamHandle};

const PING_INTERVAL: Duration = Duration::from_secs(1);
const USAGE_INTERVAL: Duration = Duration::from_secs(2);
/// Do not flood the host with keyframe requests while packets keep getting lost.
const MIN_KEYFRAME_REQUEST_GAP: Duration = Duration::from_millis(500);
/// Nothing heard from the partner for this long (it pings every second, the host answers and
/// reports usage every 2 s): the session is over even if no goodbye arrived.
const PEER_TIMEOUT: Duration = Duration::from_secs(20);
/// How often that is checked.
const LIVENESS_CHECK: Duration = Duration::from_secs(5);

/// Notes that the partner just spoke (any control message, even one we ignore).
fn heard<T>(at: &mut Instant, value: T) -> T {
    *at = Instant::now();
    value
}
/// Pause between attempts to restart a stream whose capture stopped.
const RESTART_RETRY: Duration = Duration::from_secs(2);
/// Pause between attempts to share a screen while there is none (e.g. no monitor).
const NO_SCREEN_RETRY: Duration = Duration::from_secs(5);

/// The injector for this host session: the local one, or the SYSTEM agent on Windows
/// (so remote control reaches UAC prompts and the lock screen).
enum HostInput {
    Local(input::InputSink),
    #[cfg(windows)]
    Agent(agent::client::AgentInput),
}

impl HostInput {
    /// Starts the agent injector when `agent_product` is set and the agent is running,
    /// otherwise the local one. Blocking (may wait on an OS permission prompt).
    fn start(agent_product: Option<String>) -> anyhow::Result<Self> {
        #[cfg(windows)]
        if let Some(product) = agent_product
            && agent::windows::agent_available(&product)
        {
            return Ok(Self::Agent(agent::client::AgentInput::start(
                move || async move { agent::windows::connect(&product).await },
            )?));
        }
        let _ = agent_product;
        Ok(Self::Local(input::InputSink::start()?))
    }

    fn send(&self, event: protocol::InputEvent) {
        match self {
            Self::Local(sink) => sink.send(event),
            #[cfg(windows)]
            Self::Agent(sink) => sink.send(event),
        }
    }

    fn set_area(&self, area: input::Area) {
        match self {
            Self::Local(sink) => sink.set_area(area),
            #[cfg(windows)]
            Self::Agent(sink) => sink.set_area(area),
        }
    }

    /// Health of remote control for the user. The agent reports no per-event failures, so
    /// only its applied count is known.
    fn health(&self) -> input::InputHealth {
        match self {
            Self::Local(sink) => sink.health(),
            #[cfg(windows)]
            Self::Agent(sink) => input::InputHealth {
                applied: sink.sent(),
                failed: 0,
                last_error: None,
            },
        }
    }
}

async fn send(channel: &Arc<dyn DataChannel>, msg: &ControlMsg) {
    let _ = channel.send_text(&msg.to_json()).await;
}

/// Tells the other side the session ends and waits (at most `BYE_FLUSH`) until the
/// message has left: the connection is closed right after, which would otherwise drop it.
async fn send_bye(channel: &Arc<dyn DataChannel>, reason: &str) {
    send(
        channel,
        &ControlMsg::Bye {
            reason: reason.to_owned(),
        },
    )
    .await;
    let _ = tokio::time::timeout(BYE_FLUSH, async {
        while channel.outstanding_bytes().await.unwrap_or(0) > 0 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await;
}

/// Longest wait for a `Bye` to leave before the connection closes.
const BYE_FLUSH: Duration = Duration::from_secs(1);

/// Next control message; `None` once the channel is closed.
async fn recv(channel: &Arc<dyn DataChannel>) -> Option<ControlMsg> {
    loop {
        match channel.poll().await? {
            DataChannelEvent::OnMessage(msg) => {
                if let Some(msg) = ControlMsg::from_json(&msg.data) {
                    return Some(msg);
                }
            }
            DataChannelEvent::OnClose => return None,
            _ => {}
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum HostEvent {
    Started {
        width: u32,
        height: u32,
    },
    Failed(String),
    /// Screen sharing works but remote control could not be enabled.
    ControlUnavailable(String),
    /// Remote control works: the first viewer input was applied.
    ControlActive,
    /// Viewer input is being received but replaying it fails.
    ControlFailing(String),
    /// A chat message from the viewer.
    Chat(String),
    /// Clipboard sharing news for the user (files received, sent, refused).
    Clipboard(String),
    /// Adaptive quality changed the stream's bitrate.
    Quality {
        kbps: u32,
        fps: u32,
        limited: bool,
    },
    /// The viewer blocked (or unblocked) this computer's own mouse and keyboard.
    InputBlocked(bool),
    /// A clipboard file transfer moved on.
    Transfer(crate::files::TransferProgress),
    Ended(String),
}

pub enum HostCommand {
    Stop,
    /// The viewer left (reported by the server): end without blaming the host's user.
    PartnerLeft,
    /// A chat message typed on the host, to show on the viewer.
    Chat(String),
    /// Stop the clipboard file transfer under way.
    CancelTransfer,
}

/// Host: streams until the viewer leaves, the channel closes or `Stop` arrives.
pub async fn serve_host(
    channel: Arc<dyn DataChannel>,
    files: tokio::sync::oneshot::Receiver<Arc<dyn DataChannel>>,
    audio: tokio::sync::oneshot::Receiver<Arc<dyn DataChannel>>,
    video: Arc<VideoSender>,
    config: StreamConfig,
    mut commands: mpsc::UnboundedReceiver<HostCommand>,
    mut on_event: impl FnMut(HostEvent),
) {
    let allow_control = config.allow_control;
    // A Mac without a monitor gets a virtual one for the session (ADR 0048).
    let mut screen_keeper: Option<VirtualScreen> = None;
    ensure_screen(&config, &mut screen_keeper).await;
    // `None` while there is no screen to share: the session stays open (chat, files) and
    // the capture is tried again every few seconds.
    let mut handle = match crate::start(config.clone(), video.clone()).await {
        Ok(handle) => Some(handle),
        Err(e) => {
            let message = screen_unavailable_message(&e);
            tracing::warn!("screen not shared yet: {e:#}");
            send(
                &channel,
                &ControlMsg::ScreenUnavailable {
                    message: message.clone(),
                },
            )
            .await;
            on_event(HostEvent::Failed(message));
            None
        }
    };
    // The most this picture is worth: more for large screens (`rate::max_bitrate_for`).
    let pixels = |h: &StreamHandle| u64::from(h.width) * u64::from(h.height);
    let mut rate = RateController::new(rate::max_bitrate_for(
        handle.as_ref().map_or(rate::FULL_HD_PIXELS, pixels),
        config.bitrate_bps,
    ));
    if let Some(handle) = &handle {
        send(
            &channel,
            &ControlMsg::StreamStarted {
                width: handle.width,
                height: handle.height,
            },
        )
        .await;
        on_event(HostEvent::Started {
            width: handle.width,
            height: handle.height,
        });
    }

    // Starting the injector may block on an OS permission prompt: keep it off the runtime.
    let agent_product = config.agent_product.clone();
    let input = if allow_control {
        match tokio::task::spawn_blocking(move || HostInput::start(agent_product)).await {
            Ok(Ok(sink)) => Some(sink),
            Ok(Err(e)) => {
                let message = format!("{e:#}");
                send(
                    &channel,
                    &ControlMsg::InputUnavailable {
                        message: message.clone(),
                    },
                )
                .await;
                on_event(HostEvent::ControlUnavailable(message));
                None
            }
            Err(e) => {
                on_event(HostEvent::ControlUnavailable(e.to_string()));
                None
            }
        }
    } else {
        None
    };
    if let Some(handle) = &handle {
        if let Some(sink) = &input {
            sink.set_area(area(handle.bounds));
        }
        send_displays(&channel, &config, handle.display_id).await;
    }
    let usage_task = tokio::spawn(report_usage(channel.clone()));

    // Report how remote control is going: first success, and failures (once each kind).
    let mut health_check = tokio::time::interval(Duration::from_secs(2));
    let mut reported_active = false;
    let mut reported_error: Option<String> = None;
    // Shared only once the viewer turns it on (`ClipboardMode`).
    let mut clipboard = ClipboardShare::new(files);
    // No display sleep or screen lock while someone is controlling this machine.
    let awake = crate::awake::KeepAwake::new();
    // Ends the session if the viewer goes silent (its app hung or the network died
    // without the connection noticing).
    let mut last_heard = Instant::now();
    let mut liveness = tokio::time::interval(LIVENESS_CHECK);
    // This computer's own mouse and keyboard, blocked on the viewer's request; `released`
    // fires when the person here takes them back (Esc ×3).
    let mut local_block: Option<input::block::LocalBlock> = None;
    let (released_tx, mut released) = mpsc::unbounded_channel::<()>();
    // Sound: off until the viewer asks for it.
    let mut sound = crate::sound::HostSound::new(audio);
    // HD on by default; the viewer may limit the stream to 1080p (kept across restarts).
    let mut high_resolution = true;
    // Set while the capture is down and must be started again.
    let mut restart_at = handle
        .is_none()
        .then(|| tokio::time::Instant::now() + NO_SCREEN_RETRY);
    let reason = loop {
        tokio::select! {
            event = clipboard.next() => match event {
                crate::sharing::ClipEvent::Progress(progress) => {
                    on_event(HostEvent::Transfer(progress));
                }
                event => {
                    if let Some(message) = clipboard.handle(event, &channel).await {
                        on_event(HostEvent::Clipboard(message));
                    }
                }
            },
            _ = health_check.tick(), if input.is_some() => {
                let Some(health) = input.as_ref().map(HostInput::health) else { continue };
                if health.applied > 0 && !reported_active {
                    reported_active = true;
                    on_event(HostEvent::ControlActive);
                }
                if let Some(error) = health.last_error
                    && health.failed > 0
                    && reported_error.as_ref() != Some(&error)
                {
                    reported_error = Some(error.clone());
                    send(&channel, &ControlMsg::InputUnavailable { message: error.clone() }).await;
                    on_event(HostEvent::ControlFailing(error));
                }
            }
            Some(()) = released.recv() => {
                local_block = None;
                on_event(HostEvent::InputBlocked(false));
                let message = Some("sbloccati da chi è davanti al computer (Esc ×3)".to_owned());
                send(&channel, &ControlMsg::InputBlocked { blocked: false, message }).await;
            }
            change = async {
                match handle.as_mut() {
                    Some(handle) => handle.changed().await,
                    None => std::future::pending().await,
                }
            }, if restart_at.is_none() => match change {
                StreamChange::Resized { width, height } => {
                    // Resolution lowered or restored for the bandwidth: the viewer restarts
                    // its decoder on the new size, as for another monitor.
                    send(&channel, &ControlMsg::StreamStarted { width, height }).await;
                }
                StreamChange::Ended => {
                    // E.g. leaving the login or lock screen replaced the display or changed its
                    // size: without a restart the viewer would keep a frozen picture.
                    tracing::warn!("screen capture stopped, restarting it");
                    restart_at = Some(tokio::time::Instant::now());
                }
            },
            _ = tokio::time::sleep_until(restart_at.unwrap_or_else(tokio::time::Instant::now)),
                if restart_at.is_some() =>
            {
                let restarted = match handle.as_mut() {
                    Some(current) => restart_stream(&config, &video, current).await,
                    // No screen so far: maybe a monitor was plugged in (or, on a Mac, the
                    // virtual one can be made now).
                    None => {
                        ensure_screen(&config, &mut screen_keeper).await;
                        match crate::start(config.clone(), video.clone()).await {
                            Ok(new) => {
                                rate = RateController::new(rate::max_bitrate_for(
                                    pixels(&new),
                                    config.bitrate_bps,
                                ));
                                handle = Some(new);
                                Ok(())
                            }
                            Err(e) => Err(e),
                        }
                    }
                };
                match restarted {
                    Ok(()) => {
                        let Some(handle) = handle.as_ref() else { continue };
                        restart_at = None;
                        tracing::info!(size = ?(handle.width, handle.height), "screen capture restarted");
                        handle.set_bitrate(rate.bitrate());
                        handle.set_high_resolution(high_resolution);
                        if let Some(sink) = &input {
                            sink.set_area(area(handle.bounds));
                        }
                        send(&channel, &ControlMsg::StreamStarted {
                            width: handle.width,
                            height: handle.height,
                        })
                        .await;
                        send_displays(&channel, &config, handle.display_id).await;
                        on_event(HostEvent::Started { width: handle.width, height: handle.height });
                    }
                    Err(e) => {
                        let retry = if handle.is_some() { RESTART_RETRY } else { NO_SCREEN_RETRY };
                        tracing::debug!("cannot (re)start the screen capture: {e:#}");
                        restart_at = Some(tokio::time::Instant::now() + retry);
                    }
                }
            }
            _ = liveness.tick() => {
                if last_heard.elapsed() > PEER_TIMEOUT {
                    tracing::warn!("no message from the viewer for {:?}, ending", last_heard.elapsed());
                    break "il partner non risponde".to_owned();
                }
            }
            msg = recv(&channel) => match heard(&mut last_heard, msg) {
                Some(ControlMsg::Ping { seq, sent_us }) => {
                    send(&channel, &ControlMsg::Pong { seq, sent_us }).await;
                }
                Some(ControlMsg::RequestKeyframe) => {
                    if let Some(handle) = &handle {
                        handle.request_keyframe();
                    }
                }
                Some(ControlMsg::Chat { text }) => on_event(HostEvent::Chat(text)),
                Some(ControlMsg::ClipboardMode { enabled }) => {
                    // The viewer asks; the host's own setting decides.
                    let refused = enabled && !config.allow_clipboard;
                    clipboard.set_enabled(enabled && !refused, false).await;
                    if refused {
                        send(&channel, &ControlMsg::ClipboardMode { enabled: false }).await;
                    }
                }
                Some(ControlMsg::Clipboard { text }) => clipboard.remote_text(text),
                Some(ControlMsg::TransferCancelled) => {
                    clipboard.cancel_transfer(&channel, false).await;
                    on_event(HostEvent::Clipboard("trasferimento annullato dal partner".into()));
                }
                Some(ControlMsg::BlockInput { enabled }) => {
                    let (blocked, message) = if !enabled {
                        local_block = None;
                        (false, None)
                    } else if local_block.is_some() {
                        (true, None)
                    } else if input.is_none() {
                        (false, Some("controllo remoto non attivo su questo computer".to_owned()))
                    } else {
                        let tx = released_tx.clone();
                        let started = tokio::task::spawn_blocking(move || {
                            input::block::LocalBlock::start(move || {
                                let _ = tx.send(());
                            })
                        })
                        .await;
                        match started {
                            Ok(Ok(block)) => {
                                local_block = Some(block);
                                (true, None)
                            }
                            Ok(Err(e)) => (false, Some(format!("{e:#}"))),
                            Err(e) => (false, Some(e.to_string())),
                        }
                    };
                    tracing::info!(blocked, ?message, "local input block");
                    on_event(HostEvent::InputBlocked(blocked));
                    send(&channel, &ControlMsg::InputBlocked { blocked, message }).await;
                }
                Some(ControlMsg::Audio { enabled }) => {
                    if let Err(message) = sound.set_enabled(enabled).await {
                        send(&channel, &ControlMsg::AudioUnavailable { message }).await;
                    }
                }
                Some(ControlMsg::HighResolution { enabled }) => {
                    tracing::info!(enabled, "viewer set HD");
                    high_resolution = enabled;
                    if let Some(handle) = &handle {
                        handle.set_high_resolution(enabled);
                    }
                }
                Some(ControlMsg::Report { received, lost, rtt_ms }) => {
                    let report = rate::Report {
                        received,
                        lost,
                        rtt: (rtt_ms > 0).then(|| Duration::from_millis(rtt_ms.into())),
                    };
                    tracing::debug!(received, lost, rtt_ms, "viewer report");
                    if let Some(bps) = rate.on_report(report, Instant::now())
                        && let Some(handle) = &handle
                    {
                        handle.set_bitrate(bps);
                        let kbps = bps / 1000;
                        let fps = rate::fps_for(bps, config.fps);
                        let limited = rate.limited();
                        tracing::info!(kbps, fps, received, lost, rtt_ms, "adaptive quality");
                        send(&channel, &ControlMsg::Quality { kbps, fps, limited }).await;
                        on_event(HostEvent::Quality { kbps, fps, limited });
                    }
                }
                Some(ControlMsg::Input { event }) => {
                    if let Some(sink) = &input {
                        awake.user_active();
                        sink.send(event);
                    }
                }
                Some(ControlMsg::SelectDisplay { id }) if handle.as_ref().is_some_and(|h| h.display_id != id) => {
                    let Some(handle) = handle.as_mut() else { continue };
                    match switch_display(&config, &video, handle, id).await {
                        Ok(()) => {
                            handle.set_bitrate(rate.bitrate());
                            handle.set_high_resolution(high_resolution);
                            if let Some(sink) = &input {
                                sink.set_area(area(handle.bounds));
                            }
                            send(&channel, &ControlMsg::StreamStarted {
                                width: handle.width,
                                height: handle.height,
                            })
                            .await;
                            send_displays(&channel, &config, handle.display_id).await;
                            on_event(HostEvent::Started { width: handle.width, height: handle.height });
                        }
                        Err(e) => tracing::warn!("cannot switch to display {id}: {e:#}"),
                    }
                }
                Some(ControlMsg::Bye { reason }) => break reason,
                Some(_) => {}
                None => break "connessione chiusa".to_owned(),
            },
            cmd = commands.recv() => match cmd {
                Some(HostCommand::CancelTransfer) => {
                    clipboard.cancel_transfer(&channel, true).await;
                    on_event(HostEvent::Clipboard("trasferimento annullato".into()));
                }
                Some(HostCommand::Chat(text)) => {
                    send(&channel, &ControlMsg::Chat { text }).await;
                }
                Some(HostCommand::PartnerLeft) => break "il partner si è disconnesso".to_owned(),
                Some(HostCommand::Stop) | None => {
                    send_bye(&channel, "terminata dall'host").await;
                    break "terminata da te".to_owned();
                }
            },
        }
    };
    usage_task.abort();
    // Dropping the sink releases any key or button still held by the viewer.
    stop_in_background(handle, input);
    on_event(HostEvent::Ended(reason));
}

/// Tells the viewer, every 2 s, how much CPU and memory sharing costs on this machine.
async fn report_usage(channel: Arc<dyn DataChannel>) {
    let sampler = Arc::new(std::sync::Mutex::new(None::<crate::usage::UsageSampler>));
    let mut tick = tokio::time::interval(USAGE_INTERVAL);
    loop {
        tick.tick().await;
        let sampler = sampler.clone();
        let usage = tokio::task::spawn_blocking(move || {
            let mut sampler = sampler.lock().unwrap();
            sampler
                .get_or_insert_with(crate::usage::UsageSampler::new)
                .sample()
        })
        .await;
        if let Ok(Some(usage)) = usage {
            let msg = ControlMsg::Usage {
                cpu: usage.cpu_percent,
                ram_mb: (usage.memory_bytes / 1_000_000) as u32,
            };
            send(&channel, &msg).await;
        }
    }
}

fn area(r: capture::Rect) -> input::Area {
    input::Area {
        x: r.x,
        y: r.y,
        width: r.width,
        height: r.height,
    }
}

/// Tells the viewer which monitors exist and which one it is seeing.
async fn send_displays(channel: &Arc<dyn DataChannel>, config: &StreamConfig, current: u32) {
    let displays = match config.source {
        crate::Source::Screen { .. } => tokio::task::spawn_blocking(capture::displays)
            .await
            .ok()
            .and_then(Result::ok)
            .unwrap_or_default(),
        crate::Source::TestPattern { width, height } => vec![capture::Display {
            id: 0,
            name: "Immagine di prova".into(),
            width,
            height,
            scale: 1.0,
            primary: true,
            bounds: capture::Rect::default(),
        }],
    };
    // More than one monitor: also offer all of them at once.
    let all = (displays.len() > 1).then(|| {
        let (_, (width, height), _) = capture::composite::layout(&displays, None);
        protocol::DisplayInfo {
            id: capture::ALL_DISPLAYS,
            name: "Tutti gli schermi".into(),
            width,
            height,
            primary: false,
        }
    });
    let displays = displays
        .into_iter()
        .map(|d| protocol::DisplayInfo {
            id: d.id,
            name: d.name,
            width: d.width,
            height: d.height,
            primary: d.primary,
        })
        .chain(all)
        .collect();
    send(channel, &ControlMsg::Displays { displays, current }).await;
}

/// Restarts capture on another monitor. The old stream stops first so the two encoders
/// never interleave frames on the track; on failure the previous monitor is restored.
async fn switch_display(
    config: &StreamConfig,
    video: &Arc<VideoSender>,
    handle: &mut StreamHandle,
    id: u32,
) -> anyhow::Result<()> {
    let crate::Source::Screen { .. } = config.source else {
        anyhow::bail!("not a screen source");
    };
    let previous = handle.display_id;
    let with_display = |display_id| StreamConfig {
        source: crate::Source::Screen {
            display_id: Some(display_id),
        },
        ..config.clone()
    };
    stop_and_wait(handle).await;
    match crate::start(with_display(id), video.clone()).await {
        Ok(new) => {
            *handle = new;
            Ok(())
        }
        Err(e) => {
            *handle = crate::start(with_display(previous), video.clone()).await?;
            Err(e)
        }
    }
}

/// Starts the stream again (sizes and placement read afresh): on the same display, or on
/// the main one if that display is gone (macOS swaps displays when leaving the login window).
async fn restart_stream(
    config: &StreamConfig,
    video: &Arc<VideoSender>,
    handle: &mut StreamHandle,
) -> anyhow::Result<()> {
    let with_display = |display_id| StreamConfig {
        source: crate::Source::Screen { display_id },
        ..config.clone()
    };
    let id = handle.display_id;
    stop_and_wait(handle).await;
    let crate::Source::Screen { .. } = config.source else {
        *handle = crate::start(config.clone(), video.clone()).await?;
        return Ok(());
    };
    *handle = match crate::start(with_display(Some(id)), video.clone()).await {
        Ok(new) => new,
        Err(e) => {
            tracing::info!("display {id} unavailable ({e:#}), using the main one");
            crate::start(with_display(None), video.clone()).await?
        }
    };
    Ok(())
}

async fn stop_and_wait(handle: &mut StreamHandle) {
    let stop = handle.stopper();
    let _ = tokio::task::spawn_blocking(stop).await;
}

/// Joining the encoder thread can take up to one capture timeout; keep it off the runtime.
fn stop_in_background(handle: Option<StreamHandle>, input: Option<HostInput>) {
    tokio::task::spawn_blocking(move || {
        drop(input);
        if let Some(mut handle) = handle {
            handle.stop();
        }
    });
}

/// What the viewer reads while this computer has no screen to share.
fn screen_unavailable_message(e: &anyhow::Error) -> String {
    let no_display = e.chain().any(|cause| {
        matches!(
            cause.downcast_ref::<capture::CaptureError>(),
            Some(capture::CaptureError::NoDisplay)
        )
    });
    if no_display {
        "nessuno schermo collegato al computer remoto: la sessione resta aperta per chat e file, \
         lo schermo compare appena ne viene collegato uno"
            .into()
    } else {
        format!(
            "schermo del partner non disponibile ({e:#}): la sessione resta aperta per chat e file"
        )
    }
}

/// The virtual monitor of a Mac without one, kept for the session; none elsewhere.
#[cfg(target_os = "macos")]
type VirtualScreen = Arc<capture::virtual_display::VirtualDisplay>;
#[cfg(not(target_os = "macos"))]
enum VirtualScreen {}

/// macOS: with no monitor at all, makes a virtual one (kept in `keeper`) so there is
/// something to share. Elsewhere, nothing (a missing monitor leaves the session waiting).
#[cfg(target_os = "macos")]
async fn ensure_screen(config: &StreamConfig, keeper: &mut Option<VirtualScreen>) {
    if keeper.is_some() || !matches!(config.source, crate::Source::Screen { .. }) {
        return;
    }
    let made = tokio::task::spawn_blocking(|| {
        let none = capture::virtual_display::no_monitor()
            || match capture::displays() {
                Ok(displays) => displays.is_empty(),
                Err(e) => matches!(e, capture::CaptureError::NoDisplay),
            };
        none.then(capture::virtual_display::shared)
    })
    .await;
    match made {
        Ok(Some(Ok(screen))) => {
            tracing::info!(id = screen.id(), "no monitor: sharing a virtual one");
            *keeper = Some(screen);
        }
        Ok(Some(Err(e))) => tracing::warn!("no monitor and no virtual one: {e}"),
        Ok(None) | Err(_) => {}
    }
}

#[cfg(not(target_os = "macos"))]
async fn ensure_screen(_config: &StreamConfig, _keeper: &mut Option<VirtualScreen>) {}

#[derive(Debug, Clone)]
pub enum ViewerEvent {
    Started {
        width: u32,
        height: u32,
    },
    Frame(VideoFrame),
    Rtt(Duration),
    /// The host adapted the stream quality to the available bandwidth.
    Quality {
        kbps: u32,
        fps: u32,
        limited: bool,
    },
    /// Clipboard sharing news for the user (files received, sent, refused).
    Clipboard(String),
    /// Clipboard sharing is now on or off (off also when this machine's clipboard is unusable).
    ClipboardEnabled(bool),
    /// A chat message from the host.
    Chat(String),
    /// CPU and memory the partner's app uses.
    PartnerUsage {
        cpu: f32,
        ram_mb: u32,
    },
    /// A keyframe was requested to resync the picture (loss or decoder trouble).
    Resync,
    Error(String),
    /// The partner has no screen to share yet (session stays open; `Started` follows).
    ScreenUnavailable(String),
    /// The host can be watched but not controlled.
    ControlUnavailable(String),
    /// The host's monitors and the one being shown.
    Displays {
        displays: Vec<protocol::DisplayInfo>,
        current: u32,
    },
    /// The host's sound cannot be played (here or on the host); the button goes off.
    AudioUnavailable(String),
    /// A clipboard file transfer moved on.
    Transfer(crate::files::TransferProgress),
    /// Whether the host's own mouse and keyboard are blocked, and why not if it failed.
    InputBlocked {
        blocked: bool,
        message: Option<String>,
    },
    Ended(String),
}

pub enum ViewerCommand {
    /// The decoder lost sync (e.g. decode error in the UI).
    RequestKeyframe,
    /// A chat message typed on the viewer, to show on the host.
    Chat(String),
    /// Share clipboards with the host (on by default).
    Clipboard(bool),
    Input(protocol::InputEvent),
    SelectDisplay(u32),
    /// HD on (up to the host's full size) or off (at most 1080p).
    HighResolution(bool),
    /// Play the host's sound or not (off by default).
    Audio(bool),
    /// Volume of the host's sound here, 0.0–1.0.
    Volume(f32),
    /// Stop the clipboard file transfer under way.
    CancelTransfer,
    /// Block the host's own mouse and keyboard or not.
    BlockInput(bool),
    Stop,
}

/// Viewer: receives the host's screen until either side ends the session.
pub async fn run_viewer(
    channel: Arc<dyn DataChannel>,
    files: tokio::sync::oneshot::Receiver<Arc<dyn DataChannel>>,
    audio: tokio::sync::oneshot::Receiver<Arc<dyn DataChannel>>,
    mut tracks: mpsc::UnboundedReceiver<Arc<dyn TrackRemote>>,
    mut commands: mpsc::UnboundedReceiver<ViewerCommand>,
    share_clipboard: bool,
    mut on_event: impl FnMut(ViewerEvent),
) {
    let (frames_tx, mut frames) = mpsc::unbounded_channel::<Assembled>();
    let packets = Arc::new(SharedPacketStats::default());
    let video_task = tokio::spawn({
        let packets = packets.clone();
        async move {
            let Some(track) = tracks.recv().await else {
                return;
            };
            let mut receiver = VideoReceiver::with_stats(track, packets);
            while let Some(event) = receiver.next().await {
                if frames_tx.send(event).is_err() {
                    break;
                }
            }
        }
    });

    // Sound: off until the user turns it on.
    let mut sound = crate::sound::ViewerSound::new(audio);
    let start = Instant::now();
    let mut ping = tokio::time::interval(PING_INTERVAL);
    let mut seq = 0u32;
    let mut last_keyframe_request: Option<Instant> = None;
    let mut want_keyframe = false;
    // Packet counters at the previous report, and the latest RTT, for adaptive quality.
    let mut reported = packets.get();
    let mut last_rtt_ms = 0u32;
    // What the viewer copied before connecting counts too.
    let mut clipboard = ClipboardShare::new(files);
    clipboard.set_enabled(share_clipboard, true).await;
    let enabled = clipboard.enabled();
    send(&channel, &ControlMsg::ClipboardMode { enabled }).await;
    on_event(ViewerEvent::ClipboardEnabled(enabled));

    let mut last_heard = Instant::now();
    let mut liveness = tokio::time::interval(LIVENESS_CHECK);
    let reason = loop {
        if want_keyframe
            && last_keyframe_request.is_none_or(|t| t.elapsed() >= MIN_KEYFRAME_REQUEST_GAP)
        {
            send(&channel, &ControlMsg::RequestKeyframe).await;
            last_keyframe_request = Some(Instant::now());
            want_keyframe = false;
            on_event(ViewerEvent::Resync);
        }
        tokio::select! {
            _ = liveness.tick() => {
                if last_heard.elapsed() > PEER_TIMEOUT {
                    tracing::warn!("no message from the host for {:?}, ending", last_heard.elapsed());
                    break "il partner non risponde".to_owned();
                }
            }
            msg = recv(&channel) => match heard(&mut last_heard, msg) {
                Some(ControlMsg::StreamStarted { width, height }) => {
                    on_event(ViewerEvent::Started { width, height });
                }
                Some(ControlMsg::StreamError { message }) => {
                    on_event(ViewerEvent::Error(message.clone()));
                    send_bye(&channel, "condivisione non disponibile").await;
                    break message;
                }
                Some(ControlMsg::Pong { sent_us, .. }) => {
                    let now_us = start.elapsed().as_micros() as u64;
                    let rtt = Duration::from_micros(now_us.saturating_sub(sent_us));
                    last_rtt_ms = (rtt.as_millis() as u32).max(1);
                    on_event(ViewerEvent::Rtt(rtt));
                }
                Some(ControlMsg::Clipboard { text }) => clipboard.remote_text(text),
                Some(ControlMsg::TransferCancelled) => {
                    clipboard.cancel_transfer(&channel, false).await;
                    on_event(ViewerEvent::Clipboard("trasferimento annullato dal partner".into()));
                }
                // The host does not allow clipboard sharing.
                Some(ControlMsg::ClipboardMode { enabled: false }) => {
                    clipboard.set_enabled(false, false).await;
                    on_event(ViewerEvent::ClipboardEnabled(false));
                }
                Some(ControlMsg::Chat { text }) => on_event(ViewerEvent::Chat(text)),
                Some(ControlMsg::InputBlocked { blocked, message }) => {
                    on_event(ViewerEvent::InputBlocked { blocked, message });
                }
                Some(ControlMsg::AudioUnavailable { message }) => {
                    let _ = sound.set_enabled(false).await;
                    on_event(ViewerEvent::AudioUnavailable(message));
                }
                Some(ControlMsg::Usage { cpu, ram_mb }) => {
                    on_event(ViewerEvent::PartnerUsage { cpu, ram_mb })
                }
                Some(ControlMsg::Quality { kbps, fps, limited }) => {
                    on_event(ViewerEvent::Quality { kbps, fps, limited })
                }
                Some(ControlMsg::Displays { displays, current }) => {
                    on_event(ViewerEvent::Displays { displays, current });
                }
                Some(ControlMsg::InputUnavailable { message }) => {
                    on_event(ViewerEvent::ControlUnavailable(message));
                }
                Some(ControlMsg::ScreenUnavailable { message }) => {
                    on_event(ViewerEvent::ScreenUnavailable(message));
                }
                Some(ControlMsg::Bye { reason }) => break reason,
                Some(_) => {}
                None => break "connessione chiusa".to_owned(),
            },
            Some(event) = frames.recv() => match event {
                Assembled::Frame(frame) => on_event(ViewerEvent::Frame(frame)),
                Assembled::Lost => want_keyframe = true,
            },
            cmd = commands.recv() => match cmd {
                Some(ViewerCommand::RequestKeyframe) => want_keyframe = true,
                Some(ViewerCommand::Chat(text)) => send(&channel, &ControlMsg::Chat { text }).await,
                Some(ViewerCommand::Clipboard(enabled)) => {
                    clipboard.set_enabled(enabled, true).await;
                    let enabled = clipboard.enabled();
                    send(&channel, &ControlMsg::ClipboardMode { enabled }).await;
                    on_event(ViewerEvent::ClipboardEnabled(enabled));
                }
                Some(ViewerCommand::Input(event)) => send(&channel, &ControlMsg::Input { event }).await,
                Some(ViewerCommand::SelectDisplay(id)) => send(&channel, &ControlMsg::SelectDisplay { id }).await,
                Some(ViewerCommand::HighResolution(enabled)) => {
                    send(&channel, &ControlMsg::HighResolution { enabled }).await;
                }
                Some(ViewerCommand::BlockInput(enabled)) => {
                    send(&channel, &ControlMsg::BlockInput { enabled }).await;
                }
                Some(ViewerCommand::Volume(volume)) => sound.set_volume(volume),
                Some(ViewerCommand::CancelTransfer) => {
                    clipboard.cancel_transfer(&channel, true).await;
                    on_event(ViewerEvent::Clipboard("trasferimento annullato".into()));
                }
                Some(ViewerCommand::Audio(enabled)) => {
                    // Speakers first, so the first packets are not lost; host last on stop.
                    match sound.set_enabled(enabled).await {
                        Ok(()) => send(&channel, &ControlMsg::Audio { enabled }).await,
                        Err(message) => on_event(ViewerEvent::AudioUnavailable(message)),
                    }
                }
                Some(ViewerCommand::Stop) | None => {
                    send_bye(&channel, "il partner si è disconnesso").await;
                    break "disconnesso".to_owned();
                }
            },
            event = clipboard.next() => match event {
                crate::sharing::ClipEvent::Progress(progress) => {
                    on_event(ViewerEvent::Transfer(progress));
                }
                event => {
                    if let Some(message) = clipboard.handle(event, &channel).await {
                        on_event(ViewerEvent::Clipboard(message));
                    }
                }
            },
            _ = ping.tick() => {
                let now = packets.get();
                send(&channel, &ControlMsg::Report {
                    received: (now.received - reported.received) as u32,
                    lost: (now.lost - reported.lost) as u32,
                    rtt_ms: last_rtt_ms,
                })
                .await;
                reported = now;
                let sent_us = start.elapsed().as_micros() as u64;
                send(&channel, &ControlMsg::Ping { seq, sent_us }).await;
                seq = seq.wrapping_add(1);
            }
            // Re-check pending keyframe requests that were rate limited.
            _ = tokio::time::sleep(MIN_KEYFRAME_REQUEST_GAP), if want_keyframe => {}
        }
    };
    video_task.abort();
    on_event(ViewerEvent::Ended(reason));
}
