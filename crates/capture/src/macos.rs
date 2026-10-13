//! macOS backend built on ScreenCaptureKit (macOS 14+).

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::time::{Duration, Instant};

use screencapturekit::cm::SCFrameStatus;
use screencapturekit::cv::CVPixelBufferLockFlags;
use screencapturekit::prelude::*;

use crate::{CaptureConfig, CaptureError, Capturer, Display, Frame, Result};

/// Frames buffered between the ScreenCaptureKit callback and the consumer.
/// Kept tiny on purpose: for remote control a fresh frame beats a complete history.
const CHANNEL_DEPTH: usize = 2;

fn backend(err: impl std::fmt::Display) -> CaptureError {
    CaptureError::Backend(err.to_string())
}

fn shareable_displays() -> Result<Vec<SCDisplay>> {
    let content =
        SCShareableContent::get().map_err(|e| CaptureError::PermissionDenied(e.to_string()))?;
    Ok(content.displays())
}

fn display_filter(display: &SCDisplay) -> Result<SCContentFilter> {
    SCContentFilter::create()
        .with_display(display)
        .with_excluding_windows(&[])
        .build()
        .map_err(backend)
}

/// The main display has its origin at (0, 0) in global coordinates.
fn is_main(display: &SCDisplay) -> bool {
    let f = display.frame();
    f.origin.x == 0.0 && f.origin.y == 0.0
}

fn bounds(display: &SCDisplay) -> crate::Rect {
    let f = display.frame();
    crate::Rect {
        x: f.origin.x,
        y: f.origin.y,
        width: f.size.width,
        height: f.size.height,
    }
}

/// SCDisplay reports its size in points; scale it to native pixels.
fn describe(display: &SCDisplay, index: usize) -> Result<Display> {
    let scale = display_filter(display)?.point_pixel_scale();
    Ok(Display {
        id: display.display_id(),
        name: format!("Schermo {}", index + 1),
        width: (display.width() as f32 * scale).round() as u32,
        height: (display.height() as f32 * scale).round() as u32,
        scale,
        primary: is_main(display),
        bounds: bounds(display),
    })
}

/// Displays with the main one first, then left to right.
fn ordered(mut displays: Vec<SCDisplay>) -> Vec<SCDisplay> {
    displays.sort_by(|a, b| {
        is_main(b)
            .cmp(&is_main(a))
            .then(a.frame().origin.x.total_cmp(&b.frame().origin.x))
    });
    displays
}

pub fn displays() -> Result<Vec<Display>> {
    ordered(shareable_displays()?)
        .iter()
        .enumerate()
        .map(|(i, d)| describe(d, i))
        .collect()
}

pub struct MacCapturer {
    stream: SCStream,
    frames: Receiver<Frame>,
    size: (u32, u32),
    display_id: u32,
    bounds: crate::Rect,
}

impl MacCapturer {
    pub fn start(config: &CaptureConfig) -> Result<Self> {
        let displays = ordered(shareable_displays()?);
        let display = match config.display_id {
            Some(id) => displays
                .into_iter()
                .find(|d| d.display_id() == id)
                .ok_or(CaptureError::DisplayNotFound(id))?,
            // Primary first after ordering: mouse injection defaults to it too.
            None => displays.into_iter().next().ok_or(CaptureError::NoDisplay)?,
        };

        let info = describe(&display, 0)?;
        let (width, height) = crate::output_size(info.width, info.height, config.max_size);
        let fps = config.max_fps.max(1);

        let stream_config = SCStreamConfiguration::new()
            .with_width(width)
            .with_height(height)
            .with_pixel_format(PixelFormat::BGRA)
            .with_shows_cursor(config.show_cursor)
            .with_minimum_frame_interval(&CMTime::new(1, fps as i32))
            .with_queue_depth(3);

        let (tx, rx) = mpsc::sync_channel(CHANNEL_DEPTH);
        let mut stream =
            SCStream::new(&display_filter(&display)?, &stream_config).map_err(backend)?;
        stream
            .add_output_handler(FrameSink { tx }, SCStreamOutputType::Screen)
            .map_err(backend)?;
        stream.start_capture().map_err(backend)?;

        tracing::info!(
            display = info.id,
            native = ?(info.width, info.height),
            output = ?(width, height),
            fps,
            "ScreenCaptureKit stream started"
        );

        Ok(Self {
            stream,
            frames: rx,
            size: (width, height),
            display_id: info.id,
            bounds: info.bounds,
        })
    }
}

impl Capturer for MacCapturer {
    fn output_size(&self) -> (u32, u32) {
        self.size
    }

    fn display_id(&self) -> u32 {
        self.display_id
    }

    fn bounds(&self) -> crate::Rect {
        self.bounds
    }

    fn next_frame(&mut self, timeout: Duration) -> Result<Option<Frame>> {
        match self.frames.recv_timeout(timeout) {
            Ok(frame) => Ok(Some(frame)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(CaptureError::Stopped),
        }
    }
}

impl Drop for MacCapturer {
    fn drop(&mut self) {
        if let Err(e) = self.stream.stop_capture() {
            tracing::warn!("failed to stop capture: {e}");
        }
    }
}

struct FrameSink {
    tx: SyncSender<Frame>,
}

impl SCStreamOutputTrait for FrameSink {
    fn did_output_sample_buffer(&self, sample: CMSampleBuffer, kind: SCStreamOutputType) {
        if !matches!(kind, SCStreamOutputType::Screen) {
            return;
        }
        // Idle/blank frames carry no new content.
        if sample.frame_status() != Some(SCFrameStatus::Complete) {
            return;
        }
        let Some(frame) = copy_frame(&sample) else {
            return;
        };
        match self.tx.try_send(frame) {
            Ok(()) | Err(TrySendError::Disconnected(_)) => {}
            Err(TrySendError::Full(_)) => tracing::trace!("consumer slow, frame dropped"),
        }
    }
}

/// Copies the pixel buffer into a tightly packed BGRA `Vec`, removing row padding.
fn copy_frame(sample: &CMSampleBuffer) -> Option<Frame> {
    let captured_at = Instant::now();
    let pixel_buffer = sample.pixel_buffer()?;
    let guard = pixel_buffer.lock(CVPixelBufferLockFlags::READ_ONLY).ok()?;
    let (width, height, stride) = (guard.width(), guard.height(), guard.bytes_per_row());
    // SAFETY: the buffer stays locked for the lifetime of `guard`.
    let src = unsafe { guard.as_slice() }?;

    let row_len = width * 4;
    let mut data = Vec::with_capacity(row_len * height);
    for row in src.chunks(stride).take(height) {
        data.extend_from_slice(&row[..row_len]);
    }

    Some(Frame {
        width: width as u32,
        height: height as u32,
        data,
        captured_at,
    })
}
