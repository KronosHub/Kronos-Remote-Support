//! The host's mouse pointer shape, so the viewer can show the same one (ADR 0077).
//!
//! Sampled a few times per second on a thread of its own; only changes are reported.
//! Windows: the system cursors map to CSS names (the viewer shows its own native ones),
//! other cursors travel as a picture. macOS: always a picture (the cursor's 2x image).
//! On the Windows secure desktop (Ctrl+Alt+Del) the arrow. Linux: not sampled (the viewer
//! keeps its arrow).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// How often the pointer is looked at.
pub const SAMPLE_INTERVAL: Duration = Duration::from_millis(100);

/// Largest picture sent (bigger cursors keep the previous shape).
const MAX_SIDE: u32 = 256;

#[derive(Debug, Clone, PartialEq)]
pub enum Shape {
    /// A CSS cursor keyword: "default", "text", "pointer", "none" (hidden)...
    Named(&'static str),
    /// A picture: PNG, hot spot in pixels of the picture, `scale` pixels per point.
    Image {
        png: Vec<u8>,
        hot_x: u32,
        hot_y: u32,
        scale: f32,
    },
}

/// Remembers the last shape and returns the new one when it changes.
pub struct Watcher {
    state: platform::State,
}

impl Watcher {
    pub fn new() -> Self {
        Self {
            state: platform::State::default(),
        }
    }

    /// The pointer shape if it changed since the last call (None: same, or unknown).
    pub fn changed(&mut self) -> Option<Shape> {
        platform::sample(&mut self.state)
    }
}

impl Default for Watcher {
    fn default() -> Self {
        Self::new()
    }
}

/// Samples the pointer on a thread until `stop` is set; every change goes to `on_change`.
pub fn watch(stop: Arc<AtomicBool>, mut on_change: impl FnMut(Shape) + Send + 'static) {
    let spawned = std::thread::Builder::new()
        .name("cursor-watch".into())
        .spawn(move || {
            let mut watcher = Watcher::new();
            while !stop.load(Ordering::Relaxed) {
                if let Some(shape) = watcher.changed() {
                    on_change(shape);
                }
                std::thread::sleep(SAMPLE_INTERVAL);
            }
        });
    if let Err(e) = spawned {
        tracing::warn!("cannot start the cursor thread: {e}");
    }
}

#[cfg(windows)]
#[doc(hidden)]
pub use platform::{on_user_desktop, system_pictures};

/// PNG of tightly packed RGBA pixels.
#[cfg_attr(not(windows), allow(dead_code))]
fn png(rgba: Vec<u8>, width: u32, height: u32) -> Option<Vec<u8>> {
    let image = image::RgbaImage::from_raw(width, height, rgba)?;
    let mut out = std::io::Cursor::new(Vec::new());
    image.write_to(&mut out, image::ImageFormat::Png).ok()?;
    Some(out.into_inner())
}

#[cfg(windows)]
mod platform {
    use super::{MAX_SIDE, Shape, png};
    use windows_sys::Win32::Graphics::Gdi::{
        BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection,
        DIB_RGB_COLORS, DeleteDC, DeleteObject, GetObjectW, HBITMAP, SelectObject,
    };
    use windows_sys::Win32::System::StationsAndDesktops::{
        CloseDesktop, DESKTOP_READOBJECTS, GetUserObjectInformationW, OpenInputDesktop, UOI_NAME,
    };
    use windows_sys::Win32::UI::HiDpi::GetDpiForSystem;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CURSOR_SHOWING, CURSORINFO, DI_NORMAL, DrawIconEx, GetCursorInfo, GetIconInfo, HCURSOR,
        ICONINFO, IDC_APPSTARTING, IDC_ARROW, IDC_CROSS, IDC_HAND, IDC_HELP, IDC_IBEAM, IDC_NO,
        IDC_SIZEALL, IDC_SIZENESW, IDC_SIZENS, IDC_SIZENWSE, IDC_SIZEWE, IDC_UPARROW, IDC_WAIT,
        LoadCursorW,
    };

    #[derive(Default)]
    pub struct State {
        /// Last cursor handle seen (0: hidden or none yet), and whether one was reported.
        last: Option<isize>,
        named: Vec<(isize, &'static str)>,
    }

    pub fn sample(state: &mut State) -> Option<Shape> {
        let mut info: CURSORINFO = unsafe { std::mem::zeroed() };
        info.cbSize = size_of::<CURSORINFO>() as u32;
        // On the secure desktop (Ctrl+Alt+Del, UAC, lock screen) this process sees neither
        // the desktop nor its pointer: always the arrow there.
        let handle = if !on_user_desktop() || unsafe { GetCursorInfo(&mut info) } == 0 {
            SECURE
        } else if info.flags & CURSOR_SHOWING == 0 {
            0
        } else {
            info.hCursor as isize
        };
        if state.last == Some(handle) {
            return None;
        }
        state.last = Some(handle);
        match handle {
            SECURE => return Some(Shape::Named("default")),
            0 => return Some(Shape::Named("none")),
            _ => {}
        }
        if state.named.is_empty() {
            state.named = system_cursors();
        }
        if let Some((_, name)) = state.named.iter().find(|(h, _)| *h == handle) {
            return Some(Shape::Named(name));
        }
        picture(info.hCursor)
    }

    /// Stands for "on a desktop this process cannot see" in `State::last`.
    const SECURE: isize = -1;

    /// The input desktop is the user's own ("Default"), not the secure one ("Winlogon"),
    /// which a user process cannot even open.
    pub fn on_user_desktop() -> bool {
        let desk = unsafe { OpenInputDesktop(0, 0, DESKTOP_READOBJECTS) };
        if desk.is_null() {
            return false;
        }
        let mut buf = [0u16; 64];
        let mut needed = 0u32;
        let ok = unsafe {
            GetUserObjectInformationW(
                desk as _,
                UOI_NAME,
                buf.as_mut_ptr().cast(),
                (buf.len() * 2) as u32,
                &mut needed,
            )
        };
        unsafe { CloseDesktop(desk) };
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        ok == 0 || String::from_utf16_lossy(&buf[..len]).eq_ignore_ascii_case("default")
    }

    /// The shared handles of the system cursors and the CSS cursor each one stands for.
    fn system_cursors() -> Vec<(isize, &'static str)> {
        [
            (IDC_ARROW, "default"),
            (IDC_IBEAM, "text"),
            (IDC_WAIT, "wait"),
            (IDC_CROSS, "crosshair"),
            (IDC_UPARROW, "default"),
            (IDC_SIZENWSE, "nwse-resize"),
            (IDC_SIZENESW, "nesw-resize"),
            (IDC_SIZEWE, "ew-resize"),
            (IDC_SIZENS, "ns-resize"),
            (IDC_SIZEALL, "move"),
            (IDC_NO, "not-allowed"),
            (IDC_HAND, "pointer"),
            (IDC_APPSTARTING, "progress"),
            (IDC_HELP, "help"),
        ]
        .into_iter()
        .map(|(id, name)| {
            (
                unsafe { LoadCursorW(std::ptr::null_mut(), id) } as isize,
                name,
            )
        })
        .filter(|(h, _)| *h != 0)
        .collect()
    }

    /// Every system cursor as a picture (for checking the drawing, see `examples/cursor_probe`).
    pub fn system_pictures() -> Vec<(&'static str, Option<Shape>)> {
        system_cursors()
            .into_iter()
            .map(|(handle, name)| (name, picture(handle as HCURSOR)))
            .collect()
    }

    /// The cursor drawn on black and on white: the two pictures give colour and opacity
    /// (inverting pixels, as in old monochrome cursors, come out black).
    fn picture(cursor: HCURSOR) -> Option<Shape> {
        let mut icon: ICONINFO = unsafe { std::mem::zeroed() };
        if unsafe { GetIconInfo(cursor, &mut icon) } == 0 {
            return None;
        }
        let size = bitmap_size(icon.hbmColor).or_else(|| {
            // Monochrome: the mask holds the AND and XOR halves one above the other.
            bitmap_size(icon.hbmMask).map(|(w, h)| (w, h / 2))
        });
        for bitmap in [icon.hbmColor, icon.hbmMask] {
            if !bitmap.is_null() {
                unsafe { DeleteObject(bitmap) };
            }
        }
        let (width, height) = size?;
        if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
            return None;
        }
        let black = draw(cursor, width, height, 0x00)?;
        let white = draw(cursor, width, height, 0xff)?;
        let mut rgba = Vec::with_capacity(black.len());
        for (b, w) in black
            .as_chunks::<4>()
            .0
            .iter()
            .zip(white.as_chunks::<4>().0)
        {
            // BGRA in, RGBA out. Opacity from how much the background shows through.
            let through = (0..3)
                .map(|i| i32::from(w[i]) - i32::from(b[i]))
                .max()
                .unwrap_or(0);
            let alpha = (255 - through).clamp(0, 255);
            if alpha == 0 {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
                continue;
            }
            // Inverting pixels (white on black, black on white) give `through` < 0.
            let channel = |i: usize| {
                if through < 0 {
                    0
                } else {
                    (u32::from(b[i]) * 255 / alpha as u32).min(255) as u8
                }
            };
            rgba.extend_from_slice(&[channel(2), channel(1), channel(0), alpha as u8]);
        }
        Some(Shape::Image {
            png: png(rgba, width, height)?,
            hot_x: icon.xHotspot,
            hot_y: icon.yHotspot,
            // Cursors grow with the display scaling (48 px at 150%): same size in points.
            scale: (unsafe { GetDpiForSystem() } as f32 / 96.0).max(1.0),
        })
    }

    fn bitmap_size(bitmap: HBITMAP) -> Option<(u32, u32)> {
        if bitmap.is_null() {
            return None;
        }
        let mut b: BITMAP = unsafe { std::mem::zeroed() };
        let read = unsafe {
            GetObjectW(
                bitmap,
                size_of::<BITMAP>() as i32,
                (&mut b as *mut BITMAP).cast(),
            )
        };
        (read != 0).then_some((b.bmWidth as u32, b.bmHeight.unsigned_abs()))
    }

    /// BGRA pixels of the cursor drawn over a background of `fill`.
    fn draw(cursor: HCURSOR, width: u32, height: u32, fill: u8) -> Option<Vec<u8>> {
        unsafe {
            let dc = CreateCompatibleDC(std::ptr::null_mut());
            if dc.is_null() {
                return None;
            }
            let mut info: BITMAPINFO = std::mem::zeroed();
            info.bmiHeader = BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width as i32,
                biHeight: -(height as i32), // top-down
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..std::mem::zeroed()
            };
            let mut bits = std::ptr::null_mut();
            let dib = CreateDIBSection(
                dc,
                &info,
                DIB_RGB_COLORS,
                &mut bits,
                std::ptr::null_mut(),
                0,
            );
            if dib.is_null() || bits.is_null() {
                DeleteDC(dc);
                return None;
            }
            let len = (width * height * 4) as usize;
            std::ptr::write_bytes(bits.cast::<u8>(), fill, len);
            let old = SelectObject(dc, dib);
            let drawn = DrawIconEx(
                dc,
                0,
                0,
                cursor,
                width as i32,
                height as i32,
                0,
                std::ptr::null_mut(),
                DI_NORMAL,
            );
            let pixels = std::slice::from_raw_parts(bits.cast::<u8>(), len).to_vec();
            SelectObject(dc, old);
            DeleteObject(dib);
            DeleteDC(dc);
            (drawn != 0).then_some(pixels)
        }
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::{MAX_SIDE, Shape};
    use objc2::rc::autoreleasepool;
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep, NSCursor};
    use objc2_foundation::NSDictionary;

    #[derive(Default)]
    pub struct State {
        /// Size and hot spot of the last cursor (cheap to compare every time).
        key: Option<(i64, i64, i64, i64)>,
        /// The last picture sent, to drop look-alike changes of key.
        last_png: Vec<u8>,
        /// Samples since the picture was last compared in full.
        since_full: u32,
    }

    /// How many samples between full comparisons (two cursors may share size and hot spot).
    const FULL_EVERY: u32 = 10;

    pub fn sample(state: &mut State) -> Option<Shape> {
        autoreleasepool(|_| {
            #[allow(deprecated)]
            let cursor = NSCursor::currentSystemCursor()?;
            let image = cursor.image();
            let size = image.size();
            let hot = cursor.hotSpot();
            let key = (
                size.width as i64,
                size.height as i64,
                hot.x as i64,
                hot.y as i64,
            );
            state.since_full += 1;
            if state.key == Some(key) && state.since_full < FULL_EVERY {
                return None;
            }
            state.key = Some(key);
            state.since_full = 0;
            // The 2x picture (Retina): sharp on Retina viewers, fine elsewhere.
            let tiff = image.TIFFRepresentation()?;
            let reps = NSBitmapImageRep::imageRepsWithData(&tiff);
            let want = (size.width * 2.0).round() as isize;
            let rep = reps
                .iter()
                .filter_map(|r| r.downcast::<NSBitmapImageRep>().ok())
                .min_by_key(|r| (r.pixelsWide() - want).abs())?;
            let (w, h) = (rep.pixelsWide(), rep.pixelsHigh());
            if w <= 0 || h <= 0 || w as u32 > MAX_SIDE || h as u32 > MAX_SIDE {
                return None;
            }
            let png = unsafe {
                rep.representationUsingType_properties(
                    NSBitmapImageFileType::PNG,
                    &NSDictionary::new(),
                )
            }?
            .to_vec();
            if png == state.last_png {
                return None;
            }
            state.last_png = png.clone();
            let scale = w as f64 / size.width.max(1.0);
            Some(Shape::Image {
                png,
                hot_x: (hot.x * scale).round() as u32,
                hot_y: (hot.y * scale).round() as u32,
                scale: scale as f32,
            })
        })
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod platform {
    use super::Shape;

    #[derive(Default)]
    pub struct State;

    pub fn sample(_: &mut State) -> Option<Shape> {
        None
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    #[test]
    fn samples_the_cursor_once() {
        let mut watcher = super::Watcher::new();
        // Headless test runners may have no cursor at all; only check the shape is sane.
        let first = watcher.changed();
        eprintln!(
            "first sample: {:?}",
            first.as_ref().map(|s| match s {
                super::Shape::Image {
                    png,
                    hot_x,
                    hot_y,
                    scale,
                } => format!("{} bytes hot {hot_x},{hot_y} x{scale}", png.len()),
                super::Shape::Named(n) => n.to_string(),
            })
        );
        if let Some(super::Shape::Image { png, scale, .. }) = first {
            assert!(png.starts_with(b"\x89PNG"));
            assert!(scale >= 1.0);
            assert_eq!(watcher.changed(), None, "unchanged pointer reported twice");
        }
    }
}
