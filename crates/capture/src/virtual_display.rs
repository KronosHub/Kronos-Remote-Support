//! macOS: a virtual monitor for a Mac without one (e.g. a Mac mini with nothing plugged
//! in), which otherwise has no screen to capture. Made with CoreGraphics'
//! `CGVirtualDisplay` (private, macOS 11+; DeskPad and BetterDisplay use it too): every
//! selector is checked first, so a macOS without it reports an error instead of crashing.
//!
//! The display exists while a [`VirtualDisplay`] is alive; sessions share one through
//! [`shared`].

use std::ffi::{c_char, c_void};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

use objc2::rc::{Allocated, Retained};
use objc2::runtime::{AnyClass, AnyObject, Bool, Sel};
use objc2::{Encode, Encoding, msg_send, sel};
use objc2_foundation::{NSArray, NSString};

use crate::{CaptureError, Result};

/// Size of the virtual monitor: what most remote screens are viewed at.
pub const WIDTH: u32 = 1920;
pub const HEIGHT: u32 = 1080;
/// Every size it offers, smallest first: for a viewer whose window asks for one (ADR 0061)
/// or a size fixed in the preferences (ADR 0065). It starts at `WIDTH`×`HEIGHT`.
pub const SIZES: [(u32, u32); 10] = [
    (1280, 720),
    (1280, 800),
    (1440, 900),
    (1600, 900),
    (1680, 1050),
    (WIDTH, HEIGHT),
    (1920, 1200),
    (2048, 1152),
    (2560, 1440),
    (2560, 1600),
];
/// Largest of all the modes.
const MAX_PIXELS: (u32, u32) = (2560, 1600);
/// How long to wait for the new display to become capturable.
const APPEAR_TIMEOUT: Duration = Duration::from_secs(5);

#[repr(C)]
#[derive(Clone, Copy)]
struct CGSize {
    width: f64,
    height: f64,
}

// SAFETY: same layout and encoding as CoreGraphics' CGSize (two CGFloat = f64 on 64-bit).
unsafe impl Encode for CGSize {
    const ENCODING: Encoding = Encoding::Struct("CGSize", &[f64::ENCODING, f64::ENCODING]);
}

unsafe extern "C" {
    fn dispatch_queue_create(label: *const c_char, attr: *const c_void) -> *mut AnyObject;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGGetActiveDisplayList(max: u32, displays: *mut u32, count: *mut u32) -> i32;
    fn CGGetOnlineDisplayList(max: u32, displays: *mut u32, count: *mut u32) -> i32;
    fn CGMainDisplayID() -> u32;
    fn CGDisplayIsBuiltin(display: u32) -> u32;
    fn CGDisplayMirrorsDisplay(display: u32) -> u32;
    fn CGDisplayVendorNumber(display: u32) -> u32;
    fn CGDisplayModelNumber(display: u32) -> u32;
    fn CGDisplaySerialNumber(display: u32) -> u32;
    fn CGDisplayIsAsleep(display: u32) -> u32;
    fn CGDisplayPixelsWide(display: u32) -> usize;
    fn CGDisplayPixelsHigh(display: u32) -> usize;
    fn CGDisplayCopyDisplayMode(display: u32) -> *const c_void;
    fn CGDisplayModeGetPixelWidth(mode: *const c_void) -> usize;
    fn CGDisplayModeGetPixelHeight(mode: *const c_void) -> usize;
    fn CGDisplayModeRelease(mode: *const c_void);
    fn CGBeginDisplayConfiguration(config: *mut *const c_void) -> i32;
    fn CGConfigureDisplayMirrorOfDisplay(config: *const c_void, display: u32, master: u32) -> i32;
    fn CGConfigureDisplayOrigin(config: *const c_void, display: u32, x: i32, y: i32) -> i32;
    fn CGCompleteDisplayConfiguration(config: *const c_void, option: u32) -> i32;
    fn CGCancelDisplayConfiguration(config: *const c_void) -> i32;
}

/// `kCGConfigureForSession`: the change lasts at most until logout.
const CONFIGURE_FOR_SESSION: u32 = 1;
/// `kCGNullDirectDisplay`: as a mirror master, ends the mirroring.
const NO_DISPLAY: u32 = 0;

/// Whether the Mac has a built-in screen in use (a MacBook, an iMac): its owner may be in
/// front of it, so the virtual display never takes it over.
pub fn has_builtin() -> bool {
    // SAFETY: plain queries on display ids CoreGraphics just listed.
    display_list(false).is_some_and(|ids| {
        ids.into_iter()
            .any(|id| unsafe { CGDisplayIsBuiltin(id) } != 0)
    })
}

/// Writes to the log what CoreGraphics says about each display: on a Mac seen only
/// remotely, the way to tell what macOS is drawing on.
pub fn log_displays() {
    let main = unsafe { CGMainDisplayID() };
    let active = display_list(false).unwrap_or_default();
    for id in display_list(true).unwrap_or_default() {
        // SAFETY: queries on a listed display id; the mode is released after use.
        unsafe {
            let mode = CGDisplayCopyDisplayMode(id);
            let mode_pixels = if mode.is_null() {
                (0, 0)
            } else {
                let pixels = (
                    CGDisplayModeGetPixelWidth(mode),
                    CGDisplayModeGetPixelHeight(mode),
                );
                CGDisplayModeRelease(mode);
                pixels
            };
            tracing::info!(
                id,
                vendor = format_args!("{:#x}", CGDisplayVendorNumber(id)),
                model = format_args!("{:#x}", CGDisplayModelNumber(id)),
                serial = format_args!("{:#x}", CGDisplaySerialNumber(id)),
                builtin = CGDisplayIsBuiltin(id) != 0,
                main = id == main,
                active = active.contains(&id),
                asleep = CGDisplayIsAsleep(id) != 0,
                mirrors = CGDisplayMirrorsDisplay(id),
                points = ?(CGDisplayPixelsWide(id), CGDisplayPixelsHigh(id)),
                pixels = ?mode_pixels,
                virtual_display = current().is_some_and(|d| d.id == id),
                "display"
            );
        }
    }
}

/// Applies one display configuration for the login session, or cancels it on error.
fn configure(apply: impl FnOnce(*const c_void) -> i32) -> Result<()> {
    let mut config = std::ptr::null();
    // SAFETY: the configuration is completed or cancelled exactly once.
    unsafe {
        let error = CGBeginDisplayConfiguration(&mut config);
        if error != 0 || config.is_null() {
            return Err(CaptureError::Backend(format!(
                "configurazione schermi non avviata ({error})"
            )));
        }
        let error = apply(config);
        if error != 0 {
            CGCancelDisplayConfiguration(config);
            return Err(CaptureError::Backend(format!(
                "configurazione schermi rifiutata ({error})"
            )));
        }
        let error = CGCompleteDisplayConfiguration(config, CONFIGURE_FOR_SESSION);
        if error != 0 {
            return Err(CaptureError::Backend(format!(
                "configurazione schermi non applicata ({error})"
            )));
        }
    }
    Ok(())
}

/// The Mac's real monitors (the virtual display left out).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Monitors {
    /// At least one is on and can be captured.
    On,
    /// Connected but all asleep (display sleep): they come back with user activity, and
    /// the login or lock screen shows up on them, not on a virtual display.
    Asleep,
    /// None at all (no monitor connected, lid closed without one...).
    None,
}

const MAX_DISPLAYS: usize = 32;

/// The displays CoreGraphics lists: `online` ones include those asleep, active ones are
/// drawing. `None` when the call fails.
fn display_list(online: bool) -> Option<Vec<u32>> {
    let mut ids = [0u32; MAX_DISPLAYS];
    let mut count = 0u32;
    // SAFETY: the list has room for `MAX_DISPLAYS` ids and CoreGraphics writes at most that.
    let error = unsafe {
        if online {
            CGGetOnlineDisplayList(MAX_DISPLAYS as u32, ids.as_mut_ptr(), &mut count)
        } else {
            CGGetActiveDisplayList(MAX_DISPLAYS as u32, ids.as_mut_ptr(), &mut count)
        }
    };
    (error == 0).then(|| ids[..(count as usize).min(MAX_DISPLAYS)].to_vec())
}

/// The state of the real monitors; `On` when CoreGraphics cannot tell (nothing virtual is
/// made then).
pub fn monitors() -> Monitors {
    let virtual_id = current().map(|d| d.id);
    let real = |online| {
        display_list(online).map(|ids| ids.into_iter().filter(|&id| Some(id) != virtual_id).count())
    };
    match (real(false), real(true)) {
        (Some(0), Some(0)) => Monitors::None,
        (Some(0), _) => Monitors::Asleep,
        _ => Monitors::On,
    }
}

/// The serial queue the display reports on (created once, never released).
fn queue() -> *mut AnyObject {
    static QUEUE: OnceLock<usize> = OnceLock::new();
    *QUEUE.get_or_init(|| {
        // SAFETY: a static label and the default (serial) attributes.
        let queue = unsafe {
            dispatch_queue_create(c"tech.kronos.virtual-display".as_ptr(), std::ptr::null())
        };
        queue as usize
    }) as *mut AnyObject
}

pub struct VirtualDisplay {
    _display: Retained<AnyObject>,
    id: u32,
    /// The displays made to mirror this one by [`VirtualDisplay::take_over`].
    mirrored: Mutex<Vec<u32>>,
}

// SAFETY: the object is only kept alive and released; CoreGraphics does its work on its
// own queue.
unsafe impl Send for VirtualDisplay {}
unsafe impl Sync for VirtualDisplay {}

impl VirtualDisplay {
    /// The display's id, as listed by [`crate::displays`].
    pub fn id(&self) -> u32 {
        self.id
    }

    /// Makes this the main display, with every other display mirroring it: the desktop,
    /// menu bar and Dock move here. For a Mac used only remotely whose own display does not
    /// capture well (a Mac mini without a monitor can keep a stand-in display that comes
    /// out shrunk and discoloured). Undone when the virtual display goes, at the latest at
    /// logout.
    pub fn take_over(&self) -> Result<()> {
        let mut mirrored = self.mirrored.lock().unwrap();
        let others: Vec<u32> = display_list(false)
            .unwrap_or_default()
            .into_iter()
            // SAFETY: plain query on a listed display id.
            .filter(|&id| id != self.id && unsafe { CGDisplayMirrorsDisplay(id) } != self.id)
            .collect();
        // SAFETY: plain query.
        if others.is_empty() && unsafe { CGMainDisplayID() } == self.id {
            return Ok(());
        }
        let main_only = |config| {
            // SAFETY: a configuration in progress.
            unsafe { CGConfigureDisplayOrigin(config, self.id, 0, 0) }
        };
        let mirror = configure(|config| {
            for &id in &others {
                // SAFETY: a configuration in progress and a display CoreGraphics listed.
                let error = unsafe { CGConfigureDisplayMirrorOfDisplay(config, id, self.id) };
                if error != 0 {
                    return error;
                }
            }
            main_only(config)
        });
        if let Err(e) = mirror {
            // At least the menu bar and new windows come here; the others stay beside it.
            tracing::warn!("displays not mirrored ({e}): virtual display only made the main one");
            configure(main_only)?;
            return Ok(());
        }
        tracing::info!(id = self.id, mirroring = ?others, "virtual display is now the main one");
        mirrored.extend(others);
        Ok(())
    }

    /// Creates a `WIDTH`×`HEIGHT` monitor and waits until it can be captured.
    pub fn create() -> Result<Self> {
        let virtual_display = make()?;
        let deadline = Instant::now() + APPEAR_TIMEOUT;
        while Instant::now() < deadline {
            if crate::displays().is_ok_and(|all| all.iter().any(|d| d.id == virtual_display.id)) {
                tracing::info!(id = virtual_display.id, "virtual display ready");
                return Ok(virtual_display);
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        Err(CaptureError::Backend(
            "lo schermo virtuale non è comparso".into(),
        ))
    }
}

impl Drop for VirtualDisplay {
    fn drop(&mut self) {
        let mirrored = std::mem::take(self.mirrored.get_mut().unwrap());
        if mirrored.is_empty() {
            return;
        }
        // The displays show their own desktop again (macOS would also do it once this one
        // is gone, but not always placing them as before).
        let restored = configure(|config| {
            for &id in &mirrored {
                // SAFETY: a configuration in progress; a display gone meanwhile only fails.
                let error = unsafe { CGConfigureDisplayMirrorOfDisplay(config, id, NO_DISPLAY) };
                if error != 0 {
                    return error;
                }
            }
            0
        });
        if let Err(e) = restored {
            tracing::warn!("displays not restored: {e}");
        }
    }
}

/// The virtual display shared by the sessions that need one, created on first use and
/// removed when the last of them drops it.
static CURRENT: Mutex<Weak<VirtualDisplay>> = Mutex::new(Weak::new());

/// The shared virtual display, if one exists now.
fn current() -> Option<Arc<VirtualDisplay>> {
    CURRENT.lock().unwrap().upgrade()
}

pub fn shared() -> Result<Arc<VirtualDisplay>> {
    let mut current = CURRENT.lock().unwrap();
    if let Some(display) = current.upgrade() {
        return Ok(display);
    }
    let display = Arc::new(VirtualDisplay::create()?);
    *current = Arc::downgrade(&display);
    Ok(display)
}

fn class(name: &std::ffi::CStr) -> Result<&'static AnyClass> {
    AnyClass::get(name).ok_or_else(|| unsupported(&name.to_string_lossy()))
}

fn unsupported(what: &str) -> CaptureError {
    CaptureError::Backend(format!(
        "schermo virtuale non disponibile su questa versione di macOS ({what})"
    ))
}

fn check(object: &AnyObject, selector: Sel) -> Result<()> {
    // SAFETY: NSObject's respondsToSelector:.
    let responds: Bool = unsafe { msg_send![object, respondsToSelector: selector] };
    if responds.as_bool() {
        Ok(())
    } else {
        Err(unsupported(selector.name().to_str().unwrap_or("?")))
    }
}

fn make() -> Result<VirtualDisplay> {
    // SAFETY: each selector is checked with respondsToSelector: before the call, with the
    // argument types of the CoreGraphics declarations (unsigned int, double, CGSize, objects).
    unsafe {
        let descriptor: Retained<AnyObject> = msg_send![class(c"CGVirtualDisplayDescriptor")?, new];
        for selector in [
            sel!(setName:),
            sel!(setMaxPixelsWide:),
            sel!(setMaxPixelsHigh:),
            sel!(setSizeInMillimeters:),
            sel!(setProductID:),
            sel!(setVendorID:),
            sel!(setSerialNum:),
        ] {
            check(&descriptor, selector)?;
        }
        if check(&descriptor, sel!(setDispatchQueue:)).is_ok() {
            let _: () = msg_send![&descriptor, setDispatchQueue: queue()];
        } else {
            check(&descriptor, sel!(setQueue:))?;
            let _: () = msg_send![&descriptor, setQueue: queue()];
        }
        let name = NSString::from_str("Kronos Remote Support");
        let _: () = msg_send![&descriptor, setName: &*name];
        let _: () = msg_send![&descriptor, setMaxPixelsWide: MAX_PIXELS.0];
        let _: () = msg_send![&descriptor, setMaxPixelsHigh: MAX_PIXELS.1];
        // A 24" monitor: macOS derives the default scaling from the physical size.
        let size = CGSize {
            width: 527.0,
            height: 296.0,
        };
        let _: () = msg_send![&descriptor, setSizeInMillimeters: size];
        let _: () = msg_send![&descriptor, setProductID: 0x4b52u32];
        let _: () = msg_send![&descriptor, setVendorID: 0x4b52u32];
        let _: () = msg_send![&descriptor, setSerialNum: 1u32];

        let allocated: Allocated<AnyObject> = msg_send![class(c"CGVirtualDisplay")?, alloc];
        let display: Option<Retained<AnyObject>> =
            msg_send![allocated, initWithDescriptor: &*descriptor];
        let display = display.ok_or_else(|| {
            CaptureError::Backend("macOS non ha creato lo schermo virtuale".into())
        })?;
        check(&display, sel!(applySettings:))?;
        check(&display, sel!(displayID))?;

        let settings: Retained<AnyObject> = msg_send![class(c"CGVirtualDisplaySettings")?, new];
        check(&settings, sel!(setHiDPI:))?;
        check(&settings, sel!(setModes:))?;
        // The first mode is the one it starts with.
        let mut modes = Vec::with_capacity(SIZES.len());
        let others = SIZES.into_iter().filter(|&size| size != (WIDTH, HEIGHT));
        for (width, height) in std::iter::once((WIDTH, HEIGHT)).chain(others) {
            let allocated: Allocated<AnyObject> = msg_send![class(c"CGVirtualDisplayMode")?, alloc];
            let mode: Option<Retained<AnyObject>> = msg_send![
                allocated,
                initWithWidth: width,
                height: height,
                refreshRate: 60.0f64
            ];
            modes.push(mode.ok_or_else(|| unsupported("CGVirtualDisplayMode"))?);
        }
        let modes = NSArray::from_retained_slice(&modes);
        let _: () = msg_send![&settings, setHiDPI: 0u32];
        let _: () = msg_send![&settings, setModes: &*modes];
        let applied: Bool = msg_send![&display, applySettings: &*settings];
        if !applied.as_bool() {
            return Err(CaptureError::Backend(
                "macOS ha rifiutato le impostazioni dello schermo virtuale".into(),
            ));
        }
        let id: u32 = msg_send![&display, displayID];
        if id == 0 {
            return Err(CaptureError::Backend("schermo virtuale senza id".into()));
        }
        Ok(VirtualDisplay {
            _display: display,
            id,
            mirrored: Mutex::new(Vec::new()),
        })
    }
}
