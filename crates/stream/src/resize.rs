//! The shared screen set to the viewer's window size (ADR 0061), and set back afterwards.

use std::collections::HashMap;
use std::sync::Mutex;

use capture::resolution::{self, Request};

/// Displays changed by host sessions, with how many sessions want them so: the user's own
/// resolution comes back when the last of them ends.
static CHANGED: Mutex<Option<HashMap<u32, usize>>> = Mutex::new(None);

/// One host session's change of the shared screen's resolution.
#[derive(Default)]
pub struct DisplayResize {
    changed: Option<u32>,
}

impl DisplayResize {
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
        if self.changed.is_some_and(|d| d != display_id) {
            self.release().await;
        }
        let want = Request {
            width: want.width.min(max.0),
            height: want.height.min(max.1),
            ..want
        };
        let result = tokio::task::spawn_blocking(move || resolution::set(display_id, want)).await;
        match result {
            Ok(Ok(Some(size))) => {
                tracing::info!(
                    display = display_id,
                    ?want,
                    ?size,
                    "shared screen set to the viewer's window size"
                );
                self.hold(display_id);
                Some(size)
            }
            Ok(Ok(None)) => None,
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

    fn hold(&mut self, display: u32) {
        if self.changed == Some(display) {
            return;
        }
        self.changed = Some(display);
        let mut changed = CHANGED.lock().unwrap();
        *changed
            .get_or_insert_with(HashMap::new)
            .entry(display)
            .or_default() += 1;
    }

    /// This session no longer needs the change: the display goes back to its own resolution
    /// unless another session still wants it changed.
    pub async fn release(&mut self) {
        let Some(display_id) = self.changed.take() else {
            return;
        };
        let last = {
            let mut changed = CHANGED.lock().unwrap();
            let map = changed.get_or_insert_with(HashMap::new);
            let users = map.entry(display_id).or_default();
            *users = users.saturating_sub(1);
            let last = *users == 0;
            if last {
                map.remove(&display_id);
            }
            last
        };
        if !last {
            return;
        }
        match tokio::task::spawn_blocking(move || resolution::restore(display_id)).await {
            Ok(Ok(())) => tracing::info!(
                display = display_id,
                "shared screen back to its own resolution"
            ),
            Ok(Err(e)) => {
                tracing::warn!(display = display_id, "cannot restore the resolution: {e}")
            }
            Err(e) => tracing::warn!("resolution restore failed: {e}"),
        }
    }
}
