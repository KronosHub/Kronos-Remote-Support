//! Host screen streaming: capture → H.264 → WebRTC video track.
//!
//! Capture and encoding are blocking and CPU-bound, so they run on a dedicated thread;
//! an async task forwards encoded frames to the track.

pub mod awake;
pub mod clipboard;
pub mod files;
pub mod fs;
pub mod rate;
mod resize;
pub mod session;
mod sharing;
mod sound;
pub mod usage;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow};
use capture::{CaptureConfig, Capturer, Frame, TestPattern};
use codec::{EncoderSettings, VideoEncoder};
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
    /// Encode on the media engine when this build and machine can (ADR 0058); OpenH264
    /// otherwise.
    pub hardware_encoding: bool,
    /// Host: the viewer may set the shared screen to its window size (ADR 0061); only for
    /// computers used remotely, where nobody sees the monitor change.
    pub resize_display: bool,
}

impl Default for StreamConfig {
    fn default() -> Self {
        Self {
            source: Source::Screen { display_id: None },
            // Sharp text on large and Retina screens; slow links and slow machines step
            // down from here (`rate::scale_for`, `HEAVY_ENCODE`).
            max_size: (2560, 1600),
            fps: 30,
            // Cap; the actual limit follows the picture size (`rate::max_bitrate_for`).
            bitrate_bps: 6_000_000,
            // A full picture every 60 s at most: losses ask for one at once anyway, and the
            // bits go to the picture instead.
            keyframe_interval: 1800,
            allow_control: true,
            allow_clipboard: true,
            agent_product: None,
            hardware_encoding: true,
            resize_display: false,
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
    high_resolution: Arc<AtomicBool>,
    hardware: Arc<AtomicBool>,
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

    /// HD on (the default): up to the captured size; off: at most 1080p.
    pub fn set_high_resolution(&self, enabled: bool) {
        self.high_resolution.store(enabled, Ordering::Relaxed);
    }

    /// Whether the video chip encodes the stream now (false: the processor, OpenH264).
    pub fn hardware(&self) -> bool {
        self.hardware.load(Ordering::Relaxed)
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

/// Average encoding time above this share of the frame interval: the machine struggles
/// with a large screen, so it is held to [`rate::FULL_HD_PIXELS`] for the rest of the session.
const HEAVY_ENCODE: f64 = 0.6;
/// Frames encoded before judging the encoding time (the first ones include warm-up).
const HEAVY_SAMPLE: u32 = 60;

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
                // The hardware encoder reads the captured image as it is (no copy).
                native: config.hardware_encoding && codec::HARDWARE_BUILT,
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
    let high_resolution = Arc::new(AtomicBool::new(true));
    let hardware = Arc::new(AtomicBool::new(false));
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
            high_resolution: high_resolution.clone(),
            hardware: hardware.clone(),
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
        high_resolution,
        hardware,
        size: size_rx,
        thread: Some(thread),
    })
}

/// Shared with [`StreamHandle`]: how the async side steers the encoder thread.
struct Controls {
    stop: Arc<AtomicBool>,
    keyframe: Arc<AtomicBool>,
    bitrate: Arc<AtomicU32>,
    high_resolution: Arc<AtomicBool>,
    hardware: Arc<AtomicBool>,
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
        high_resolution,
        hardware: encoding_in_hardware,
        size,
    } = controls;
    let hardware_failed = std::cell::Cell::new(false);
    let new_encoder = |(width, height): (u32, u32)| -> codec::Result<Box<dyn VideoEncoder>> {
        let settings = EncoderSettings {
            fps: config.fps,
            bitrate_bps: rate::max_bitrate_for(
                u64::from(width) * u64::from(height),
                config.bitrate_bps,
            ),
            keyframe_interval: config.keyframe_interval,
            ..EncoderSettings::new(width, height)
        };
        // Once the hardware refused, stay on software for this stream.
        let hardware = config.hardware_encoding && !hardware_failed.get();
        let (encoder, failure) = codec::new_encoder(&settings, hardware)?;
        if let Some(e) = failure {
            hardware_failed.set(true);
            tracing::warn!("hardware encoder unavailable, using software: {e}");
        }
        encoding_in_hardware.store(encoder.is_hardware(), Ordering::Relaxed);
        tracing::info!(encoder = encoder.name(), size = ?(width, height), "video encoder ready");
        Ok(encoder)
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
    let captured_pixels = u64::from(captured_size.0) * u64::from(captured_size.1);
    // Resolution in percent of the captured size, lowered while the bandwidth is low.
    let mut scale = 100;
    // Highest scale this machine encodes fast enough (lowered once if it struggles).
    let mut cpu_cap = 100;
    let mut encode_avg: Option<Duration> = None;
    let mut encoded_frames = 0u32;
    // Limit chosen by the viewer (HD off: 1080p), applied without waiting for the gaps.
    let full_hd = rate::scale_to_fit(captured_pixels, rate::FULL_HD_PIXELS);
    let mut user_cap = 100;
    let mut last_rescale: Option<Instant> = None;
    let start = Instant::now();
    let mut last_keyframe: Option<Instant> = None;
    // Kept so a keyframe can be produced even while the screen is static.
    let mut last_frame: Option<Frame> = None;
    // Weights and buffers of the shrink to the reduced resolution, kept between frames.
    let mut shrinker = capture::scale::Shrinker::default();
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
        let cap = if high_resolution.load(Ordering::Relaxed) {
            100
        } else {
            full_hd
        };
        if cap != user_cap {
            user_cap = cap;
            last_rescale = None;
        }
        let wanted = rate::scale_for(applied_bitrate, scale, captured_pixels)
            .min(cpu_cap)
            .min(user_cap);
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
        let work_started = Instant::now();

        if want_keyframe {
            encoder.force_keyframe();
            last_keyframe = Some(Instant::now());
        }
        last_encoded = Some(Instant::now());
        let ts = frame.captured_at.duration_since(start).as_millis() as u64;
        // Full size on the media engine: the captured image as it is, no copy. Otherwise
        // BGRA, shrunk on the CPU while the bandwidth is low.
        #[cfg(target_os = "macos")]
        let native = match (&frame.native, scale) {
            (Some(image), 100) => encoder.encode_native(image.as_ptr(), ts),
            _ => None,
        };
        #[cfg(not(target_os = "macos"))]
        let native: Option<codec::Result<Option<codec::EncodedFrame>>> = None;
        let result = match native {
            Some(result) => result,
            None => {
                let pixels = frame.pixels();
                let data: &[u8] = if (width, height) == (frame.width, frame.height) {
                    &pixels
                } else {
                    shrinker.shrink(&pixels, frame.width, frame.height, width, height)
                };
                encoder.encode_bgra(data, width, height, ts)
            }
        };
        let took = work_started.elapsed();
        encode_avg = Some(encode_avg.map_or(took, |avg| avg.mul_f64(0.9) + took.mul_f64(0.1)));
        encoded_frames = encoded_frames.saturating_add(1);
        let full_rate = Duration::from_secs(1) / config.fps.max(1);
        if cpu_cap == 100
            && !encoder.is_hardware()
            && encoded_frames >= HEAVY_SAMPLE
            && u64::from(width) * u64::from(height) > rate::FULL_HD_PIXELS
            && encode_avg.is_some_and(|avg| avg > full_rate.mul_f64(HEAVY_ENCODE))
        {
            cpu_cap = rate::scale_to_fit(captured_pixels, rate::FULL_HD_PIXELS);
            tracing::warn!(
                avg_ms = encode_avg.map(|a| a.as_millis()),
                size = ?(width, height),
                cap = cpu_cap,
                "encoding too slow for this screen size, limiting the resolution"
            );
        }
        match result {
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
