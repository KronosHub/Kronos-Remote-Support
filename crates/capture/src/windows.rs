//! Windows backend: DXGI Desktop Duplication of one monitor.
//!
//! Notes: the mouse pointer is not part of duplicated frames; the secure desktop (UAC
//! prompts, lock screen) cannot be captured from a user process and shows as access lost.
//! A lost or failed duplication is reopened until it works again: the stream only pauses.

use std::time::{Duration, Instant};

use windows::Win32::Graphics::Gdi::{GetMonitorInfoW, HMONITOR, MONITORINFO, MONITORINFOEXW};
use windows_capture::dxgi_duplication_api::{DxgiDuplicationApi, DxgiDuplicationFormat, Error};
use windows_capture::monitor::Monitor;

use crate::{CaptureConfig, CaptureError, Capturer, Display, Frame, Rect, Result};

/// `MONITORINFOF_PRIMARY`.
const PRIMARY: u32 = 1;

fn backend(err: impl std::fmt::Display) -> CaptureError {
    CaptureError::Backend(err.to_string())
}

/// Monitor rectangle in virtual-desktop pixels and whether it is the primary one.
fn placement(monitor: &Monitor) -> (Rect, bool) {
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    let handle = HMONITOR(monitor.as_raw_hmonitor());
    if !unsafe { GetMonitorInfoW(handle, &mut info) }.as_bool() {
        return (Rect::default(), false);
    }
    let r = info.rcMonitor;
    (
        Rect {
            x: f64::from(r.left),
            y: f64::from(r.top),
            width: f64::from(r.right - r.left),
            height: f64::from(r.bottom - r.top),
        },
        info.dwFlags & PRIMARY != 0,
    )
}

fn describe(index: usize, monitor: &Monitor) -> Result<Display> {
    let (bounds, primary) = placement(monitor);
    Ok(Display {
        id: index as u32,
        name: monitor
            .name()
            .ok()
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| format!("Schermo {}", index + 1)),
        width: monitor.width().map_err(backend)?,
        height: monitor.height().map_err(backend)?,
        scale: 1.0,
        primary,
        bounds,
    })
}

/// GDI name of the display (`\\.\DISPLAY1`), NUL-terminated, for the display mode calls.
pub(crate) fn device_name(display_id: u32) -> Result<Vec<u16>> {
    let monitor = monitors()?
        .into_iter()
        .nth(display_id as usize)
        .ok_or(CaptureError::DisplayNotFound(display_id))?;
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    let handle = HMONITOR(monitor.as_raw_hmonitor());
    if !unsafe { GetMonitorInfoW(handle, &mut info.monitorInfo) }.as_bool() {
        return Err(CaptureError::DisplayNotFound(display_id));
    }
    let len = info
        .szDevice
        .iter()
        .position(|&c| c == 0)
        .unwrap_or(info.szDevice.len());
    let mut name = info.szDevice[..len].to_vec();
    name.push(0);
    Ok(name)
}

fn monitors() -> Result<Vec<Monitor>> {
    Monitor::enumerate().map_err(backend)
}

pub fn displays() -> Result<Vec<Display>> {
    let mut list: Vec<Display> = monitors()?
        .iter()
        .enumerate()
        .map(|(i, m)| describe(i, m))
        .collect::<Result<_>>()?;
    list.sort_by_key(|d| (!d.primary, d.bounds.x as i64));
    Ok(list)
}

/// Finds the monitor to capture: by id (enumeration index) or the primary one.
fn select(display_id: Option<u32>) -> Result<(Monitor, Display)> {
    let monitors = monitors()?;
    let found = monitors
        .into_iter()
        .enumerate()
        .map(|(i, m)| describe(i, &m).map(|d| (m, d)))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .find(|(_, d)| match display_id {
            Some(id) => d.id == id,
            None => d.primary,
        });
    match (found, display_id) {
        (Some(pair), _) => Ok(pair),
        (None, Some(id)) => Err(CaptureError::DisplayNotFound(id)),
        (None, None) => Err(CaptureError::NoDisplay),
    }
}

pub struct WinCapturer {
    duplication: Option<DxgiDuplicationApi>,
    display: Display,
    output: (u32, u32),
    scratch: Vec<u8>,
}

impl WinCapturer {
    pub fn start(config: &CaptureConfig) -> Result<Self> {
        // Agent (SYSTEM): duplicate the desktop receiving input (normal or secure one).
        windesk::follow_input_desktop();
        let (monitor, chosen) = select(config.display_id)?;
        let output = crate::output_size(chosen.width, chosen.height, config.max_size);
        let duplication = DxgiDuplicationApi::new_options(monitor, &[DxgiDuplicationFormat::Bgra8])
            .map_err(backend)?;
        tracing::info!(
            display = chosen.id,
            native = ?(chosen.width, chosen.height),
            ?output,
            "DXGI duplication started"
        );
        Ok(Self {
            duplication: Some(duplication),
            display: chosen,
            output,
            scratch: Vec::new(),
        })
    }

    fn reopen(&self) -> Option<DxgiDuplicationApi> {
        let (monitor, _) = select(Some(self.display.id)).ok()?;
        DxgiDuplicationApi::new_options(monitor, &[DxgiDuplicationFormat::Bgra8]).ok()
    }
}

impl Capturer for WinCapturer {
    fn output_size(&self) -> (u32, u32) {
        self.output
    }

    fn display_id(&self) -> u32 {
        self.display.id
    }

    fn bounds(&self) -> Rect {
        self.display.bounds
    }

    fn next_frame(&mut self, timeout: Duration) -> Result<Option<Frame>> {
        // The input desktop changed (UAC prompt, lock screen): a duplication belongs to one
        // desktop, so open a new one on the desktop now shown.
        if windesk::follow_input_desktop() {
            tracing::info!(desktop = ?windesk::input_desktop_name(), "input desktop changed");
            self.duplication = None;
            self.duplication = self.reopen();
            return Ok(None);
        }
        let Some(duplication) = self.duplication.as_mut() else {
            // Lost earlier (e.g. secure desktop): retry opening now.
            std::thread::sleep(timeout);
            self.duplication = self.reopen();
            if self.duplication.is_some() {
                tracing::info!("desktop duplication reopened");
            }
            return Ok(None);
        };
        let timeout_ms = timeout.as_millis().min(u32::MAX as u128) as u32;
        let output = self.output;
        let native = (self.display.width, self.display.height);
        let result = match duplication.acquire_next_frame(timeout_ms) {
            Ok(mut frame) => {
                // Zero present time: only the pointer moved, the image is unchanged.
                if frame.frame_info().LastPresentTime == 0 {
                    return Ok(None);
                }
                let captured_at = Instant::now();
                frame.buffer().map(|buffer| {
                    let (w, h) = (buffer.width(), buffer.height());
                    let pixels = buffer.as_nopadding_buffer(&mut self.scratch);
                    // One copy out of the duplication's buffer: scaled straight from it, or
                    // copied as is at native size (another size ends the capture below).
                    let data = if (w, h) != native {
                        Vec::new()
                    } else if (w, h) == output {
                        pixels.to_vec()
                    } else {
                        crate::scale::scale_bgra(pixels, w, h, output.0, output.1)
                    };
                    (data, w, h, captured_at)
                })
            }
            Err(Error::Timeout) => return Ok(None),
            Err(e) => Err(e),
        };
        let (data, w, h, captured_at) = match result {
            Ok(frame) => frame,
            Err(Error::AccessLost) => {
                tracing::info!("desktop duplication access lost, recreating");
                self.duplication = self
                    .duplication
                    .take()
                    .and_then(|d| d.recreate_options(&[DxgiDuplicationFormat::Bgra8]).ok());
                return Ok(None);
            }
            // Other failures happen too while Windows moves between the lock screen and the
            // desktop: never end the stream for them, open a new duplication instead.
            Err(e) => {
                tracing::warn!("desktop duplication failed, reopening: {e}");
                self.duplication = None;
                return Ok(None);
            }
        };
        if (w, h) != (self.display.width, self.display.height) {
            // E.g. the desktop after login runs at another resolution than the lock screen:
            // end here so the stream starts again at the new size.
            return Err(CaptureError::Backend(format!(
                "monitor size changed to {w}x{h}"
            )));
        }
        Ok(Some(Frame {
            width: self.output.0,
            height: self.output.1,
            data: data.into(),
            captured_at,
            native: None,
        }))
    }
}
