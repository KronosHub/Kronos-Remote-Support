//! The shared screen set to the viewer's window size (ADR 0061) or to the mode the viewer
//! picked (ADR 0069), and set back afterwards.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};

use capture::resolution::{self, Mode, Request};

/// A display changed by host sessions: how many of them want it so, and the user's own
/// mode from before the first change. The user's resolution comes back when the last of
/// them ends.
struct Held {
    users: usize,
    user_mode: Option<Mode>,
}

static CHANGED: Mutex<Option<HashMap<u32, Held>>> = Mutex::new(None);

/// File that exists while a display is changed: an app that crashed (or was killed) finds
/// it at the next start and puts the user's resolution back (see [`restore_after_crash`]).
static MARKER: OnceLock<PathBuf> = OnceLock::new();

/// Where to keep the marker of a changed resolution; call once at start, after
/// [`restore_after_crash`].
pub fn set_marker(path: PathBuf) {
    let _ = MARKER.set(path);
}

/// At start: if the previous run left a display changed (it crashed before restoring it),
/// every display goes back to the user's settings. macOS undoes such changes by itself
/// when the app quits; Windows keeps them until the user logs off.
pub fn restore_after_crash(path: &std::path::Path) {
    if !path.exists() {
        return;
    }
    match resolution::restore_all() {
        Ok(()) => tracing::info!("resolution left changed by the previous run restored"),
        Err(e) => tracing::warn!("cannot restore the resolution left changed: {e}"),
    }
    let _ = std::fs::remove_file(path);
}

fn update_marker(map: &HashMap<u32, Held>) {
    let Some(path) = MARKER.get() else { return };
    let result = if map.is_empty() {
        std::fs::remove_file(path).or_else(|e| match e.kind() {
            std::io::ErrorKind::NotFound => Ok(()),
            _ => Err(e),
        })
    } else {
        std::fs::write(path, b"resolution changed\n")
    };
    if let Err(e) = result {
        tracing::warn!("resolution marker {}: {e}", path.display());
    }
}

/// The user's own mode of a display, if a session changed it.
pub fn user_mode(display_id: u32) -> Option<Mode> {
    CHANGED
        .lock()
        .unwrap()
        .as_ref()
        .and_then(|map| map.get(&display_id))
        .and_then(|held| held.user_mode)
}

/// One host session's change of the shared screen's resolution.
#[derive(Default)]
pub struct DisplayResize {
    changed: Option<u32>,
}

impl DisplayResize {
    /// Whether this session changed a display (and which).
    pub fn changed(&self) -> Option<u32> {
        self.changed
    }

    /// Sets `display_id` to the mode closest to the request, at most `max` pixels (more would
    /// only be shrunk again before encoding). Returns the new size if it changed.
    pub async fn request(
        &mut self,
        display_id: u32,
        want: Request,
        max: (u32, u32),
    ) -> Option<(u32, u32)> {
        if want.width == 0 || want.height == 0 {
            self.release().await;
            return None;
        }
        let want = Request {
            width: want.width.min(max.0),
            height: want.height.min(max.1),
            ..want
        };
        self.apply(display_id, want).await
    }

    /// Sets `display_id` to `mode`, one of those `resolution::modes` offered (ADR 0069).
    /// Returns the new size if it changed.
    pub async fn set_mode(&mut self, display_id: u32, mode: Mode) -> Option<(u32, u32)> {
        let want = Request {
            width: mode.width,
            height: mode.height,
            scale: mode.scale as f32,
        };
        self.apply(display_id, want).await
    }

    async fn apply(&mut self, display_id: u32, want: Request) -> Option<(u32, u32)> {
        if self.changed.is_some_and(|d| d != display_id) {
            self.release().await;
        }
        // The user's own mode, read before the first change of this display.
        let known = user_mode(display_id).is_some();
        let result = tokio::task::spawn_blocking(move || {
            let before = (!known)
                .then(|| resolution::modes(display_id).ok().map(|(_, m)| m))
                .flatten();
            resolution::set(display_id, want).map(|size| (size, before))
        })
        .await;
        match result {
            Ok(Ok((Some(size), before))) => {
                tracing::info!(
                    display = display_id,
                    ?want,
                    ?size,
                    "shared screen resolution set"
                );
                self.hold(display_id, before);
                Some(size)
            }
            Ok(Ok((None, _))) => None,
            Ok(Err(e)) => {
                tracing::warn!(
                    display = display_id,
                    ?want,
                    "cannot change the resolution: {e}"
                );
                None
            }
            Err(e) => {
                tracing::warn!("resolution change failed: {e}");
                None
            }
        }
    }

    fn hold(&mut self, display: u32, before: Option<Mode>) {
        if self.changed == Some(display) {
            return;
        }
        self.changed = Some(display);
        let mut changed = CHANGED.lock().unwrap();
        let map = changed.get_or_insert_with(HashMap::new);
        let held = map.entry(display).or_insert(Held {
            users: 0,
            user_mode: before,
        });
        held.users += 1;
        if held.user_mode.is_none() {
            held.user_mode = before;
        }
        update_marker(map);
    }

    /// This session no longer needs the change: the display goes back to its own resolution
    /// unless another session still wants it changed. True when it was restored.
    pub async fn release(&mut self) -> bool {
        let Some(display_id) = self.changed.take() else {
            return false;
        };
        let last = {
            let mut changed = CHANGED.lock().unwrap();
            let map = changed.get_or_insert_with(HashMap::new);
            let last = map.get_mut(&display_id).is_none_or(|held| {
                held.users = held.users.saturating_sub(1);
                held.users == 0
            });
            if last {
                map.remove(&display_id);
            }
            last
        };
        if !last {
            return false;
        }
        let restored =
            match tokio::task::spawn_blocking(move || resolution::restore(display_id)).await {
                Ok(Ok(())) => {
                    tracing::info!(
                        display = display_id,
                        "shared screen back to its own resolution"
                    );
                    true
                }
                Ok(Err(e)) => {
                    tracing::warn!(display = display_id, "cannot restore the resolution: {e}");
                    false
                }
                Err(e) => {
                    tracing::warn!("resolution restore failed: {e}");
                    false
                }
            };
        // The marker goes once the display is back (or could not be: a restart would not
        // do better than this attempt, and the user can change it by hand).
        if let Some(map) = CHANGED.lock().unwrap().as_ref() {
            update_marker(map);
        }
        restored
    }
}
