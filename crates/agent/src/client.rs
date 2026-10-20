//! App side: a [`Capturer`] and an input injector backed by the agent.
//!
//! Each runs its connection on a small dedicated thread (single-threaded tokio runtime), so
//! it can be used from the synchronous encoder thread and from anywhere in the app.

use std::future::Future;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, mpsc as std_mpsc};
use std::time::{Duration, Instant};

use capture::{CaptureConfig, CaptureError, Capturer, Frame, Rect};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{mpsc, oneshot};

use crate::{PROTOCOL_VERSION, Reply, Request, Wire, read_message, write_message};

/// How long the agent may take to start a capture (some displays take seconds).
const START_TIMEOUT: Duration = Duration::from_secs(20);

fn runtime() -> std::io::Result<tokio::runtime::Runtime> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
}

struct Started {
    size: (u32, u32),
    display_id: u32,
    bounds: Rect,
}

pub struct AgentCapturer {
    started: Started,
    frames: std_mpsc::Receiver<Result<Frame, String>>,
    stop: Option<oneshot::Sender<()>>,
}

impl AgentCapturer {
    /// Asks the agent reached through `connect` to capture with `config`.
    pub fn start<C, F, S>(connect: C, config: &CaptureConfig) -> capture::Result<Self>
    where
        C: FnOnce() -> F + Send + 'static,
        F: Future<Output = std::io::Result<S>>,
        S: AsyncRead + AsyncWrite + Send + 'static,
    {
        let request = Request::StartCapture {
            display_id: config.display_id,
            max_size: config.max_size,
            max_fps: config.max_fps,
            show_cursor: config.show_cursor,
        };
        let (started_tx, started_rx) = std_mpsc::channel::<Result<Started, String>>();
        let (frames_tx, frames) = std_mpsc::sync_channel::<Result<Frame, String>>(2);
        let (stop_tx, mut stop_rx) = oneshot::channel::<()>();
        let backend = |e: std::io::Error| CaptureError::Backend(format!("agent: {e}"));
        let rt = runtime().map_err(backend)?;
        std::thread::Builder::new()
            .name("agent-capture-client".into())
            .spawn(move || {
                rt.block_on(async move {
                    let fail = |message: String| {
                        let _ = started_tx.send(Err(message));
                    };
                    let stream = match connect().await {
                        Ok(s) => s,
                        Err(e) => return fail(format!("cannot reach the agent: {e}")),
                    };
                    let (mut reader, mut writer) = tokio::io::split(stream);
                    let hello = Wire::Json(Request::Hello {
                        version: PROTOCOL_VERSION,
                    });
                    if let Err(e) = write_message(&mut writer, &hello).await {
                        return fail(e.to_string());
                    }
                    if let Err(e) = write_message(&mut writer, &Wire::Json(request)).await {
                        return fail(e.to_string());
                    }
                    let mut started = Some(started_tx);
                    loop {
                        let message = tokio::select! {
                            _ = &mut stop_rx => break,
                            m = read_message::<_, Reply>(&mut reader) => m,
                        };
                        let report = |message: String| match &started {
                            Some(tx) => {
                                let _ = tx.send(Err(message));
                            }
                            None => {
                                let _ = frames_tx.send(Err(message));
                            }
                        };
                        match message {
                            Ok(Some(Wire::Json(Reply::CaptureStarted {
                                width,
                                height,
                                display_id,
                                bounds: (x, y, w, h),
                            }))) => {
                                if let Some(tx) = started.take() {
                                    let _ = tx.send(Ok(Started {
                                        size: (width, height),
                                        display_id,
                                        bounds: Rect {
                                            x,
                                            y,
                                            width: w,
                                            height: h,
                                        },
                                    }));
                                }
                            }
                            Ok(Some(Wire::Json(Reply::Error { message }))) => {
                                report(message);
                                break;
                            }
                            Ok(Some(Wire::Json(_))) => {}
                            Ok(Some(Wire::Frame {
                                width,
                                height,
                                data,
                            })) => {
                                let frame = Frame {
                                    width,
                                    height,
                                    data,
                                    captured_at: Instant::now(),
                                    native: None,
                                };
                                // Blocks while the encoder is busy: the agent then waits too.
                                if frames_tx.send(Ok(frame)).is_err() {
                                    break;
                                }
                            }
                            Ok(None) => {
                                report("the agent closed the connection".into());
                                break;
                            }
                            Err(e) => {
                                report(format!("agent connection: {e}"));
                                break;
                            }
                        }
                    }
                });
            })
            .map_err(backend)?;
        let started = match started_rx.recv_timeout(START_TIMEOUT) {
            Ok(Ok(started)) => started,
            Ok(Err(message)) => return Err(CaptureError::Backend(message)),
            Err(_) => return Err(CaptureError::Backend("the agent did not answer".into())),
        };
        Ok(Self {
            started,
            frames,
            stop: Some(stop_tx),
        })
    }
}

impl Capturer for AgentCapturer {
    fn output_size(&self) -> (u32, u32) {
        self.started.size
    }

    fn display_id(&self) -> u32 {
        self.started.display_id
    }

    fn bounds(&self) -> Rect {
        self.started.bounds
    }

    fn next_frame(&mut self, timeout: Duration) -> capture::Result<Option<Frame>> {
        match self.frames.recv_timeout(timeout) {
            Ok(Ok(frame)) => Ok(Some(frame)),
            Ok(Err(message)) => Err(CaptureError::Backend(message)),
            Err(std_mpsc::RecvTimeoutError::Timeout) => Ok(None),
            Err(std_mpsc::RecvTimeoutError::Disconnected) => Err(CaptureError::Stopped),
        }
    }
}

impl Drop for AgentCapturer {
    fn drop(&mut self) {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
    }
}

/// Mouse and keyboard replayed by the agent.
pub struct AgentInput {
    requests: mpsc::UnboundedSender<Request>,
    sent: Arc<AtomicU64>,
}

impl AgentInput {
    pub fn start<C, F, S>(connect: C) -> anyhow::Result<Self>
    where
        C: FnOnce() -> F + Send + 'static,
        F: Future<Output = std::io::Result<S>>,
        S: AsyncRead + AsyncWrite + Send + 'static,
    {
        let (requests, mut rx) = mpsc::unbounded_channel::<Request>();
        let (ready_tx, ready_rx) = std_mpsc::channel::<Result<(), String>>();
        let rt = runtime()?;
        std::thread::Builder::new()
            .name("agent-input-client".into())
            .spawn(move || {
                rt.block_on(async move {
                    let stream = match connect().await {
                        Ok(s) => s,
                        Err(e) => {
                            let _ = ready_tx.send(Err(format!("cannot reach the agent: {e}")));
                            return;
                        }
                    };
                    let (mut reader, mut writer) = tokio::io::split(stream);
                    for request in [
                        Request::Hello {
                            version: PROTOCOL_VERSION,
                        },
                        Request::StartInput,
                    ] {
                        if let Err(e) = write_message(&mut writer, &Wire::Json(request)).await {
                            let _ = ready_tx.send(Err(e.to_string()));
                            return;
                        }
                    }
                    loop {
                        match read_message::<_, Reply>(&mut reader).await {
                            Ok(Some(Wire::Json(Reply::InputStarted))) => break,
                            Ok(Some(Wire::Json(Reply::Error { message }))) => {
                                let _ = ready_tx.send(Err(message));
                                return;
                            }
                            Ok(Some(_)) => {}
                            Ok(None) | Err(_) => {
                                let _ =
                                    ready_tx.send(Err("the agent closed the connection".into()));
                                return;
                            }
                        }
                    }
                    let _ = ready_tx.send(Ok(()));
                    while let Some(request) = rx.recv().await {
                        if write_message(&mut writer, &Wire::Json(request))
                            .await
                            .is_err()
                        {
                            break;
                        }
                    }
                });
            })?;
        match ready_rx.recv_timeout(START_TIMEOUT) {
            Ok(Ok(())) => Ok(Self {
                requests,
                sent: Arc::default(),
            }),
            Ok(Err(message)) => anyhow::bail!(message),
            Err(_) => anyhow::bail!("the agent did not answer"),
        }
    }

    pub fn send(&self, event: protocol::InputEvent) {
        if self.requests.send(Request::Input { event }).is_ok() {
            self.sent.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn set_area(&self, area: input::Area) {
        let _ = self.requests.send(Request::SetArea {
            x: area.x,
            y: area.y,
            width: area.width,
            height: area.height,
        });
    }

    /// Events handed to the agent (it reports no per-event failures).
    pub fn sent(&self) -> u64 {
        self.sent.load(Ordering::Relaxed)
    }
}
