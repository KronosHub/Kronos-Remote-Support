//! Blocks this computer's own mouse and keyboard while a viewer controls it (the viewer's
//! choice), letting through only the events we inject.
//!
//! - macOS: an event tap at the HID level drops every keyboard and mouse event that does not
//!   carry [`MARK`] (set on the events enigo posts for us). Needs the Accessibility
//!   permission, which remote control already requires.
//! - Windows: low-level keyboard and mouse hooks drop events not flagged as injected.
//! - Linux: not supported.
//!
//! Way out for the person at the computer: Esc pressed three times within two seconds
//! releases the block (`on_released` is called). The block also ends with the session, and
//! with the process if it crashes (taps and hooks belong to it).

use std::time::{Duration, Instant};

use anyhow::Result;

/// Value of `EVENT_SOURCE_USER_DATA` on the events enigo posts for us (macOS).
pub const MARK: i64 = 0x4B52_4F4E; // "KRON"

/// Physical Esc presses that release the block, and the window they must fall in.
const ESCAPES: usize = 3;
const ESCAPE_WINDOW: Duration = Duration::from_secs(2);

/// Counts physical Esc presses; true when the block must be released.
#[derive(Default)]
struct EscapeCounter(Vec<Instant>);

impl EscapeCounter {
    fn pressed(&mut self, now: Instant) -> bool {
        self.0.retain(|t| now.duration_since(*t) < ESCAPE_WINDOW);
        self.0.push(now);
        self.0.len() >= ESCAPES
    }
}

/// Active while held; dropping it gives the mouse and keyboard back.
pub struct LocalBlock {
    _inner: imp::Inner,
}

impl LocalBlock {
    /// Starts blocking. `on_released` runs (once, on another thread) if the local user
    /// escapes with Esc ×3.
    pub fn start(on_released: impl FnOnce() + Send + 'static) -> Result<Self> {
        Ok(Self {
            _inner: imp::Inner::start(Box::new(on_released))?,
        })
    }
}

pub const fn supported() -> bool {
    cfg!(any(target_os = "macos", target_os = "windows"))
}

type OnReleased = Box<dyn FnOnce() + Send>;

#[cfg(target_os = "macos")]
mod imp {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex, mpsc};
    use std::thread::JoinHandle;
    use std::time::{Duration, Instant};

    use anyhow::{Result, anyhow};
    use core_foundation::runloop::{CFRunLoop, kCFRunLoopCommonModes, kCFRunLoopDefaultMode};
    use core_graphics::event::{
        CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventType,
        CallbackResult, EventField,
    };

    use super::{EscapeCounter, MARK, OnReleased};

    /// Esc on Mac keyboards.
    const ESCAPE: i64 = 53;

    const BLOCKED: [CGEventType; 13] = [
        CGEventType::KeyDown,
        CGEventType::KeyUp,
        CGEventType::FlagsChanged,
        CGEventType::MouseMoved,
        CGEventType::LeftMouseDown,
        CGEventType::LeftMouseUp,
        CGEventType::LeftMouseDragged,
        CGEventType::RightMouseDown,
        CGEventType::RightMouseUp,
        CGEventType::RightMouseDragged,
        CGEventType::OtherMouseDown,
        CGEventType::OtherMouseUp,
        CGEventType::ScrollWheel,
    ];

    pub struct Inner {
        stop: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
    }

    impl Inner {
        pub fn start(on_released: OnReleased) -> Result<Self> {
            let stop = Arc::new(AtomicBool::new(false));
            let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
            let thread_stop = stop.clone();
            let thread = std::thread::Builder::new()
                .name("input-block".into())
                .spawn(move || run(thread_stop, on_released, ready_tx))?;
            match ready_rx.recv() {
                Ok(Ok(())) => Ok(Self {
                    stop,
                    thread: Some(thread),
                }),
                Ok(Err(e)) => Err(anyhow!(e)),
                Err(_) => Err(anyhow!("the input block thread ended")),
            }
        }
    }

    fn run(
        stop: Arc<AtomicBool>,
        on_released: OnReleased,
        ready: mpsc::Sender<Result<(), String>>,
    ) {
        let released = Arc::new(Mutex::new(Some(on_released)));
        let escapes = Mutex::new(EscapeCounter::default());
        let callback_stop = stop.clone();
        let tap = CGEventTap::new(
            CGEventTapLocation::HID,
            CGEventTapPlacement::HeadInsertEventTap,
            CGEventTapOptions::Default,
            BLOCKED.to_vec(),
            move |_proxy, kind, event| {
                if callback_stop.load(Ordering::Relaxed)
                    || event.get_integer_value_field(EventField::EVENT_SOURCE_USER_DATA) == MARK
                {
                    return CallbackResult::Keep;
                }
                if matches!(kind, CGEventType::KeyDown)
                    && event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE) == ESCAPE
                    && escapes.lock().unwrap().pressed(Instant::now())
                {
                    callback_stop.store(true, Ordering::Relaxed);
                    if let Some(on_released) = released.lock().unwrap().take() {
                        std::thread::spawn(on_released);
                    }
                    tracing::info!("input block released by the local user (Esc ×3)");
                }
                CallbackResult::Drop
            },
        );
        let Ok(tap) = tap else {
            let _ = ready.send(Err(
                "impossibile bloccare mouse e tastiera (permesso Accessibilità?)".into(),
            ));
            return;
        };
        let Ok(source) = tap.mach_port().create_runloop_source(0) else {
            let _ = ready.send(Err("impossibile bloccare mouse e tastiera".into()));
            return;
        };
        CFRunLoop::get_current().add_source(&source, unsafe { kCFRunLoopCommonModes });
        tap.enable();
        let _ = ready.send(Ok(()));
        tracing::info!("local input blocked");
        while !stop.load(Ordering::Relaxed) {
            CFRunLoop::run_in_mode(
                unsafe { kCFRunLoopDefaultMode },
                Duration::from_millis(200),
                false,
            );
            // macOS disables a tap that was slow or after secure input: turn it back on.
            tap.enable();
        }
        tracing::info!("local input unblocked");
    }

    impl Drop for Inner {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }
}

#[cfg(target_os = "windows")]
mod imp {
    use std::sync::Mutex;
    use std::sync::mpsc;
    use std::thread::JoinHandle;
    use std::time::Instant;

    use anyhow::{Result, anyhow};
    use windows::Win32::Foundation::{LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::Threading::GetCurrentThreadId;
    use windows::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, GetMessageW, KBDLLHOOKSTRUCT, LLKHF_INJECTED, LLMHF_INJECTED, MSG,
        MSLLHOOKSTRUCT, PostThreadMessageW, SetWindowsHookExW, UnhookWindowsHookEx, WH_KEYBOARD_LL,
        WH_MOUSE_LL, WM_KEYDOWN, WM_QUIT, WM_SYSKEYDOWN,
    };

    use super::{EscapeCounter, OnReleased};

    /// Hook procedures get no context: the state of the (single) active block.
    struct State {
        escapes: EscapeCounter,
        on_released: Option<OnReleased>,
        released: bool,
    }

    static STATE: Mutex<Option<State>> = Mutex::new(None);

    /// True when the event must go through: injected, or the block was released.
    fn pass(injected: bool) -> bool {
        injected
            || STATE
                .lock()
                .unwrap()
                .as_ref()
                .is_none_or(|state| state.released)
    }

    unsafe extern "system" fn keyboard(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0 {
            let info = unsafe { &*(lparam.0 as *const KBDLLHOOKSTRUCT) };
            let injected = info.flags.0 & LLKHF_INJECTED.0 != 0;
            if !pass(injected) {
                let down = matches!(wparam.0 as u32, WM_KEYDOWN | WM_SYSKEYDOWN);
                if down && info.vkCode == u32::from(VK_ESCAPE.0) {
                    let mut state = STATE.lock().unwrap();
                    if let Some(state) = state.as_mut()
                        && state.escapes.pressed(Instant::now())
                    {
                        state.released = true;
                        if let Some(on_released) = state.on_released.take() {
                            std::thread::spawn(on_released);
                        }
                        tracing::info!("input block released by the local user (Esc ×3)");
                    }
                }
                return LRESULT(1);
            }
        }
        unsafe { CallNextHookEx(None, code, wparam, lparam) }
    }

    unsafe extern "system" fn mouse(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        if code >= 0 {
            let info = unsafe { &*(lparam.0 as *const MSLLHOOKSTRUCT) };
            if !pass(info.flags & LLMHF_INJECTED != 0) {
                return LRESULT(1);
            }
        }
        unsafe { CallNextHookEx(None, code, wparam, lparam) }
    }

    pub struct Inner {
        thread_id: u32,
        thread: Option<JoinHandle<()>>,
    }

    impl Inner {
        pub fn start(on_released: OnReleased) -> Result<Self> {
            {
                let mut state = STATE.lock().unwrap();
                if state.is_some() {
                    anyhow::bail!("mouse e tastiera sono già bloccati");
                }
                *state = Some(State {
                    escapes: EscapeCounter::default(),
                    on_released: Some(on_released),
                    released: false,
                });
            }
            let (ready_tx, ready_rx) = mpsc::channel::<Result<u32, String>>();
            let thread = std::thread::Builder::new()
                .name("input-block".into())
                .spawn(move || unsafe {
                    // Hooks call back on the thread that installed them, via its message loop.
                    let keyboard = SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard), None, 0);
                    let mouse = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse), None, 0);
                    let (Ok(keyboard), Ok(mouse)) = (keyboard, mouse) else {
                        let _ = ready_tx.send(Err("impossibile bloccare mouse e tastiera".into()));
                        return;
                    };
                    let _ = ready_tx.send(Ok(GetCurrentThreadId()));
                    tracing::info!("local input blocked");
                    let mut msg = MSG::default();
                    while GetMessageW(&mut msg, None, 0, 0).0 > 0 {}
                    let _ = UnhookWindowsHookEx(keyboard);
                    let _ = UnhookWindowsHookEx(mouse);
                    tracing::info!("local input unblocked");
                })?;
            match ready_rx.recv() {
                Ok(Ok(thread_id)) => Ok(Self {
                    thread_id,
                    thread: Some(thread),
                }),
                result => {
                    *STATE.lock().unwrap() = None;
                    let _ = thread.join();
                    Err(match result {
                        Ok(Err(e)) => anyhow!(e),
                        _ => anyhow!("the input block thread ended"),
                    })
                }
            }
        }
    }

    impl Drop for Inner {
        fn drop(&mut self) {
            unsafe {
                let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
            }
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
            *STATE.lock().unwrap() = None;
        }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod imp {
    use anyhow::Result;

    use super::OnReleased;

    pub struct Inner;

    impl Inner {
        pub fn start(_on_released: OnReleased) -> Result<Self> {
            anyhow::bail!("bloccare mouse e tastiera non è supportato su Linux")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn three_quick_escapes_release() {
        let mut counter = EscapeCounter::default();
        let t = Instant::now();
        assert!(!counter.pressed(t));
        assert!(!counter.pressed(t + Duration::from_millis(500)));
        assert!(counter.pressed(t + Duration::from_millis(900)));
    }

    #[test]
    fn slow_escapes_do_not() {
        let mut counter = EscapeCounter::default();
        let t = Instant::now();
        assert!(!counter.pressed(t));
        assert!(!counter.pressed(t + Duration::from_millis(1500)));
        assert!(!counter.pressed(t + Duration::from_millis(3000)));
        assert!(!counter.pressed(t + Duration::from_millis(4600)));
    }
}
