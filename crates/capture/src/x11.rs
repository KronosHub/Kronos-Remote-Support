//! Linux backend: X11 `GetImage` of the root window (all monitors).
//!
//! Wayland sessions are not supported yet (they need xdg-desktop-portal + PipeWire):
//! through XWayland, native Wayland windows would come out black.

use std::time::{Duration, Instant};

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt, ImageFormat, Window};
use x11rb::rust_connection::RustConnection;

use crate::{CaptureConfig, CaptureError, Capturer, Display, Frame, Result};

fn backend(err: impl std::fmt::Display) -> CaptureError {
    CaptureError::Backend(err.to_string())
}

fn ensure_x11() -> Result<()> {
    let wayland =
        std::env::var("XDG_SESSION_TYPE").is_ok_and(|t| t.eq_ignore_ascii_case("wayland"));
    if wayland {
        return Err(CaptureError::Backend(
            "sessione Wayland non ancora supportata: accedi con una sessione Xorg".into(),
        ));
    }
    Ok(())
}

fn connect() -> Result<(RustConnection, Window, u32, u32, u8)> {
    ensure_x11()?;
    let (conn, screen_num) = x11rb::connect(None).map_err(|e| backend(format!("X11: {e}")))?;
    let screen = &conn.setup().roots[screen_num];
    let (root, w, h, depth) = (
        screen.root,
        u32::from(screen.width_in_pixels),
        u32::from(screen.height_in_pixels),
        screen.root_depth,
    );
    Ok((conn, root, w, h, depth))
}

pub fn displays() -> Result<Vec<Display>> {
    let (_, _, width, height, _) = connect()?;
    Ok(vec![Display {
        id: 0,
        name: "Tutti gli schermi".into(),
        width,
        height,
        scale: 1.0,
        primary: true,
        bounds: root_bounds(width, height),
    }])
}

fn root_bounds(width: u32, height: u32) -> crate::Rect {
    crate::Rect {
        x: 0.0,
        y: 0.0,
        width: f64::from(width),
        height: f64::from(height),
    }
}

pub struct X11Capturer {
    conn: RustConnection,
    root: Window,
    native: (u32, u32),
    output: (u32, u32),
    interval: Duration,
    next_due: Instant,
    previous: Vec<u8>,
}

impl X11Capturer {
    pub fn start(config: &CaptureConfig) -> Result<Self> {
        let (conn, root, w, h, depth) = connect()?;
        if depth != 24 && depth != 32 {
            return Err(backend(format!(
                "profondità colore X11 non supportata: {depth}"
            )));
        }
        let output = crate::output_size(w, h, config.max_size);
        tracing::info!(native = ?(w, h), ?output, "X11 capture started");
        Ok(Self {
            conn,
            root,
            native: (w, h),
            output,
            interval: Duration::from_secs(1) / config.max_fps.max(1),
            next_due: Instant::now(),
            previous: Vec::new(),
        })
    }

    fn grab(&self) -> Result<Vec<u8>> {
        let (w, h) = self.native;
        let reply = self
            .conn
            .get_image(
                ImageFormat::Z_PIXMAP,
                self.root,
                0,
                0,
                w as u16,
                h as u16,
                !0,
            )
            .map_err(backend)?
            .reply()
            .map_err(backend)?;
        // 24/32-bit TrueColor on little-endian servers is BGRX, which is our BGRA layout.
        if reply.data.len() != (w * h * 4) as usize {
            return Err(backend("formato immagine X11 inatteso"));
        }
        Ok(reply.data)
    }
}

impl Capturer for X11Capturer {
    fn output_size(&self) -> (u32, u32) {
        self.output
    }

    fn display_id(&self) -> u32 {
        0
    }

    fn bounds(&self) -> crate::Rect {
        root_bounds(self.native.0, self.native.1)
    }

    fn next_frame(&mut self, timeout: Duration) -> Result<Option<Frame>> {
        let now = Instant::now();
        if self.next_due > now {
            let wait = self.next_due - now;
            std::thread::sleep(wait.min(timeout));
            if wait > timeout {
                return Ok(None);
            }
        }
        self.next_due = Instant::now() + self.interval;

        let captured_at = Instant::now();
        let pixels = self.grab()?;
        // X11 has no damage-driven delivery here: skip frames identical to the last one.
        if pixels == self.previous {
            return Ok(None);
        }
        let (w, h) = self.native;
        let data = crate::scale::scale_bgra(&pixels, w, h, self.output.0, self.output.1);
        self.previous = pixels;
        Ok(Some(Frame {
            width: self.output.0,
            height: self.output.1,
            data,
            captured_at,
            native: None,
        }))
    }
}
