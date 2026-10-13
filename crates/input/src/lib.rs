//! Replays a remote viewer's mouse and keyboard on this machine.
//!
//! Injection runs on a dedicated thread (the OS handles are not `Send` everywhere); the
//! async side only queues events. Keys and buttons still held when the session ends are
//! released, so a dropped connection never leaves e.g. Shift stuck down.

mod keymap;

use std::collections::HashSet;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::JoinHandle;

use anyhow::{Context, Result, anyhow};
#[cfg(target_os = "macos")]
use enigo::Coordinate;
#[cfg(not(target_os = "windows"))]
use enigo::Keyboard;
use enigo::{Axis, Button, Direction, Enigo, Mouse, Settings};
use protocol::{InputEvent, MouseButton};

pub use keymap::{KeyCodes, lookup};

enum Msg {
    Event(InputEvent),
    Area(Area),
    Stop,
}

/// Region that normalized mouse coordinates map onto, in the OS input coordinate space
/// (macOS: global points; Windows: virtual-desktop pixels; X11: root pixels).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Area {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Handle to the injection thread. Dropping it releases held keys and stops the thread.
pub struct InputSink {
    tx: mpsc::Sender<Msg>,
    thread: Option<JoinHandle<()>>,
    health: Arc<Health>,
}

/// What happened to the replayed events, for reporting to the user.
#[derive(Default)]
struct Health {
    applied: AtomicU64,
    failed: AtomicU64,
    last_error: Mutex<Option<String>>,
}

/// Snapshot of [`InputSink::health`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputHealth {
    pub applied: u64,
    pub failed: u64,
    pub last_error: Option<String>,
}

impl InputSink {
    /// Starts the injector. Fails if the OS refuses input simulation (on macOS: the app
    /// lacks the Accessibility permission; the system prompt is shown on first use).
    pub fn start() -> Result<Self> {
        let health = Arc::new(Health::default());
        let thread_health = health.clone();
        let (tx, rx) = mpsc::channel::<Msg>();
        let (ready_tx, ready_rx) = mpsc::channel::<Result<()>>();
        let thread = std::thread::Builder::new()
            .name("input-injector".into())
            .spawn(move || {
                let mut replayer = match Replayer::new() {
                    Ok(r) => {
                        let _ = ready_tx.send(Ok(()));
                        r
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                let mut failures = 0u32;
                loop {
                    let event = match rx.recv() {
                        Ok(Msg::Event(event)) => event,
                        Ok(Msg::Area(area)) => {
                            replayer.area = area;
                            continue;
                        }
                        Ok(Msg::Stop) | Err(_) => break,
                    };
                    let result = replayer.apply(&event);
                    match &result {
                        Ok(()) => {
                            thread_health.applied.fetch_add(1, Ordering::Relaxed);
                        }
                        Err(e) => {
                            thread_health.failed.fetch_add(1, Ordering::Relaxed);
                            *thread_health.last_error.lock().unwrap() = Some(format!("{e:#}"));
                        }
                    }
                    if let Err(e) = result {
                        failures += 1;
                        // Mouse moves can fail by the hundred; keep the log readable.
                        if failures <= 5 || failures.is_power_of_two() {
                            tracing::warn!(failures, "input event {event:?} failed: {e:#}");
                        }
                    }
                }
                replayer.release_all();
            })
            .context("spawning input thread")?;
        ready_rx
            .recv()
            .context("input thread exited during startup")??;
        Ok(Self {
            tx,
            thread: Some(thread),
            health,
        })
    }

    pub fn send(&self, event: InputEvent) {
        let _ = self.tx.send(Msg::Event(event));
    }

    /// Counters of applied/failed events and the last error, to tell the user when
    /// remote control silently fails (missing permission, protected window...).
    pub fn health(&self) -> InputHealth {
        InputHealth {
            applied: self.health.applied.load(Ordering::Relaxed),
            failed: self.health.failed.load(Ordering::Relaxed),
            last_error: self.health.last_error.lock().unwrap().clone(),
        }
    }

    /// Maps future mouse positions onto `area` (the display being shared).
    pub fn set_area(&self, area: Area) {
        let _ = self.tx.send(Msg::Area(area));
    }
}

impl Drop for InputSink {
    fn drop(&mut self) {
        let _ = self.tx.send(Msg::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Replayer {
    enigo: Enigo,
    /// Linux: own X11 connection for absolute pointer moves (see `move_to`).
    #[cfg(target_os = "linux")]
    x11: X11Pointer,
    /// Where normalized mouse positions land; the main display until told otherwise.
    area: Area,
    keys_down: HashSet<String>,
    buttons_down: HashSet<MouseButton>,
}

fn button(b: MouseButton) -> Button {
    match b {
        MouseButton::Left => Button::Left,
        MouseButton::Right => Button::Right,
        MouseButton::Middle => Button::Middle,
        MouseButton::Back => Button::Back,
        MouseButton::Forward => Button::Forward,
    }
}

fn direction(down: bool) -> Direction {
    if down {
        Direction::Press
    } else {
        Direction::Release
    }
}

/// Maps a normalized coordinate onto an axis starting at `origin` and `size` units long.
fn scale(v: f32, origin: f64, size: f64) -> i32 {
    let span = (size - 1.0).max(0.0);
    (origin + f64::from(v.clamp(0.0, 1.0)) * span).round() as i32
}

impl Replayer {
    fn new() -> Result<Self> {
        if is_wayland() {
            return Err(anyhow!(
                "sessione Wayland: il controllo remoto non è ancora supportato (usa una sessione Xorg)"
            ));
        }
        let enigo = Enigo::new(&Settings::default()).map_err(|e| match e {
            enigo::NewConError::NoPermission => anyhow!(
                "permesso Accessibilità mancante: concedilo a Kronos Remote Support in Impostazioni di \
                 Sistema > Privacy e sicurezza > Accessibilità"
            ),
            other => anyhow!("impossibile simulare l'input: {other}"),
        })?;
        #[cfg(target_os = "linux")]
        let x11 = X11Pointer::connect()?;
        #[cfg(target_os = "linux")]
        let display = x11.size;
        #[cfg(not(target_os = "linux"))]
        let display = display_size(&enigo)?;
        let area = Area {
            x: 0.0,
            y: 0.0,
            width: f64::from(display.0),
            height: f64::from(display.1),
        };
        Ok(Self {
            enigo,
            #[cfg(target_os = "linux")]
            x11,
            area,
            keys_down: HashSet::new(),
            buttons_down: HashSet::new(),
        })
    }

    fn apply(&mut self, event: &InputEvent) -> Result<()> {
        // Agent (SYSTEM): inject into the desktop receiving input, e.g. a UAC prompt.
        #[cfg(target_os = "windows")]
        windesk::follow_input_desktop();
        match event {
            InputEvent::MouseMove { x, y } => {
                let a = self.area;
                let (px, py) = (scale(*x, a.x, a.width), scale(*y, a.y, a.height));
                self.move_to(px, py)?;
            }
            InputEvent::MouseButton { button: b, down } => {
                self.enigo.button(button(*b), direction(*down))?;
                if *down {
                    self.buttons_down.insert(*b);
                } else {
                    self.buttons_down.remove(b);
                }
            }
            InputEvent::Wheel { dx, dy } => {
                if *dy != 0 {
                    self.enigo.scroll(*dy, Axis::Vertical)?;
                }
                if *dx != 0 {
                    self.enigo.scroll(*dx, Axis::Horizontal)?;
                }
            }
            InputEvent::Key { code, down } => {
                let keys = lookup(code).with_context(|| format!("unknown key {code}"))?;
                self.key(keys, *down)?;
                if *down {
                    self.keys_down.insert(code.clone());
                } else {
                    self.keys_down.remove(code);
                }
            }
        }
        Ok(())
    }

    #[cfg(target_os = "macos")]
    fn move_to(&mut self, x: i32, y: i32) -> Result<()> {
        Ok(self.enigo.move_mouse(x, y, Coordinate::Abs)?)
    }

    /// `SetCursorPos` takes virtual-desktop coordinates, so secondary monitors work too
    /// (enigo's absolute moves only cover the primary monitor).
    #[cfg(target_os = "windows")]
    fn move_to(&mut self, x: i32, y: i32) -> Result<()> {
        unsafe { windows::Win32::UI::WindowsAndMessaging::SetCursorPos(x, y) }?;
        Ok(())
    }

    #[cfg(target_os = "linux")]
    fn move_to(&mut self, x: i32, y: i32) -> Result<()> {
        self.x11.warp(x, y)
    }

    #[cfg(target_os = "macos")]
    fn key(&mut self, keys: KeyCodes, down: bool) -> Result<()> {
        let code = keys.mac.context("key not available on macOS")?;
        Ok(self.enigo.raw(code, direction(down))?)
    }

    #[cfg(target_os = "windows")]
    fn key(&mut self, keys: KeyCodes, down: bool) -> Result<()> {
        let scan = keys.win.context("key not available on Windows")?;
        windows_keys::send_scancode(scan, down)
    }

    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    fn key(&mut self, keys: KeyCodes, down: bool) -> Result<()> {
        let code = keys.evdev.context("key not available on Linux")?;
        // X11 keycodes are evdev codes shifted by 8.
        Ok(self.enigo.raw(code + 8, direction(down))?)
    }

    fn release_all(&mut self) {
        for code in std::mem::take(&mut self.keys_down) {
            if let Some(keys) = lookup(&code) {
                let _ = self.key(keys, false);
            }
        }
        for b in std::mem::take(&mut self.buttons_down) {
            let _ = self.enigo.button(button(b), Direction::Release);
        }
    }
}

/// Size of the area mouse coordinates refer to; must match what the capture streams.
#[cfg(not(target_os = "linux"))]
fn display_size(enigo: &Enigo) -> Result<(i32, i32)> {
    enigo
        .main_display()
        .map_err(|e| anyhow!("dimensioni dello schermo non disponibili: {e}"))
}

/// X11 pointer moves with `WarpPointer` on the root window.
///
/// enigo's absolute XTest motion is tied to an XInput device and got rescaled by the
/// server (x doubled on Xvfb), and its `main_display()` reads the first RandR mode rather
/// than the current one. The root window is also exactly what the capture streams.
#[cfg(target_os = "linux")]
struct X11Pointer {
    conn: x11rb::rust_connection::RustConnection,
    root: u32,
    size: (i32, i32),
}

#[cfg(target_os = "linux")]
impl X11Pointer {
    fn connect() -> Result<Self> {
        use x11rb::connection::Connection;
        let (conn, screen) = x11rb::connect(None).context("connessione a X11")?;
        let root = &conn.setup().roots[screen];
        let (root, size) = (
            root.root,
            (
                i32::from(root.width_in_pixels),
                i32::from(root.height_in_pixels),
            ),
        );
        Ok(Self { conn, root, size })
    }

    fn warp(&self, x: i32, y: i32) -> Result<()> {
        use x11rb::connection::Connection;
        use x11rb::protocol::xproto::ConnectionExt;
        self.conn
            .warp_pointer(x11rb::NONE, self.root, 0, 0, 0, 0, x as i16, y as i16)?;
        self.conn.flush()?;
        Ok(())
    }
}

/// macOS: shows the Accessibility prompt and adds the app to the Accessibility list in
/// System Settings, so the user only has to switch it on. No-op elsewhere.
pub fn prompt_accessibility() {
    #[cfg(target_os = "macos")]
    {
        let _ = Enigo::new(&Settings::default());
    }
}

fn is_wayland() -> bool {
    cfg!(target_os = "linux")
        && (std::env::var("XDG_SESSION_TYPE").is_ok_and(|t| t.eq_ignore_ascii_case("wayland"))
            || (std::env::var_os("WAYLAND_DISPLAY").is_some()
                && std::env::var_os("DISPLAY").is_none()))
}

#[cfg(target_os = "windows")]
mod windows_keys {
    //! Scancode injection with `SendInput`: positional, independent of the host layout.

    use anyhow::{Result, bail};
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP,
        KEYEVENTF_SCANCODE, SendInput, VIRTUAL_KEY,
    };

    pub fn send_scancode(scan: u16, down: bool) -> Result<()> {
        let mut flags = KEYEVENTF_SCANCODE;
        if scan & 0xFF00 == 0xE000 {
            flags |= KEYEVENTF_EXTENDEDKEY;
        }
        if !down {
            flags |= KEYEVENTF_KEYUP;
        }
        let input = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(0),
                    wScan: scan & 0x00FF,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        let sent = unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) };
        if sent != 1 {
            bail!("SendInput failed for scancode {scan:#06x}");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::scale;

    #[test]
    fn normalized_coordinates_map_to_pixels() {
        assert_eq!(scale(0.0, 0.0, 1920.0), 0);
        assert_eq!(scale(1.0, 0.0, 1920.0), 1919);
        assert_eq!(scale(0.5, 0.0, 1081.0), 540);
        assert_eq!(scale(-3.0, 0.0, 100.0), 0);
        assert_eq!(scale(7.0, 0.0, 100.0), 99);
    }

    #[test]
    fn secondary_display_offsets() {
        // A monitor left of the primary one has negative coordinates.
        assert_eq!(scale(0.0, -1920.0, 1920.0), -1920);
        assert_eq!(scale(1.0, -1920.0, 1920.0), -1);
        assert_eq!(scale(0.5, 1512.0, 1920.0), 2472);
    }
}
