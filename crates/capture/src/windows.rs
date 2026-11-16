//! Windows backend: DXGI Desktop Duplication of one monitor.
//!
//! Notes: the mouse pointer is not part of duplicated frames; the secure desktop (UAC
//! prompts, lock screen, Ctrl+Alt+Del) cannot be captured from a user process and shows as
//! access lost. The agent (SYSTEM) follows it. A new duplication there only delivers frames
//! once something changes, so the first picture after each desktop change is a GDI copy;
//! where the duplication cannot be opened at all, GDI stands in until it can. A lost or
//! failed duplication is reopened until it works again.

use std::time::{Duration, Instant};

use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CAPTUREBLT, CreateCompatibleDC, CreateDIBSection,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetMonitorInfoW, HMONITOR, MONITORINFO,
    MONITORINFOEXW, ReleaseDC, SRCCOPY, SelectObject,
};
use windows_capture::dxgi_duplication_api::{DxgiDuplicationApi, DxgiDuplicationFormat, Error};
use windows_capture::monitor::Monitor;

use crate::{CaptureConfig, CaptureError, Capturer, Display, Frame, Rect, Result};

/// `MONITORINFOF_PRIMARY`.
const PRIMARY: u32 = 1;
/// Least time between two GDI copies (~10 fps: secure desktops are mostly still).
const GDI_INTERVAL: Duration = Duration::from_millis(100);
/// How often a failed duplication is tried again while GDI stands in.
const DXGI_RETRY: Duration = Duration::from_secs(2);

/// Copies `bounds` (virtual-desktop pixels) of the calling thread's desktop with GDI into
/// `out` as top-down BGRA.
fn gdi_copy(bounds: Rect, out: &mut Vec<u8>) -> std::result::Result<(u32, u32), String> {
    let (x, y) = (bounds.x as i32, bounds.y as i32);
    let (w, h) = (bounds.width as i32, bounds.height as i32);
    if w <= 0 || h <= 0 {
        return Err("empty monitor".into());
    }
    let info = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h, // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        },
        ..Default::default()
    };
    // SAFETY: plain GDI calls; every object made here is released before returning.
    unsafe {
        let screen = GetDC(None);
        if screen.is_invalid() {
            return Err("GetDC failed".into());
        }
        let memory = CreateCompatibleDC(Some(screen));
        let mut bits: *mut core::ffi::c_void = std::ptr::null_mut();
        let result = match CreateDIBSection(Some(memory), &info, DIB_RGB_COLORS, &mut bits, None, 0)
        {
            Ok(bitmap) => {
                let previous = SelectObject(memory, bitmap.into());
                let copied = BitBlt(memory, 0, 0, w, h, Some(screen), x, y, SRCCOPY | CAPTUREBLT);
                SelectObject(memory, previous);
                let result = match copied {
                    Ok(()) if !bits.is_null() => {
                        let len = (w as usize) * (h as usize) * 4;
                        out.clear();
                        out.extend_from_slice(std::slice::from_raw_parts(bits.cast::<u8>(), len));
                        Ok((w as u32, h as u32))
                    }
                    Ok(()) => Err("no DIB bits".to_string()),
                    Err(e) => Err(format!("BitBlt: {e}")),
                };
                let _ = DeleteObject(bitmap.into());
                result
            }
            Err(e) => Err(format!("CreateDIBSection: {e}")),
        };
        let _ = DeleteDC(memory);
        ReleaseDC(None, screen);
        result
    }
}

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
    /// The next frame is a GDI copy: the desktop just changed and the new duplication
    /// stays silent until something on screen changes.
    primer: bool,
    last_gdi: Option<Instant>,
    last_dxgi_try: Instant,
    /// A GDI failure was logged since the last desktop change.
    gdi_failed: bool,
}

impl WinCapturer {
    pub fn start(config: &CaptureConfig) -> Result<Self> {
        // Agent (SYSTEM): duplicate the desktop receiving input (normal or secure one).
        windesk::follow_input_desktop();
        let (monitor, chosen) = select(config.display_id)?;
        let output = crate::output_size(chosen.width, chosen.height, config.max_size);
        let duplication =
            match DxgiDuplicationApi::new_options(monitor, &[DxgiDuplicationFormat::Bgra8]) {
                Ok(duplication) => Some(duplication),
                // The agent may start on the secure desktop (a session opened while the
                // Ctrl+Alt+Del or lock screen is up): GDI until the duplication opens.
                Err(e) if windesk::enabled() => {
                    tracing::warn!(
                        desktop = ?windesk::input_desktop_name(),
                        "cannot duplicate this desktop, copying it with GDI: {e}"
                    );
                    None
                }
                Err(e) => return Err(backend(e)),
            };
        tracing::info!(
            display = chosen.id,
            native = ?(chosen.width, chosen.height),
            ?output,
            gdi = duplication.is_none(),
            "capture started"
        );
        Ok(Self {
            duplication,
            display: chosen,
            output,
            scratch: Vec::new(),
            primer: false,
            last_gdi: None,
            last_dxgi_try: Instant::now(),
            gdi_failed: false,
        })
    }

    fn reopen(&self) -> std::result::Result<DxgiDuplicationApi, String> {
        let (monitor, _) = select(Some(self.display.id)).map_err(|e| e.to_string())?;
        DxgiDuplicationApi::new_options(monitor, &[DxgiDuplicationFormat::Bgra8])
            .map_err(|e| e.to_string())
    }

    /// GDI stands in for the duplication (paced at `GDI_INTERVAL`), trying the duplication
    /// again every `DXGI_RETRY` when it failed to open.
    fn gdi_frame(&mut self, timeout: Duration) -> Result<Option<Frame>> {
        if self.duplication.is_none() && self.last_dxgi_try.elapsed() >= DXGI_RETRY {
            self.last_dxgi_try = Instant::now();
            if let Ok(duplication) = self.reopen() {
                tracing::info!("desktop duplication reopened");
                self.duplication = Some(duplication);
                return Ok(None);
            }
        }
        if let Some(last) = self.last_gdi {
            let wait = GDI_INTERVAL.saturating_sub(last.elapsed());
            if wait > timeout {
                std::thread::sleep(timeout);
                return Ok(None);
            }
            std::thread::sleep(wait);
        }
        self.last_gdi = Some(Instant::now());
        let bounds = Rect {
            width: f64::from(self.display.width),
            height: f64::from(self.display.height),
            ..self.display.bounds
        };
        let (w, h) = match gdi_copy(bounds, &mut self.scratch) {
            Ok(size) => size,
            Err(e) => {
                if !self.gdi_failed {
                    self.gdi_failed = true;
                    tracing::warn!(desktop = ?windesk::input_desktop_name(), "GDI copy failed: {e}");
                }
                return Ok(None);
            }
        };
        let captured_at = Instant::now();
        let data = if (w, h) == self.output {
            self.scratch.clone()
        } else {
            crate::scale::scale_bgra(&self.scratch, w, h, self.output.0, self.output.1)
        };
        Ok(Some(Frame {
            width: self.output.0,
            height: self.output.1,
            data: data.into(),
            captured_at,
            native: None,
        }))
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
            let desktop = windesk::input_desktop_name();
            tracing::info!(?desktop, "input desktop changed");
            self.duplication = None;
            self.duplication = match self.reopen() {
                Ok(duplication) => Some(duplication),
                Err(e) => {
                    tracing::warn!(
                        ?desktop,
                        "cannot duplicate this desktop, copying it with GDI: {e}"
                    );
                    None
                }
            };
            self.last_dxgi_try = Instant::now();
            self.gdi_failed = false;
            self.primer = true;
            self.last_gdi = None;
        }
        if self.primer {
            self.primer = false;
            return self.gdi_frame(timeout);
        }
        if self.duplication.is_none() {
            return self.gdi_frame(timeout);
        }
        let Some(duplication) = self.duplication.as_mut() else {
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
