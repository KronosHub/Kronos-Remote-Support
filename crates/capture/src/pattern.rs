//! Synthetic moving image: lets the streaming pipeline run (and be tested) without screen
//! access, on any platform, including CI machines.

use std::time::{Duration, Instant};

use crate::{Capturer, Frame, Result};

pub struct TestPattern {
    width: u32,
    height: u32,
    interval: Duration,
    next_due: Instant,
    frame_no: u32,
}

impl TestPattern {
    /// `width`/`height` are rounded down to even values.
    pub fn new(width: u32, height: u32, fps: u32) -> Self {
        Self {
            width: width.max(2) & !1,
            height: height.max(2) & !1,
            interval: Duration::from_secs(1) / fps.max(1),
            next_due: Instant::now(),
            frame_no: 0,
        }
    }

    /// Diagonal colour bands plus a bar sweeping left to right.
    fn render(&self) -> Vec<u8> {
        let (w, h) = (self.width as usize, self.height as usize);
        let shift = self.frame_no as usize * 4;
        let bar_x = (self.frame_no as usize * 8) % w;
        let mut data = vec![0u8; w * h * 4];
        for (y, row) in data.chunks_exact_mut(w * 4).enumerate() {
            for (x, px) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let band = ((x + y + shift) / 64) % 6;
                let (r, g, b) = match band {
                    0 => (29, 78, 216),
                    1 => (13, 148, 136),
                    2 => (241, 245, 249),
                    3 => (217, 119, 6),
                    4 => (220, 38, 38),
                    _ => (15, 23, 42),
                };
                let on_bar = x.abs_diff(bar_x) < 12;
                let (r, g, b) = if on_bar { (255, 255, 255) } else { (r, g, b) };
                *px = [b, g, r, 255];
            }
        }
        data
    }
}

impl Capturer for TestPattern {
    fn output_size(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn display_id(&self) -> u32 {
        0
    }

    fn bounds(&self) -> crate::Rect {
        crate::Rect {
            x: 0.0,
            y: 0.0,
            width: f64::from(self.width),
            height: f64::from(self.height),
        }
    }

    fn next_frame(&mut self, timeout: Duration) -> Result<Option<Frame>> {
        let now = Instant::now();
        if self.next_due > now {
            let wait = self.next_due - now;
            if wait > timeout {
                std::thread::sleep(timeout);
                return Ok(None);
            }
            std::thread::sleep(wait);
        }
        self.next_due += self.interval;
        // Do not try to catch up after a stall: skip to now.
        if self.next_due < Instant::now() {
            self.next_due = Instant::now() + self.interval;
        }
        let frame = Frame {
            width: self.width,
            height: self.height,
            data: self.render().into(),
            captured_at: Instant::now(),
            native: None,
        };
        self.frame_no = self.frame_no.wrapping_add(1);
        Ok(Some(frame))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn produces_moving_frames_of_the_right_size() {
        let mut p = TestPattern::new(65, 33, 1000);
        assert_eq!(p.output_size(), (64, 32));
        let a = p.next_frame(Duration::from_secs(1)).unwrap().unwrap();
        let b = p.next_frame(Duration::from_secs(1)).unwrap().unwrap();
        assert_eq!(a.data.len(), 64 * 32 * 4);
        assert_ne!(a.data, b.data);
    }
}
