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
}

/// True when the Mac has no active display at all (no monitor connected, lid closed
/// without one...), i.e. nothing to capture without a virtual one.
pub fn no_monitor() -> bool {
    let mut count = 0u32;
    // SAFETY: with a null list, CoreGraphics only writes the number of active displays.
    let error = unsafe { CGGetActiveDisplayList(0, std::ptr::null_mut(), &mut count) };
    error == 0 && count == 0
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

/// The virtual display shared by the sessions that need one, created on first use and
/// removed when the last of them drops it.
pub fn shared() -> Result<Arc<VirtualDisplay>> {
    static CURRENT: Mutex<Weak<VirtualDisplay>> = Mutex::new(Weak::new());
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
        let _: () = msg_send![&descriptor, setMaxPixelsWide: WIDTH];
        let _: () = msg_send![&descriptor, setMaxPixelsHigh: HEIGHT];
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
        let allocated: Allocated<AnyObject> = msg_send![class(c"CGVirtualDisplayMode")?, alloc];
        let mode: Option<Retained<AnyObject>> = msg_send![
            allocated,
            initWithWidth: WIDTH,
            height: HEIGHT,
            refreshRate: 60.0f64
        ];
        let mode = mode.ok_or_else(|| unsupported("CGVirtualDisplayMode"))?;
        let modes = NSArray::from_retained_slice(&[mode]);
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
        })
    }
}
