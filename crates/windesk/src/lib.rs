//! Windows shows UAC prompts and the lock screen on separate desktops ("Winlogon"), apart
//! from the user's "Default" desktop. A process running as SYSTEM in the user's session (the
//! Kronos agent) can capture and control them, but each thread must first switch to the
//! desktop currently receiving input. Elsewhere, and until [`enable`] is called, this does
//! nothing.

use std::sync::atomic::{AtomicBool, Ordering};

static ENABLED: AtomicBool = AtomicBool::new(false);

/// Turns desktop following on for the whole process (the agent calls it at start).
pub fn enable() {
    ENABLED.store(true, Ordering::Relaxed);
}

pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// Moves the calling thread to the input desktop if it changed. Returns `true` when the
/// thread switched (callers then recreate desktop-bound resources, e.g. a duplication).
pub fn follow_input_desktop() -> bool {
    if !enabled() {
        return false;
    }
    imp::follow()
}

/// Name of the input desktop ("Default", "Winlogon"...), if it can be opened.
pub fn input_desktop_name() -> Option<String> {
    imp::input_name()
}

#[cfg(target_os = "windows")]
mod imp {
    use std::cell::RefCell;
    use windows_sys::Win32::System::StationsAndDesktops::{
        CloseDesktop, DESKTOP_ACCESS_FLAGS, GetUserObjectInformationW, HDESK, OpenInputDesktop,
        SetThreadDesktop, UOI_NAME,
    };

    /// GENERIC_ALL: needed to read the screen and inject input on that desktop.
    const ACCESS: DESKTOP_ACCESS_FLAGS = 0x1000_0000;

    thread_local! {
        /// Desktop the thread was last moved to (name, handle kept open while in use).
        static CURRENT: RefCell<Option<(String, HDESK)>> = const { RefCell::new(None) };
    }

    fn name_of(desk: HDESK) -> Option<String> {
        let mut buf = [0u16; 256];
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
        if ok == 0 {
            return None;
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        Some(String::from_utf16_lossy(&buf[..len]))
    }

    pub fn input_name() -> Option<String> {
        let desk = unsafe { OpenInputDesktop(0, 0, ACCESS) };
        if desk.is_null() {
            return None;
        }
        let name = name_of(desk);
        unsafe { CloseDesktop(desk) };
        name
    }

    pub fn follow() -> bool {
        let desk = unsafe { OpenInputDesktop(0, 0, ACCESS) };
        if desk.is_null() {
            return false;
        }
        let Some(name) = name_of(desk) else {
            unsafe { CloseDesktop(desk) };
            return false;
        };
        CURRENT.with(|current| {
            let mut current = current.borrow_mut();
            if current.as_ref().is_some_and(|(n, _)| *n == name) {
                unsafe { CloseDesktop(desk) };
                return false;
            }
            if unsafe { SetThreadDesktop(desk) } == 0 {
                unsafe { CloseDesktop(desk) };
                return false;
            }
            if let Some((_, old)) = current.replace((name, desk)) {
                unsafe { CloseDesktop(old) };
            }
            true
        })
    }
}

#[cfg(not(target_os = "windows"))]
mod imp {
    pub fn input_name() -> Option<String> {
        None
    }

    pub fn follow() -> bool {
        false
    }
}
