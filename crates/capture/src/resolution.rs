//! Changing the resolution of a shared screen to suit the viewer's window (ADR 0061).
//!
//! Only for computers used remotely ("solo da remoto"): the person at a monitor would see
//! the resolution change. The change is temporary: Windows keeps the registry settings
//! (`CDS_FULLSCREEN`), macOS reverts it when the app quits (`kCGConfigureForAppOnly`), and
//! [`restore`] brings the saved resolution back at the end of the session.

use crate::Result;

/// A display mode, in pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mode {
    pub width: u32,
    pub height: u32,
    /// Pixels per point: 2 for the "HiDPI" modes of macOS, otherwise 1.
    pub scale: u32,
    pub refresh: f64,
}

/// The resolution wanted by the viewer: the device pixels of the area showing the picture
/// and how many of them make one of its points.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Request {
    pub width: u32,
    pub height: u32,
    pub scale: f32,
}

/// The mode to use among `modes`: the largest one that fits in the request, with the same
/// pixels per point as the viewer (text as large as on its own screen) and the refresh rate
/// closest to the current one; without any that fits, the smallest one.
pub fn choose(modes: &[Mode], want: Request, current: Mode) -> Option<Mode> {
    let scale = if want.scale >= 1.5 { 2 } else { 1 };
    let area = |m: &Mode| u64::from(m.width) * u64::from(m.height);
    let refresh_gap = |m: &Mode| (m.refresh - current.refresh).abs();
    let fits = |m: &&Mode| m.width <= want.width && m.height <= want.height;
    let pick = |candidates: Vec<&Mode>| -> Option<Mode> {
        if candidates.iter().any(fits) {
            candidates
                .into_iter()
                .filter(fits)
                .max_by(|a, b| {
                    area(a)
                        .cmp(&area(b))
                        .then(refresh_gap(b).total_cmp(&refresh_gap(a)))
                })
                .copied()
        } else {
            candidates
                .into_iter()
                .min_by(|a, b| {
                    area(a)
                        .cmp(&area(b))
                        .then(refresh_gap(a).total_cmp(&refresh_gap(b)))
                })
                .copied()
        }
    };
    let same_scale: Vec<&Mode> = modes.iter().filter(|m| m.scale == scale).collect();
    if same_scale.is_empty() {
        pick(modes.iter().collect())
    } else {
        pick(same_scale)
    }
}

/// Sets the display (`Display::id`) to the mode closest to `want`; returns the new size, or
/// `None` when it already has it.
pub fn set(display_id: u32, want: Request) -> Result<Option<(u32, u32)>> {
    platform::set(display_id, want)
}

/// Brings the display back to its own resolution.
pub fn restore(display_id: u32) -> Result<()> {
    platform::restore(display_id)
}

#[cfg(target_os = "windows")]
mod platform {
    use windows::Win32::Graphics::Gdi::{
        CDS_FULLSCREEN, CDS_TYPE, ChangeDisplaySettingsExW, DEVMODEW, DISP_CHANGE_SUCCESSFUL,
        DM_DISPLAYFREQUENCY, DM_PELSHEIGHT, DM_PELSWIDTH, ENUM_CURRENT_SETTINGS,
        ENUM_DISPLAY_SETTINGS_FLAGS, ENUM_DISPLAY_SETTINGS_MODE, EnumDisplaySettingsExW,
    };
    use windows::core::PCWSTR;

    use super::{Mode, Request, choose};
    use crate::{CaptureError, Result};

    fn mode_at(device: &[u16], index: ENUM_DISPLAY_SETTINGS_MODE) -> Option<DEVMODEW> {
        let mut mode = DEVMODEW {
            dmSize: std::mem::size_of::<DEVMODEW>() as u16,
            ..Default::default()
        };
        unsafe {
            EnumDisplaySettingsExW(
                PCWSTR(device.as_ptr()),
                index,
                &mut mode,
                ENUM_DISPLAY_SETTINGS_FLAGS(0),
            )
        }
        .as_bool()
        .then_some(mode)
    }

    fn describe(mode: &DEVMODEW) -> Mode {
        Mode {
            width: mode.dmPelsWidth,
            height: mode.dmPelsHeight,
            scale: 1,
            refresh: f64::from(mode.dmDisplayFrequency),
        }
    }

    pub fn set(display_id: u32, want: Request) -> Result<Option<(u32, u32)>> {
        let device = crate::windows::device_name(display_id)?;
        let current = mode_at(&device, ENUM_CURRENT_SETTINGS)
            .ok_or_else(|| CaptureError::Backend("risoluzione attuale non leggibile".into()))?;
        let modes: Vec<DEVMODEW> = (0..)
            .map_while(|i| mode_at(&device, ENUM_DISPLAY_SETTINGS_MODE(i)))
            .filter(|m| m.dmBitsPerPel == 32)
            .collect();
        let described: Vec<Mode> = modes.iter().map(describe).collect();
        let Some(chosen) = choose(&described, want, describe(&current)) else {
            return Ok(None);
        };
        if (chosen.width, chosen.height) == (current.dmPelsWidth, current.dmPelsHeight) {
            return Ok(None);
        }
        let mut mode = modes[described.iter().position(|m| *m == chosen).unwrap()];
        mode.dmFields = DM_PELSWIDTH | DM_PELSHEIGHT | DM_DISPLAYFREQUENCY;
        // Temporary: the registry keeps the user's resolution, `restore` goes back to it.
        let status = unsafe {
            ChangeDisplaySettingsExW(
                PCWSTR(device.as_ptr()),
                Some(&mode),
                None,
                CDS_FULLSCREEN,
                None,
            )
        };
        if status != DISP_CHANGE_SUCCESSFUL {
            return Err(CaptureError::Backend(format!(
                "Windows ha rifiutato {}×{} (codice {})",
                chosen.width, chosen.height, status.0
            )));
        }
        Ok(Some((chosen.width, chosen.height)))
    }

    pub fn restore(display_id: u32) -> Result<()> {
        let device = crate::windows::device_name(display_id)?;
        let status = unsafe {
            ChangeDisplaySettingsExW(PCWSTR(device.as_ptr()), None, None, CDS_TYPE(0), None)
        };
        if status != DISP_CHANGE_SUCCESSFUL {
            return Err(CaptureError::Backend(format!(
                "risoluzione non ripristinata (codice {})",
                status.0
            )));
        }
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use std::ffi::c_void;

    use super::{Mode, Request, choose};
    use crate::{CaptureError, Result};

    type CFTypeRef = *const c_void;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFBooleanTrue: CFTypeRef;
        static kCFTypeDictionaryKeyCallBacks: c_void;
        static kCFTypeDictionaryValueCallBacks: c_void;
        fn CFDictionaryCreate(
            allocator: CFTypeRef,
            keys: *const CFTypeRef,
            values: *const CFTypeRef,
            count: isize,
            key_callbacks: *const c_void,
            value_callbacks: *const c_void,
        ) -> CFTypeRef;
        fn CFArrayGetCount(array: CFTypeRef) -> isize;
        fn CFArrayGetValueAtIndex(array: CFTypeRef, index: isize) -> CFTypeRef;
        fn CFRelease(object: CFTypeRef);
    }

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        static kCGDisplayShowDuplicateLowResolutionModes: CFTypeRef;
        fn CGDisplayCopyAllDisplayModes(display: u32, options: CFTypeRef) -> CFTypeRef;
        fn CGDisplayCopyDisplayMode(display: u32) -> CFTypeRef;
        fn CGDisplayModeGetWidth(mode: CFTypeRef) -> usize;
        fn CGDisplayModeGetHeight(mode: CFTypeRef) -> usize;
        fn CGDisplayModeGetPixelWidth(mode: CFTypeRef) -> usize;
        fn CGDisplayModeGetPixelHeight(mode: CFTypeRef) -> usize;
        fn CGDisplayModeGetRefreshRate(mode: CFTypeRef) -> f64;
        fn CGDisplayModeIsUsableForDesktopGUI(mode: CFTypeRef) -> bool;
        fn CGBeginDisplayConfiguration(config: *mut CFTypeRef) -> i32;
        fn CGConfigureDisplayWithDisplayMode(
            config: CFTypeRef,
            display: u32,
            mode: CFTypeRef,
            options: CFTypeRef,
        ) -> i32;
        fn CGCompleteDisplayConfiguration(config: CFTypeRef, option: u32) -> i32;
        fn CGCancelDisplayConfiguration(config: CFTypeRef) -> i32;
        fn CGRestorePermanentDisplayConfiguration();
    }

    /// `kCGConfigureForAppOnly`: undone when the app quits.
    const CONFIGURE_FOR_APP_ONLY: u32 = 0;

    fn describe(mode: CFTypeRef) -> Mode {
        unsafe {
            let points = CGDisplayModeGetWidth(mode).max(1);
            let width = CGDisplayModeGetPixelWidth(mode);
            Mode {
                width: width as u32,
                height: CGDisplayModeGetPixelHeight(mode) as u32,
                scale: if width >= points * 2 { 2 } else { 1 },
                refresh: CGDisplayModeGetRefreshRate(mode),
            }
        }
    }

    pub fn set(display_id: u32, want: Request) -> Result<Option<(u32, u32)>> {
        unsafe {
            let current = CGDisplayCopyDisplayMode(display_id);
            if current.is_null() {
                return Err(CaptureError::DisplayNotFound(display_id));
            }
            let current_mode = describe(current);
            let current_points = (
                CGDisplayModeGetWidth(current),
                CGDisplayModeGetHeight(current),
            );
            CFRelease(current);
            // With the HiDPI modes too ("duplicate low resolution" ones).
            let key = kCGDisplayShowDuplicateLowResolutionModes;
            let value = kCFBooleanTrue;
            let options = CFDictionaryCreate(
                std::ptr::null(),
                &key,
                &value,
                1,
                &raw const kCFTypeDictionaryKeyCallBacks,
                &raw const kCFTypeDictionaryValueCallBacks,
            );
            let list = CGDisplayCopyAllDisplayModes(display_id, options);
            if !options.is_null() {
                CFRelease(options);
            }
            if list.is_null() {
                return Err(CaptureError::Backend(
                    "modalità dello schermo non leggibili".into(),
                ));
            }
            let handles: Vec<CFTypeRef> = (0..CFArrayGetCount(list))
                .map(|i| CFArrayGetValueAtIndex(list, i))
                .filter(|&m| CGDisplayModeIsUsableForDesktopGUI(m))
                .collect();
            let modes: Vec<Mode> = handles.iter().map(|&m| describe(m)).collect();
            let result = match choose(&modes, want, current_mode) {
                Some(chosen)
                    if (chosen.width, chosen.height, chosen.scale)
                        != (current_mode.width, current_mode.height, current_mode.scale) =>
                {
                    let handle = handles[modes.iter().position(|m| *m == chosen).unwrap()];
                    apply(display_id, handle).map(|()| Some((chosen.width, chosen.height)))
                }
                _ => Ok(None),
            };
            CFRelease(list);
            tracing::debug!(?current_points, ?result, "display mode request");
            result
        }
    }

    unsafe fn apply(display_id: u32, mode: CFTypeRef) -> Result<()> {
        unsafe {
            let mut config: CFTypeRef = std::ptr::null();
            if CGBeginDisplayConfiguration(&mut config) != 0 {
                return Err(CaptureError::Backend(
                    "configurazione schermi non avviata".into(),
                ));
            }
            let status =
                CGConfigureDisplayWithDisplayMode(config, display_id, mode, std::ptr::null());
            if status != 0 {
                CGCancelDisplayConfiguration(config);
                return Err(CaptureError::Backend(format!(
                    "modalità rifiutata (codice {status})"
                )));
            }
            match CGCompleteDisplayConfiguration(config, CONFIGURE_FOR_APP_ONLY) {
                0 => Ok(()),
                status => Err(CaptureError::Backend(format!(
                    "modalità non applicata (codice {status})"
                ))),
            }
        }
    }

    pub fn restore(_display_id: u32) -> Result<()> {
        // Every display back to the user's settings (only the shared one was changed).
        unsafe { CGRestorePermanentDisplayConfiguration() };
        Ok(())
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod platform {
    use super::Request;
    use crate::{CaptureError, Result};

    pub fn set(_display_id: u32, _want: Request) -> Result<Option<(u32, u32)>> {
        Err(CaptureError::Backend(
            "non disponibile su questo sistema".into(),
        ))
    }

    pub fn restore(_display_id: u32) -> Result<()> {
        Ok(())
    }
}

/// Whether this system can change the resolution of a shared screen.
pub const SUPPORTED: bool = cfg!(any(target_os = "windows", target_os = "macos"));

#[cfg(test)]
mod tests {
    use super::*;

    fn mode(width: u32, height: u32, scale: u32) -> Mode {
        Mode {
            width,
            height,
            scale,
            refresh: 60.0,
        }
    }

    const CURRENT: Mode = Mode {
        width: 1024,
        height: 768,
        scale: 1,
        refresh: 60.0,
    };

    #[test]
    fn largest_mode_that_fits() {
        let modes = [
            mode(1024, 768, 1),
            mode(1920, 1080, 1),
            mode(2560, 1440, 1),
            mode(1600, 900, 1),
        ];
        let want = Request {
            width: 2000,
            height: 1200,
            scale: 1.0,
        };
        assert_eq!(choose(&modes, want, CURRENT), Some(mode(1920, 1080, 1)));
    }

    #[test]
    fn smallest_mode_when_none_fits() {
        let modes = [mode(1920, 1080, 1), mode(1280, 720, 1)];
        let want = Request {
            width: 800,
            height: 600,
            scale: 1.0,
        };
        assert_eq!(choose(&modes, want, CURRENT), Some(mode(1280, 720, 1)));
    }

    #[test]
    fn same_pixels_per_point_as_the_viewer() {
        let modes = [
            mode(2880, 1800, 2),
            mode(2880, 1800, 1),
            mode(1440, 900, 1),
            mode(2560, 1600, 2),
        ];
        let retina = Request {
            width: 2900,
            height: 1800,
            scale: 2.0,
        };
        assert_eq!(choose(&modes, retina, CURRENT), Some(mode(2880, 1800, 2)));
        let plain = Request {
            width: 2900,
            height: 1800,
            scale: 1.0,
        };
        assert_eq!(choose(&modes, plain, CURRENT), Some(mode(2880, 1800, 1)));
    }

    #[test]
    fn refresh_rate_closest_to_the_current_one() {
        let fast = Mode {
            refresh: 144.0,
            ..mode(1920, 1080, 1)
        };
        let modes = [fast, mode(1920, 1080, 1)];
        let want = Request {
            width: 1920,
            height: 1080,
            scale: 1.0,
        };
        assert_eq!(choose(&modes, want, CURRENT), Some(mode(1920, 1080, 1)));
    }
}
