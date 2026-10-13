//! Agent side of a connection: runs capture and input on behalf of the app.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use capture::{CaptureConfig, Capturer};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::mpsc;

use crate::{PROTOCOL_VERSION, Reply, Request, Wire, read_message, write_message};

/// Something that replays input (the real one is [`input::InputSink`]).
pub trait Injector: Send {
    fn send(&self, event: protocol::InputEvent);
    fn set_area(&self, area: input::Area);
}

impl Injector for input::InputSink {
    fn send(&self, event: protocol::InputEvent) {
        input::InputSink::send(self, event);
    }
    fn set_area(&self, area: input::Area) {
        input::InputSink::set_area(self, area);
    }
}

/// How the agent captures and injects (swappable in tests).
pub trait Backend: Send + Sync + 'static {
    /// Called on the capture thread, which then keeps calling `next_frame` (on Windows the
    /// thread must stay on the desktop the duplication was opened for).
    fn start_capture(&self, config: &CaptureConfig) -> capture::Result<Box<dyn Capturer>>;
    fn start_input(&self) -> anyhow::Result<Box<dyn Injector>>;
}

/// Real capture and input of this machine.
pub struct System;

impl Backend for System {
    fn start_capture(&self, config: &CaptureConfig) -> capture::Result<Box<dyn Capturer>> {
        capture::shared::subscribe(config)
    }
    fn start_input(&self) -> anyhow::Result<Box<dyn Injector>> {
        Ok(Box::new(input::InputSink::start()?))
    }
}

/// Serves one app connection until it closes.
pub async fn serve<S>(stream: S, backend: Arc<dyn Backend>)
where
    S: AsyncRead + AsyncWrite + Send + 'static,
{
    let (mut reader, mut writer) = tokio::io::split(stream);
    // Few frames in flight: a slow app gets fresh frames, not a growing backlog.
    let (out_tx, mut out_rx) = mpsc::channel::<Wire<Reply>>(3);
    let writer_task = tokio::spawn(async move {
        while let Some(message) = out_rx.recv().await {
            if write_message(&mut writer, &message).await.is_err() {
                break;
            }
        }
    });

    let mut capture_stop: Option<Arc<AtomicBool>> = None;
    let mut injector: Option<Box<dyn Injector>> = None;
    loop {
        let request = match read_message::<_, Request>(&mut reader).await {
            Ok(Some(Wire::Json(request))) => request,
            Ok(Some(Wire::Frame { .. })) => continue,
            Ok(None) => break,
            Err(e) => {
                tracing::debug!("agent connection: {e}");
                break;
            }
        };
        match request {
            Request::Hello { version } => {
                let reply = if version == PROTOCOL_VERSION {
                    Reply::Hello {
                        version: PROTOCOL_VERSION,
                    }
                } else {
                    Reply::Error {
                        message: format!("protocol {version} not supported"),
                    }
                };
                let _ = out_tx.send(Wire::Json(reply)).await;
            }
            Request::StartCapture {
                display_id,
                max_size,
                max_fps,
                show_cursor,
            } => {
                if let Some(stop) = capture_stop.take() {
                    stop.store(true, Ordering::Relaxed);
                }
                let stop = Arc::new(AtomicBool::new(false));
                capture_stop = Some(stop.clone());
                let config = CaptureConfig {
                    display_id,
                    max_fps,
                    max_size,
                    show_cursor,
                };
                spawn_capture(backend.clone(), config, stop, out_tx.clone());
            }
            Request::StartInput => {
                let reply = match backend.start_input() {
                    Ok(sink) => {
                        injector = Some(sink);
                        Reply::InputStarted
                    }
                    Err(e) => Reply::Error {
                        message: format!("{e:#}"),
                    },
                };
                let _ = out_tx.send(Wire::Json(reply)).await;
            }
            Request::Input { event } => {
                if let Some(sink) = &injector {
                    sink.send(event);
                }
            }
            Request::SetArea {
                x,
                y,
                width,
                height,
            } => {
                if let Some(sink) = &injector {
                    sink.set_area(input::Area {
                        x,
                        y,
                        width,
                        height,
                    });
                }
            }
        }
    }
    if let Some(stop) = capture_stop {
        stop.store(true, Ordering::Relaxed);
    }
    // Dropping the injector releases any key or button still held.
    drop(injector);
    drop(out_tx);
    let _ = writer_task.await;
}

fn spawn_capture(
    backend: Arc<dyn Backend>,
    config: CaptureConfig,
    stop: Arc<AtomicBool>,
    out: mpsc::Sender<Wire<Reply>>,
) {
    let spawned = std::thread::Builder::new()
        .name("agent-capture".into())
        .spawn(move || {
            let mut capturer = match backend.start_capture(&config) {
                Ok(c) => c,
                Err(e) => {
                    let _ = out.blocking_send(Wire::Json(Reply::Error {
                        message: e.to_string(),
                    }));
                    return;
                }
            };
            let (width, height) = capturer.output_size();
            let b = capturer.bounds();
            let started = Reply::CaptureStarted {
                width,
                height,
                display_id: capturer.display_id(),
                bounds: (b.x, b.y, b.width, b.height),
            };
            if out.blocking_send(Wire::Json(started)).is_err() {
                return;
            }
            while !stop.load(Ordering::Relaxed) {
                match capturer.next_frame(Duration::from_millis(100)) {
                    Ok(Some(frame)) => {
                        let message = Wire::Frame {
                            width: frame.width,
                            height: frame.height,
                            data: frame.data,
                        };
                        if out.blocking_send(message).is_err() {
                            break;
                        }
                    }
                    Ok(None) => {}
                    Err(e) => {
                        let _ = out.blocking_send(Wire::Json(Reply::Error {
                            message: e.to_string(),
                        }));
                        break;
                    }
                }
            }
        });
    if let Err(e) = spawned {
        tracing::warn!("cannot start the agent capture thread: {e}");
    }
}
