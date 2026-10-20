//! One screen capture per monitor, shared by every session watching it.
//!
//! Windows' desktop duplication refuses a second duplication of the same output in one
//! process, so two people controlling the same PC failed. Capturing once and handing the
//! frames to each session (each keeps its own encoder and adaptive quality) also saves CPU
//! on every platform.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::thread::JoinHandle;
use std::time::Duration;

use crate::{CaptureConfig, CaptureError, Capturer, Frame, Rect};

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Key {
    display_id: Option<u32>,
    max_size: Option<(u32, u32)>,
    max_fps: u32,
    show_cursor: bool,
    native: bool,
}

impl Key {
    fn of(config: &CaptureConfig) -> Self {
        Self {
            display_id: config.display_id,
            max_size: config.max_size,
            max_fps: config.max_fps,
            show_cursor: config.show_cursor,
            native: config.native,
        }
    }
}

/// State shared between the capture thread and the sessions.
struct Shared {
    subscribers: Mutex<Vec<SyncSender<Frame>>>,
    /// Latest frame, handed to sessions that join while the screen is static.
    last: Mutex<Option<Frame>>,
    /// Why the capture stopped, for the sessions.
    error: Mutex<Option<String>>,
    /// Set (with `subscribers` locked) when the capture thread ends.
    ended: AtomicBool,
}

/// A running capture; stops when the last session lets go of it.
struct Hub {
    shared: Arc<Shared>,
    output: (u32, u32),
    display_id: u32,
    bounds: Rect,
    stop: Arc<AtomicBool>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Drop for Hub {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.lock().unwrap().take() {
            let _ = thread.join();
        }
    }
}

fn registry() -> &'static Mutex<HashMap<Key, Weak<Hub>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<Key, Weak<Hub>>>> = OnceLock::new();
    REGISTRY.get_or_init(Default::default)
}

/// Starts capturing (or joins the capture already running) for `config`.
pub fn subscribe(config: &CaptureConfig) -> crate::Result<Box<dyn Capturer>> {
    subscribe_with(config, crate::start)
}

fn subscribe_with(
    config: &CaptureConfig,
    start: fn(&CaptureConfig) -> crate::Result<Box<dyn Capturer>>,
) -> crate::Result<Box<dyn Capturer>> {
    let key = Key::of(config);
    // Held while a capture starts, so two sessions starting together share it.
    let mut registry = registry().lock().unwrap();
    registry.retain(|_, hub| hub.strong_count() > 0);
    // A capture that ended (error) may still be held by sessions about to let it go.
    let hub = match registry
        .get(&key)
        .and_then(Weak::upgrade)
        .filter(|hub| !hub.shared.ended.load(Ordering::Relaxed))
    {
        Some(hub) => hub,
        None => {
            let hub = Arc::new(start_hub(start(config)?));
            registry.insert(key, Arc::downgrade(&hub));
            hub
        }
    };
    drop(registry);

    let (tx, rx) = mpsc::sync_channel(2);
    if let Some(frame) = hub.shared.last.lock().unwrap().clone() {
        let _ = tx.try_send(frame);
    }
    let mut subscribers = hub.shared.subscribers.lock().unwrap();
    // Ended meanwhile: dropping `tx` reports the error to this session at once.
    if !hub.shared.ended.load(Ordering::Relaxed) {
        subscribers.push(tx);
    }
    drop(subscribers);
    Ok(Box::new(Subscriber { hub, frames: rx }))
}

fn start_hub(mut capturer: Box<dyn Capturer>) -> Hub {
    let shared = Arc::new(Shared {
        subscribers: Mutex::new(Vec::new()),
        last: Mutex::new(None),
        error: Mutex::new(None),
        ended: AtomicBool::new(false),
    });
    let stop = Arc::new(AtomicBool::new(false));
    let (output, display_id, bounds) = (
        capturer.output_size(),
        capturer.display_id(),
        capturer.bounds(),
    );
    let thread = {
        let (shared, stop) = (shared.clone(), stop.clone());
        std::thread::Builder::new()
            .name("shared-capture".into())
            .spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    match capturer.next_frame(Duration::from_millis(100)) {
                        Ok(Some(frame)) => {
                            *shared.last.lock().unwrap() = Some(frame.clone());
                            // A session that falls behind skips frames (it then sends a
                            // later one; its encoder never sees a gap in its own stream).
                            shared.subscribers.lock().unwrap().retain(|tx| {
                                !matches!(
                                    tx.try_send(frame.clone()),
                                    Err(TrySendError::Disconnected(_))
                                )
                            });
                        }
                        Ok(None) => {}
                        Err(e) => {
                            *shared.error.lock().unwrap() = Some(e.to_string());
                            break;
                        }
                    }
                }
                // Wakes the sessions up with "disconnected".
                let mut subscribers = shared.subscribers.lock().unwrap();
                shared.ended.store(true, Ordering::Relaxed);
                subscribers.clear();
            })
            .expect("spawning the capture thread")
    };
    Hub {
        shared,
        output,
        display_id,
        bounds,
        stop,
        thread: Mutex::new(Some(thread)),
    }
}

struct Subscriber {
    hub: Arc<Hub>,
    frames: Receiver<Frame>,
}

impl Capturer for Subscriber {
    fn output_size(&self) -> (u32, u32) {
        self.hub.output
    }

    fn display_id(&self) -> u32 {
        self.hub.display_id
    }

    fn bounds(&self) -> Rect {
        self.hub.bounds
    }

    fn next_frame(&mut self, timeout: Duration) -> crate::Result<Option<Frame>> {
        match self.frames.recv_timeout(timeout) {
            Ok(frame) => Ok(Some(frame)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => {
                Err(match self.hub.shared.error.lock().unwrap().clone() {
                    Some(message) => CaptureError::Backend(message),
                    None => CaptureError::Stopped,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    static STARTS: AtomicUsize = AtomicUsize::new(0);

    /// Like a Windows duplication: a second capture of the same screen fails.
    fn exclusive_pattern(config: &CaptureConfig) -> crate::Result<Box<dyn Capturer>> {
        if STARTS.fetch_add(1, Ordering::SeqCst) > 0 {
            return Err(CaptureError::Backend("already duplicated".into()));
        }
        let (w, h) = config.max_size.unwrap();
        Ok(Box::new(crate::TestPattern::new(w, h, config.max_fps)))
    }

    #[test]
    fn two_sessions_share_one_capture() {
        let config = CaptureConfig {
            display_id: Some(42),
            max_size: Some((64, 32)),
            ..CaptureConfig::default()
        };
        let mut first = subscribe_with(&config, exclusive_pattern).unwrap();
        let mut second = subscribe_with(&config, exclusive_pattern).unwrap();
        assert_eq!(STARTS.load(Ordering::SeqCst), 1);
        for capturer in [&mut first, &mut second] {
            let frame = (0..20)
                .find_map(|_| capturer.next_frame(Duration::from_millis(100)).unwrap())
                .expect("frames for every session");
            assert_eq!((frame.width, frame.height), (64, 32));
        }
        // Once everyone is gone the capture stops; a new session starts a new one.
        drop(first);
        drop(second);
        STARTS.store(0, Ordering::SeqCst);
        assert!(subscribe_with(&config, exclusive_pattern).is_ok());
    }

    static BROKEN_STARTS: AtomicUsize = AtomicUsize::new(0);

    /// A capture that fails at once (e.g. the monitor changed size).
    struct Broken;

    impl Capturer for Broken {
        fn output_size(&self) -> (u32, u32) {
            (2, 2)
        }
        fn display_id(&self) -> u32 {
            7
        }
        fn bounds(&self) -> Rect {
            Rect::default()
        }
        fn next_frame(&mut self, _timeout: Duration) -> crate::Result<Option<Frame>> {
            Err(CaptureError::Backend("broken".into()))
        }
    }

    fn broken(_config: &CaptureConfig) -> crate::Result<Box<dyn Capturer>> {
        BROKEN_STARTS.fetch_add(1, Ordering::SeqCst);
        Ok(Box::new(Broken))
    }

    #[test]
    fn an_ended_capture_is_started_again() {
        let config = CaptureConfig {
            display_id: Some(7),
            ..CaptureConfig::default()
        };
        let mut first = subscribe_with(&config, broken).unwrap();
        assert!(matches!(
            first.next_frame(Duration::from_secs(1)),
            Err(CaptureError::Backend(_))
        ));
        // `first` still holds the ended capture: a new session must not join it.
        let _second = subscribe_with(&config, broken).unwrap();
        assert_eq!(BROKEN_STARTS.load(Ordering::SeqCst), 2);
    }
}
