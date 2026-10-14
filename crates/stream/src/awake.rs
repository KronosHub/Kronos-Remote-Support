//! Keeps the host awake while it is being controlled. On macOS injected input does not
//! always count as user activity, so the display went to sleep and the screen locked in the
//! middle of a session.
//!
//! - macOS: an IOKit assertion against idle display sleep for the whole session, plus
//!   "user activity" declared on remote input (resets the screen saver timer too).
//! - Windows: the display and system stay on for the whole session.
//! - Elsewhere: nothing.

use std::time::Duration;

/// How often remote input is reported as user activity, at most.
const ACTIVITY_GAP: Duration = Duration::from_secs(10);

/// Held for the duration of a host session; dropping it lets the machine sleep again.
pub struct KeepAwake(imp::Inner);

impl KeepAwake {
    pub fn new() -> Self {
        Self(imp::Inner::new())
    }

    /// The viewer sent mouse or keyboard input.
    pub fn user_active(&self) {
        self.0.user_active();
    }
}

impl Default for KeepAwake {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use std::ffi::{CStr, c_char, c_void};
    use std::sync::Mutex;
    use std::time::Instant;

    type CFStringRef = *const c_void;

    const UTF8: u32 = 0x0800_0100;
    const ASSERTION_LEVEL_ON: u32 = 255;
    const USER_ACTIVE_LOCAL: u32 = 0;
    const NAME: &CStr = c"Kronos Remote Support: sessione remota";

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithCString(
            alloc: *const c_void,
            text: *const c_char,
            encoding: u32,
        ) -> CFStringRef;
        fn CFRelease(cf: *const c_void);
    }

    #[link(name = "IOKit", kind = "framework")]
    unsafe extern "C" {
        fn IOPMAssertionCreateWithName(
            kind: CFStringRef,
            level: u32,
            name: CFStringRef,
            id: *mut u32,
        ) -> i32;
        fn IOPMAssertionDeclareUserActivity(name: CFStringRef, kind: u32, id: *mut u32) -> i32;
        fn IOPMAssertionRelease(id: u32) -> i32;
    }

    /// Calls `f` with a temporary CFString.
    fn with_cf<T>(text: &CStr, f: impl FnOnce(CFStringRef) -> T) -> Option<T> {
        let cf = unsafe { CFStringCreateWithCString(std::ptr::null(), text.as_ptr(), UTF8) };
        if cf.is_null() {
            return None;
        }
        let result = f(cf);
        unsafe { CFRelease(cf) };
        Some(result)
    }

    pub struct Inner {
        display: Option<u32>,
        /// Assertion id reused by every declaration, and when the last one was made.
        activity: Mutex<(u32, Option<Instant>)>,
    }

    impl Inner {
        pub fn new() -> Self {
            let display = with_cf(c"PreventUserIdleDisplaySleep", |kind| {
                with_cf(NAME, |name| {
                    let mut id = 0u32;
                    let status = unsafe {
                        IOPMAssertionCreateWithName(kind, ASSERTION_LEVEL_ON, name, &mut id)
                    };
                    (status == 0).then_some(id)
                })
            })
            .flatten()
            .flatten();
            if display.is_none() {
                tracing::warn!("cannot keep the display awake during the session");
            }
            Self {
                display,
                activity: Mutex::new((0, None)),
            }
        }

        pub fn user_active(&self) {
            let mut activity = self.activity.lock().unwrap();
            if activity
                .1
                .is_some_and(|t| t.elapsed() < super::ACTIVITY_GAP)
            {
                return;
            }
            activity.1 = Some(Instant::now());
            let id = &mut activity.0;
            with_cf(NAME, |name| unsafe {
                IOPMAssertionDeclareUserActivity(name, USER_ACTIVE_LOCAL, id)
            });
        }
    }

    impl Drop for Inner {
        fn drop(&mut self) {
            if let Some(id) = self.display {
                unsafe { IOPMAssertionRelease(id) };
            }
            let (id, _) = *self.activity.lock().unwrap();
            if id != 0 {
                unsafe { IOPMAssertionRelease(id) };
            }
        }
    }
}

#[cfg(windows)]
mod imp {
    use std::sync::mpsc;

    use windows_sys::Win32::System::Power::{
        ES_CONTINUOUS, ES_DISPLAY_REQUIRED, ES_SYSTEM_REQUIRED, SetThreadExecutionState,
    };

    /// The execution state belongs to a thread: one lives for the session.
    pub struct Inner {
        _stop: mpsc::Sender<()>,
    }

    impl Inner {
        pub fn new() -> Self {
            let (stop, stopped) = mpsc::channel::<()>();
            let spawned = std::thread::Builder::new()
                .name("keep-awake".into())
                .spawn(move || {
                    unsafe {
                        SetThreadExecutionState(
                            ES_CONTINUOUS | ES_DISPLAY_REQUIRED | ES_SYSTEM_REQUIRED,
                        )
                    };
                    // Returns once the sender is dropped.
                    let _ = stopped.recv();
                    unsafe { SetThreadExecutionState(ES_CONTINUOUS) };
                });
            if let Err(e) = spawned {
                tracing::warn!("cannot keep the display awake during the session: {e}");
            }
            Self { _stop: stop }
        }

        /// Injected input already counts as activity on Windows.
        pub fn user_active(&self) {}
    }
}

#[cfg(not(any(target_os = "macos", windows)))]
mod imp {
    pub struct Inner;

    impl Inner {
        pub fn new() -> Self {
            Self
        }

        pub fn user_active(&self) {}
    }
}
