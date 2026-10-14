//! Host screen streaming: capture → H.264 → WebRTC video track.
//!
//! Capture and encoding are blocking and CPU-bound, so they run on a dedicated thread;
//! an async task forwards encoded frames to the track.

pub mod awake;
pub mod clipboard;
pub mod files;
pub mod rate;
pub mod session;
mod sharing;
pub mod usage;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use capture::{CaptureConfig, Capturer, Frame, TestPattern};
use codec::{EncoderSettings, H264Encoder};
use tokio::sync::{mpsc, oneshot, watch};
use transport::video::VideoSender;

#[derive(Debug, Clone)]
pub enum Source {
    /// A real display (`None` = main/first display).
    Screen { display_id: Option<u32> },
    /// Synthetic moving image, for tests and machines without capture support.
    TestPattern { width: u32, height: u32 },
}

#[derive(Debug, Clone)]
pub struct StreamConfig {
    pub source: Source,
    /// Output is scaled to fit in this box.
    pub max_size: (u32, u32),
    pub fps: u32,
    pub bitrate_bps: u32,
    /// Periodic keyframe as a safety net (frames); on-demand keyframes come from the viewer.
    pub keyframe_interval: u32,
    /// Replay the viewer's mouse and keyboard on this machine.
    pub allow_control: bool,
    /// Host: let the viewer turn on clipboard sharing (text and files) in this session.
    pub allow_clipboard: bool,
    /// Windows only: route capture and input through the SYSTEM agent of this product
    /// (e.g. "remote"), so UAC prompts and the lock screen work. Ignored when no agent is
    /// running, or off-Windows.
    pub agent_product: Option<String>,
}

impl Default for StreamConfig {
    fn default() -> Self {
        Self {
            source: Source::Screen { display_id: None },
            max_size: (1920, 1080),
            fps: 30,
            bitrate_bps: 4_000_000,
            keyframe_interval: 300,
            allow_control: true,
            allow_clipboard: true,
            agent_product: None,
        }
    }
}

/// What [`StreamHandle::changed`] reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamChange {
    Resized {
        width: u32,
        height: u32,
    },
    /// The encoder thread exited: no more frames until the stream is started again.
    Ended,
}

/// Running stream. Dropping it stops capture and encoding.
pub struct StreamHandle {
    /// Encoded size: the captured size, or less while the bandwidth is low.
    pub width: u32,
    pub height: u32,
    /// Display actually captured (resolved when the config asked for "primary").
    pub display_id: u32,
    /// Captured area in input coordinates.
    pub bounds: capture::Rect,
    stop: Arc<AtomicBool>,
    keyframe: Arc<AtomicBool>,
    bitrate: Arc<AtomicU32>,
    size: watch::Receiver<(u32, u32)>,
    thread: Option<JoinHandle<()>>,
}

impl StreamHandle {
    /// Waits until the encoded size changes (resolution lowered or restored for the
    /// bandwidth) or the stream stops by itself (capture lost, e.g. the screen changed size).
    pub async fn changed(&mut self) -> StreamChange {
        loop {
            if self.size.changed().await.is_err() {
                return StreamChange::Ended;
            }
            let (width, height) = *self.size.borrow_and_update();
            if (width, height) != (self.width, self.height) {
                self.width = width;
                self.height = height;
                return StreamChange::Resized { width, height };
            }
        }
    }

    /// Adaptive quality: target bitrate from now on (capped at the configured one); the
    /// frame rate follows ([`rate::fps_for`]).
    pub fn set_bitrate(&self, bps: u32) {
        self.bitrate.store(bps, Ordering::Relaxed);
    }

    /// The next frame sent will be an IDR (viewer joined, lost packets or decoder error).
    pub fn request_keyframe(&self) {
        self.keyframe.store(true, Ordering::Relaxed);
    }

    pub fn stop(&mut self) {
        self.stopper()();
    }

    /// Detaches a blocking "stop and join" so it can run off the async runtime.
    pub fn stopper(&mut self) -> impl FnOnce() + Send + 'static {
        self.stop.store(true, Ordering::Relaxed);
        let thread = self.thread.take();
        move || {
            if let Some(thread) = thread {
                let _ = thread.join();
            }
        }
    }
}

impl Drop for StreamHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Minimum spacing between forced keyframes.
const MIN_KEYFRAME_GAP: Duration = Duration::from_millis(300);

struct SourceInfo {
    width: u32,
    height: u32,
    display_id: u32,
    bounds: capture::Rect,
}

struct Encoded {
    data: Vec<u8>,
    captured_at: Instant,
}

/// The agent product to use for this stream, if one is configured and actually running.
#[cfg(windows)]
fn agent_product_if_available(config: &StreamConfig) -> Option<String> {
    let product = config.agent_product.as_deref()?;
    agent::windows::agent_available(product).then(|| product.to_owned())
}

fn open_source(config: &StreamConfig) -> Result<Box<dyn Capturer>> {
    match config.source {
        // Shared: several sessions may watch the same monitor (Windows allows only one
        // duplication per output and process).
        Source::Screen { display_id } => {
            let cfg = CaptureConfig {
                display_id,
                max_fps: config.fps,
                // All monitors side by side: allow up to the encoder limit so each stays legible.
                max_size: Some(if display_id == Some(capture::ALL_DISPLAYS) {
                    (codec::MAX_WIDTH, codec::MAX_HEIGHT)
                } else {
                    config.max_size
                }),
                show_cursor: true,
            };
            #[cfg(windows)]
            if let Some(product) = agent_product_if_available(config) {
                return agent::client::AgentCapturer::start(
                    move || async move { agent::windows::connect(&product).await },
                    &cfg,
                )
                .map(|c| Box::new(c) as Box<dyn Capturer>)
                .map_err(|e| anyhow!(e));
            }
            capture::shared::subscribe(&cfg).map_err(|e| anyhow!(e))
        }
        Source::TestPattern { width, height } => {
            Ok(Box::new(TestPattern::new(width, height, config.fps)))
        }
    }
}

/// Starts streaming to `video`. Fails early if the source cannot be opened
/// (e.g. screen recording permission denied, unsupported platform).
pub async fn start(config: StreamConfig, video: Arc<VideoSender>) -> Result<StreamHandle> {
    let stop = Arc::new(AtomicBool::new(false));
    let keyframe = Arc::new(AtomicBool::new(true));
    let bitrate = Arc::new(AtomicU32::new(config.bitrate_bps));
    let (ready_tx, ready_rx) = oneshot::channel::<Result<SourceInfo>>();
    // Small queue: if the network side falls behind, drop and resync with a keyframe
    // rather than building latency.
    let (frames_tx, mut frames_rx) = mpsc::channel::<Encoded>(4);
    let (size_tx, size_rx) = watch::channel((0, 0));

    let thread = {
        let controls = Controls {
            stop: stop.clone(),
            keyframe: keyframe.clone(),
            bitrate: bitrate.clone(),
            size: size_tx,
        };
        std::thread::Builder::new()
            .name("screen-encoder".into())
            .spawn(move || encode_loop(config, controls, ready_tx, frames_tx))
            .context("spawning encoder thread")?
    };

    let info = ready_rx
        .await
        .context("encoder thread exited during startup")??;

    tokio::spawn(async move {
        let mut last: Option<Instant> = None;
        while let Some(frame) = frames_rx.recv().await {
            let duration = last.map_or(Duration::from_millis(33), |l| {
                frame.captured_at.saturating_duration_since(l)
            });
            last = Some(frame.captured_at);
            if let Err(e) = video.write_frame(frame.data, duration).await {
                tracing::debug!("video write failed (session closing?): {e:#}");
                break;
            }
        }
    });

    Ok(StreamHandle {
        width: info.width,
        height: info.height,
        display_id: info.display_id,
        bounds: info.bounds,
        stop,
        keyframe,
        bitrate,
        size: size_rx,
        thread: Some(thread),
    })
}

/// Shared with [`StreamHandle`]: how the async side steers the encoder thread.
struct Controls {
    stop: Arc<AtomicBool>,
    keyframe: Arc<AtomicBool>,
    bitrate: Arc<AtomicU32>,
    size: watch::Sender<(u32, u32)>,
}

fn encode_loop(
    config: StreamConfig,
    controls: Controls,
    ready: oneshot::Sender<Result<SourceInfo>>,
    frames: mpsc::Sender<Encoded>,
) {
    let Controls {
        stop,
        keyframe,
        bitrate,
        size,
    } = controls;
    let new_encoder = |(width, height): (u32, u32)| {
        H264Encoder::new(&EncoderSettings {
            fps: config.fps,
            bitrate_bps: config.bitrate_bps,
            keyframe_interval: config.keyframe_interval,
            ..EncoderSettings::new(width, height)
        })
    };
    let setup = open_source(&config).and_then(|source| {
        let encoder = new_encoder(source.output_size())?;
        Ok((source, encoder))
    });
    let (mut source, mut encoder) = match setup {
        Ok(parts) => {
            let (width, height) = parts.0.output_size();
            let _ = ready.send(Ok(SourceInfo {
                width,
                height,
                display_id: parts.0.display_id(),
                bounds: parts.0.bounds(),
            }));
            parts
        }
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };

    let captured_size = source.output_size();
    size.send_replace(captured_size);
    // Resolution in percent of the captured size, lowered while the bandwidth is low.
    let mut scale = 100;
    let mut last_rescale: Option<Instant> = None;
    let start = Instant::now();
    let mut last_keyframe: Option<Instant> = None;
    // Kept so a keyframe can be produced even while the screen is static.
    let mut last_frame: Option<Frame> = None;
    // Newest captured frame not encoded yet (frame rate lowered by adaptive quality).
    let mut pending: Option<Frame> = None;
    let mut last_encoded: Option<Instant> = None;
    let mut applied_bitrate = config.bitrate_bps;
    let mut interval = Duration::from_secs(1) / config.fps.max(1);

    while !stop.load(Ordering::Relaxed) {
        let target = bitrate.load(Ordering::Relaxed);
        if target != applied_bitrate {
            applied_bitrate = target;
            let fps = rate::fps_for(target, config.fps);
            encoder.set_rate(target, fps);
            interval = Duration::from_secs(1) / fps;
        }
        let wanted = rate::scale_for(applied_bitrate, scale);
        let gap = if wanted < scale {
            rate::SCALE_DOWN_GAP
        } else {
            rate::SCALE_UP_GAP
        };
        if wanted != scale && last_rescale.is_none_or(|t| t.elapsed() >= gap) {
            let encoded_size = rate::scaled_size(captured_size, wanted);
            match new_encoder(encoded_size) {
                Ok(mut resized) => {
                    tracing::info!(scale = wanted, size = ?encoded_size, "resolution changed");
                    let fps = rate::fps_for(applied_bitrate, config.fps);
                    resized.set_rate(applied_bitrate, fps);
                    // A new encoder starts with a keyframe carrying the new size.
                    encoder = resized;
                    scale = wanted;
                    last_rescale = Some(Instant::now());
                    last_keyframe = Some(Instant::now());
                    keyframe.store(false, Ordering::Relaxed);
                    size.send_replace(encoded_size);
                    // Encode the current picture at once, even if the screen is static.
                    if pending.is_none()
                        && let Some(previous) = &last_frame
                    {
                        pending = Some(Frame {
                            captured_at: Instant::now(),
                            ..previous.clone()
                        });
                    }
                }
                Err(e) => {
                    tracing::warn!("cannot resize the encoder: {e:#}");
                    last_rescale = Some(Instant::now());
                }
            }
        }
        // Wait for a new frame, or only until the pending one is due.
        let wait = match (&pending, last_encoded) {
            (Some(_), Some(t)) => interval.saturating_sub(t.elapsed()),
            _ => Duration::from_millis(100),
        };
        match source.next_frame(wait) {
            Ok(Some(frame)) => pending = Some(frame),
            Ok(None) => {}
            Err(e) => {
                tracing::warn!("capture stopped: {e}");
                break;
            }
        }
        // A little slack so capture jitter does not halve the frame rate.
        if last_encoded.is_some_and(|t| t.elapsed() + Duration::from_millis(3) < interval) {
            continue;
        }
        // Honour keyframe requests at most every MIN_KEYFRAME_GAP: IDRs are large, and a
        // burst of them on a lossy link only makes things worse.
        let want_keyframe = last_keyframe.is_none_or(|t| t.elapsed() >= MIN_KEYFRAME_GAP)
            && keyframe.swap(false, Ordering::Relaxed);
        let frame = match pending.take() {
            Some(frame) => frame,
            None if want_keyframe => match &last_frame {
                Some(previous) => Frame {
                    captured_at: Instant::now(),
                    ..previous.clone()
                },
                None => {
                    keyframe.store(true, Ordering::Relaxed);
                    continue;
                }
            },
            None => continue,
        };
        if (frame.width, frame.height) != captured_size {
            tracing::warn!(size = ?(frame.width, frame.height), "frame size changed, skipping");
            continue;
        }
        let (width, height) = encoder.size();
        let scaled = (scale < 100).then(|| {
            capture::scale::scale_bgra(&frame.data, frame.width, frame.height, width, height)
        });
        let data = scaled.as_deref().unwrap_or(&frame.data);

        if want_keyframe {
            encoder.force_keyframe();
            last_keyframe = Some(Instant::now());
        }
        last_encoded = Some(Instant::now());
        let ts = frame.captured_at.duration_since(start).as_millis() as u64;
        match encoder.encode_bgra(data, width, height, ts) {
            Ok(Some(encoded)) => {
                let item = Encoded {
                    data: encoded.data,
                    captured_at: frame.captured_at,
                };
                if frames.try_send(item).is_err() {
                    if frames.is_closed() {
                        break;
                    }
                    // Dropped a frame: later deltas would reference it, so resync.
                    keyframe.store(true, Ordering::Relaxed);
                }
            }
            Ok(None) => {}
            Err(e) => {
                tracing::warn!("encode failed: {e}");
                break;
            }
        }
        last_frame = Some(frame);
    }
}
