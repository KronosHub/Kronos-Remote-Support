//! "All displays": every monitor captured at once and laid out on one canvas as arranged on
//! the host, so a single video stream shows them side by side and mouse coordinates map
//! onto the union of their areas.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::{CaptureConfig, CaptureError, Capturer, Display, Frame, Rect, Result};

/// Where one display lands on the canvas, in canvas pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Canvas size and per-display placement for `displays`, fitting in `max_size`.
/// Pixel density follows the sharpest display (Retina), then the whole canvas is scaled
/// down to fit; placements never exceed the canvas.
pub fn layout(
    displays: &[Display],
    max_size: Option<(u32, u32)>,
) -> (Rect, (u32, u32), Vec<Placement>) {
    let union = displays
        .iter()
        .map(|d| d.bounds)
        .fold(None, |acc: Option<Rect>, b| {
            Some(match acc {
                None => b,
                Some(a) => {
                    let (x, y) = (a.x.min(b.x), a.y.min(b.y));
                    let right = (a.x + a.width).max(b.x + b.width);
                    let bottom = (a.y + a.height).max(b.y + b.height);
                    Rect {
                        x,
                        y,
                        width: right - x,
                        height: bottom - y,
                    }
                }
            })
        });
    let union = union.unwrap_or_default();
    // Native pixels per input unit of the sharpest display.
    let density = displays
        .iter()
        .filter(|d| d.bounds.width > 0.0)
        .map(|d| f64::from(d.width) / d.bounds.width)
        .fold(1.0, f64::max);
    let native = (
        (union.width * density).round().max(2.0) as u32,
        (union.height * density).round().max(2.0) as u32,
    );
    let (cw, ch) = crate::output_size(native.0, native.1, max_size);
    let (kx, ky) = (
        f64::from(cw) / union.width.max(1.0),
        f64::from(ch) / union.height.max(1.0),
    );
    let placements = displays
        .iter()
        .map(|d| {
            let x = (((d.bounds.x - union.x) * kx).round() as u32).min(cw - 2) & !1;
            let y = (((d.bounds.y - union.y) * ky).round() as u32).min(ch - 2) & !1;
            let width = ((d.bounds.width * kx).round() as u32).clamp(2, cw - x) & !1;
            let height = ((d.bounds.height * ky).round() as u32).clamp(2, ch - y) & !1;
            Placement {
                x,
                y,
                width,
                height,
            }
        })
        .collect();
    (union, (cw, ch), placements)
}

type Update = std::result::Result<(usize, Frame), CaptureError>;

pub struct CompositeCapturer {
    union: Rect,
    size: (u32, u32),
    placements: Vec<Placement>,
    canvas: Vec<u8>,
    updates: Receiver<Update>,
    stop: Arc<AtomicBool>,
    threads: Vec<JoinHandle<()>>,
}

impl CompositeCapturer {
    /// Starts one capturer per display (in parallel: some take seconds to start) using
    /// `start_one`, which receives a config for that display.
    pub fn start(
        config: &CaptureConfig,
        displays: Vec<Display>,
        start_one: fn(&CaptureConfig) -> Result<Box<dyn Capturer>>,
    ) -> Result<Self> {
        if displays.is_empty() {
            return Err(CaptureError::NoDisplay);
        }
        let (union, size, placements) = layout(&displays, config.max_size);
        let stop = Arc::new(AtomicBool::new(false));
        let (updates_tx, updates) = mpsc::channel::<Update>();
        let (started_tx, started) = mpsc::channel::<Result<()>>();
        let threads = displays
            .iter()
            .zip(&placements)
            .enumerate()
            .map(|(index, (display, place))| {
                let config = CaptureConfig {
                    display_id: Some(display.id),
                    max_size: Some((place.width, place.height)),
                    ..config.clone()
                };
                let (stop, updates, started) =
                    (stop.clone(), updates_tx.clone(), started_tx.clone());
                std::thread::Builder::new()
                    .name(format!("capture-{}", display.id))
                    .spawn(move || {
                        let mut capturer = match start_one(&config) {
                            Ok(c) => {
                                let _ = started.send(Ok(()));
                                c
                            }
                            Err(e) => {
                                let _ = started.send(Err(e));
                                return;
                            }
                        };
                        while !stop.load(Ordering::Relaxed) {
                            let update = match capturer.next_frame(Duration::from_millis(100)) {
                                Ok(Some(frame)) => Ok((index, frame)),
                                Ok(None) => continue,
                                Err(e) => Err(e),
                            };
                            let failed = update.is_err();
                            if updates.send(update).is_err() || failed {
                                break;
                            }
                        }
                    })
                    .map_err(|e| CaptureError::Backend(e.to_string()))
            })
            .collect::<Result<Vec<_>>>()?;
        let mut me = Self {
            union,
            size,
            placements,
            canvas: [0u8, 0, 0, 255].repeat(size.0 as usize * size.1 as usize),
            updates,
            stop,
            threads,
        };
        for _ in 0..displays.len() {
            match started.recv() {
                Ok(Ok(())) => {}
                // Dropping `me` stops the displays that did start.
                Ok(Err(e)) => return Err(e),
                Err(_) => return Err(CaptureError::Stopped),
            }
        }
        me.canvas.shrink_to_fit();
        Ok(me)
    }

    fn draw(&mut self, index: usize, frame: &Frame) {
        let place = self.placements[index];
        let scaled;
        let pixels = if (frame.width, frame.height) == (place.width, place.height) {
            &frame.data
        } else {
            scaled = crate::scale::scale_bgra(
                &frame.data,
                frame.width,
                frame.height,
                place.width,
                place.height,
            );
            &scaled
        };
        let canvas_stride = self.size.0 as usize * 4;
        let row = place.width as usize * 4;
        for y in 0..place.height as usize {
            let dst = (place.y as usize + y) * canvas_stride + place.x as usize * 4;
            self.canvas[dst..dst + row].copy_from_slice(&pixels[y * row..(y + 1) * row]);
        }
    }
}

impl Capturer for CompositeCapturer {
    fn output_size(&self) -> (u32, u32) {
        self.size
    }

    fn display_id(&self) -> u32 {
        crate::ALL_DISPLAYS
    }

    fn bounds(&self) -> Rect {
        self.union
    }

    fn next_frame(&mut self, timeout: Duration) -> Result<Option<Frame>> {
        let first = match self.updates.recv_timeout(timeout) {
            Ok(update) => update,
            Err(RecvTimeoutError::Timeout) => return Ok(None),
            Err(RecvTimeoutError::Disconnected) => return Err(CaptureError::Stopped),
        };
        // Everything already queued goes into the same composed frame.
        let mut captured_at = Instant::now();
        let mut next = Some(first);
        while let Some(update) = next {
            let (index, frame) = update?;
            captured_at = frame.captured_at;
            self.draw(index, &frame);
            next = self.updates.try_recv().ok();
        }
        Ok(Some(Frame {
            width: self.size.0,
            height: self.size.1,
            data: self.canvas.clone(),
            captured_at,
        }))
    }
}

impl Drop for CompositeCapturer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn display(id: u32, x: f64, y: f64, w: f64, h: f64, scale: f64) -> Display {
        Display {
            id,
            name: format!("d{id}"),
            width: (w * scale) as u32,
            height: (h * scale) as u32,
            scale: scale as f32,
            primary: id == 1,
            bounds: Rect {
                x,
                y,
                width: w,
                height: h,
            },
        }
    }

    #[test]
    fn side_by_side_fits_the_box() {
        let displays = [
            display(1, 0.0, 0.0, 1920.0, 1080.0, 1.0),
            display(2, 1920.0, 0.0, 1920.0, 1080.0, 1.0),
        ];
        let (union, size, places) = layout(&displays, Some((3840, 2160)));
        assert_eq!(
            union,
            Rect {
                x: 0.0,
                y: 0.0,
                width: 3840.0,
                height: 1080.0
            }
        );
        assert_eq!(size, (3840, 1080));
        assert_eq!(
            places[1],
            Placement {
                x: 1920,
                y: 0,
                width: 1920,
                height: 1080
            }
        );
    }

    #[test]
    fn retina_left_of_external_and_negative_origin() {
        // Mac: external monitor left of the Retina laptop (origin at the laptop).
        let displays = [
            display(1, 0.0, 0.0, 1512.0, 982.0, 2.0),
            display(2, -1920.0, -100.0, 1920.0, 1080.0, 1.0),
        ];
        let (union, size, places) = layout(&displays, Some((3840, 2160)));
        assert_eq!(union.x, -1920.0);
        assert_eq!(union.width, 3432.0);
        assert!(size.0 <= 3840 && size.1 <= 2160);
        // Laptop on the right, external on the left, nothing outside the canvas.
        assert!(places[0].x > places[1].x);
        for p in &places {
            assert!(
                p.x + p.width <= size.0 && p.y + p.height <= size.1,
                "{p:?} in {size:?}"
            );
            assert!(p.width % 2 == 0 && p.height % 2 == 0);
        }
    }

    struct Solid {
        size: (u32, u32),
        value: u8,
        sent: bool,
    }

    impl Capturer for Solid {
        fn output_size(&self) -> (u32, u32) {
            self.size
        }
        fn display_id(&self) -> u32 {
            u32::from(self.value)
        }
        fn bounds(&self) -> Rect {
            Rect::default()
        }
        fn next_frame(&mut self, timeout: Duration) -> Result<Option<Frame>> {
            if self.sent {
                std::thread::sleep(timeout);
                return Ok(None);
            }
            self.sent = true;
            Ok(Some(Frame {
                width: self.size.0,
                height: self.size.1,
                data: [self.value, self.value, self.value, 255]
                    .repeat((self.size.0 * self.size.1) as usize),
                captured_at: Instant::now(),
            }))
        }
    }

    fn solid(config: &CaptureConfig) -> Result<Box<dyn Capturer>> {
        let (w, h) = config.max_size.unwrap();
        Ok(Box::new(Solid {
            size: (w, h),
            value: config.display_id.unwrap() as u8 * 100,
            sent: false,
        }))
    }

    #[test]
    fn composes_displays_on_one_canvas() {
        let displays = vec![
            display(1, 0.0, 0.0, 64.0, 32.0, 1.0),
            display(2, 64.0, 16.0, 32.0, 16.0, 1.0),
        ];
        let config = CaptureConfig {
            max_size: Some((3840, 2160)),
            ..CaptureConfig::default()
        };
        let mut capturer = CompositeCapturer::start(&config, displays, solid).unwrap();
        assert_eq!(capturer.output_size(), (96, 32));
        let mut frame = None;
        let deadline = Instant::now() + Duration::from_secs(2);
        // Both displays drawn (possibly over two frames).
        while Instant::now() < deadline {
            if let Some(f) = capturer.next_frame(Duration::from_millis(100)).unwrap() {
                frame = Some(f);
            }
            let f = frame.as_ref();
            if f.is_some_and(|f| f.data[(20 * 96 + 80) * 4] == 200 && f.data[0] == 100) {
                break;
            }
        }
        let f = frame.unwrap();
        let px = |x: usize, y: usize| f.data[(y * 96 + x) * 4];
        assert_eq!(px(10, 10), 100); // display 1
        assert_eq!(px(80, 20), 200); // display 2
        assert_eq!(px(80, 4), 0); // gap above display 2: black
    }
}
