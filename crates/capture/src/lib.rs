//! Cross-platform screen capture.
//!
//! Every backend delivers frames as tightly packed BGRA (stride = width * 4),
//! so encoders downstream do not need to care about platform row padding.

use std::time::{Duration, Instant};

pub mod composite;
#[cfg(target_os = "macos")]
mod macos;
mod pattern;
pub mod resolution;
pub mod scale;
pub mod shared;
#[cfg(target_os = "macos")]
pub mod virtual_display;
#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "linux")]
mod x11;

pub use pattern::TestPattern;

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("screen capture is not supported on this platform yet")]
    Unsupported,
    #[error("screen recording permission denied or no shareable content: {0}")]
    PermissionDenied(String),
    #[error("display {0} not found")]
    DisplayNotFound(u32),
    #[error("no displays available")]
    NoDisplay,
    #[error("capture stream stopped")]
    Stopped,
    #[error("capture backend error: {0}")]
    Backend(String),
}

pub type Result<T> = std::result::Result<T, CaptureError>;

/// `CaptureConfig::display_id` value that captures every display side by side, as laid out
/// on the host (on X11 the root window already covers all monitors).
pub const ALL_DISPLAYS: u32 = u32::MAX;

/// Rectangle in the coordinate space used for mouse injection on this OS
/// (macOS: global points; Windows: virtual-desktop pixels; X11: root window pixels).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// A physical display that can be captured.
#[derive(Debug, Clone)]
pub struct Display {
    pub id: u32,
    pub name: String,
    /// Native resolution in pixels.
    pub width: u32,
    pub height: u32,
    /// Pixels per point (2.0 on Retina).
    pub scale: f32,
    pub primary: bool,
    /// Where the display sits in input coordinates.
    pub bounds: Rect,
}

#[derive(Debug, Clone)]
pub struct CaptureConfig {
    /// Display to capture (`Display::id`); `None` means the primary display.
    /// X11 always captures the whole root window (all monitors).
    pub display_id: Option<u32>,
    /// Upper bound on delivered frames per second.
    pub max_fps: u32,
    /// Downscale (keeping aspect ratio) so the output fits in this `(width, height)` box.
    pub max_size: Option<(u32, u32)>,
    pub show_cursor: bool,
    /// Hand frames over as the system's own image where the backend can (macOS: the
    /// ScreenCaptureKit pixel buffer, for a hardware encoder) instead of copying them to
    /// BGRA; [`Frame::pixels`] copies on demand. Ignored elsewhere.
    pub native: bool,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            display_id: None,
            max_fps: 30,
            max_size: Some((1920, 1080)),
            show_cursor: true,
            native: false,
        }
    }
}

/// One captured frame: tightly packed BGRA in `data`, or (native capture) an empty `data`
/// and the system image in `native`. The pixels are shared: cloning a frame (for every
/// session, as the last frame, for a keyframe) does not copy them.
#[derive(Debug, Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub data: std::sync::Arc<Vec<u8>>,
    pub captured_at: Instant,
    pub native: Option<NativeImage>,
}

#[cfg(target_os = "macos")]
pub use macos::NativeImage;

/// No native images on this platform.
#[cfg(not(target_os = "macos"))]
#[derive(Debug, Clone)]
pub enum NativeImage {}

impl Frame {
    /// The frame in tightly packed BGRA, copied out of the native image if needed (empty
    /// if that fails).
    pub fn pixels(&self) -> std::borrow::Cow<'_, [u8]> {
        if !self.data.is_empty() {
            return std::borrow::Cow::Borrowed(&self.data);
        }
        #[cfg(target_os = "macos")]
        if let Some(native) = &self.native {
            return std::borrow::Cow::Owned(native.to_bgra().unwrap_or_default());
        }
        std::borrow::Cow::Borrowed(&self.data)
    }
}

/// A running capture session. Dropping it stops the capture.
pub trait Capturer: Send {
    /// Output frame size in pixels (always even, as required by YUV 4:2:0).
    fn output_size(&self) -> (u32, u32);

    /// Id of the display being captured.
    fn display_id(&self) -> u32;

    /// Captured area in input coordinates, so remote mouse positions can be mapped onto it.
    fn bounds(&self) -> Rect;

    /// Waits up to `timeout` for the next frame whose content changed.
    /// Returns `Ok(None)` on timeout (e.g. static screen).
    fn next_frame(&mut self, timeout: Duration) -> Result<Option<Frame>>;
}

pub fn displays() -> Result<Vec<Display>> {
    #[cfg(target_os = "macos")]
    return macos::displays();
    #[cfg(target_os = "windows")]
    return windows::displays();
    #[cfg(target_os = "linux")]
    return x11::displays();
    #[allow(unreachable_code)]
    Err(CaptureError::Unsupported)
}

pub fn start(config: &CaptureConfig) -> Result<Box<dyn Capturer>> {
    #[cfg(not(target_os = "linux"))]
    if config.display_id == Some(ALL_DISPLAYS) {
        return Ok(Box::new(composite::CompositeCapturer::start(
            config,
            displays()?,
            start_one,
        )?));
    }
    start_one(config)
}

fn start_one(config: &CaptureConfig) -> Result<Box<dyn Capturer>> {
    #[cfg(target_os = "macos")]
    return Ok(Box::new(macos::MacCapturer::start(config)?));
    #[cfg(target_os = "windows")]
    return Ok(Box::new(windows::WinCapturer::start(config)?));
    #[cfg(target_os = "linux")]
    return Ok(Box::new(x11::X11Capturer::start(config)?));
    #[allow(unreachable_code)]
    {
        let _ = config;
        Err(CaptureError::Unsupported)
    }
}

/// Fits `(width, height)` inside the `max_size` box keeping aspect ratio,
/// rounding down to even sizes. Never upscales.
pub fn output_size(width: u32, height: u32, max_size: Option<(u32, u32)>) -> (u32, u32) {
    let (w, h) = match max_size {
        Some((max_w, max_h)) if width > max_w || height > max_h => {
            let (w, h, max_w, max_h) = (
                u64::from(width),
                u64::from(height),
                u64::from(max_w),
                u64::from(max_h),
            );
            // Compare aspect ratios to find which side hits the box first.
            if w * max_h >= h * max_w {
                (max_w, h * max_w / w)
            } else {
                (w * max_h / h, max_h)
            }
        }
        _ => (u64::from(width), u64::from(height)),
    };
    (w as u32 & !1, h as u32 & !1)
}

#[cfg(test)]
mod tests {
    use super::output_size;

    #[test]
    fn keeps_size_under_limit() {
        assert_eq!(output_size(1440, 900, Some((1920, 1080))), (1440, 900));
    }

    #[test]
    fn width_limited() {
        assert_eq!(output_size(3840, 1600, Some((1920, 1080))), (1920, 800));
    }

    #[test]
    fn height_limited() {
        // MacBook Pro 16" Retina vs OpenH264 limit: height is the binding side.
        assert_eq!(output_size(3456, 2234, Some((3840, 2160))), (3340, 2160));
    }

    #[test]
    fn rounds_to_even() {
        assert_eq!(output_size(1281, 721, None), (1280, 720));
    }
}
