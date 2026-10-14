//! Linear resampling of interleaved stereo: devices run at 44.1 or 48 kHz, Opus at 48 kHz.
//! Good enough for monitoring sound remotely, and keeps no state but one frame.

pub struct Resampler {
    /// Input frames advanced per output frame.
    step: f64,
    /// Position of the next output frame, in input frames, counted from `prev`.
    pos: f64,
    /// Last input frame of the previous call.
    prev: [f32; 2],
}

impl Resampler {
    pub fn new(from: u32, to: u32) -> Self {
        Self {
            step: f64::from(from) / f64::from(to.max(1)),
            pos: 0.0,
            prev: [0.0; 2],
        }
    }

    /// Appends `input` (interleaved stereo) converted to the target rate to `out`.
    pub fn process(&mut self, input: &[f32], out: &mut Vec<f32>) {
        if self.step == 1.0 {
            out.extend_from_slice(input);
            return;
        }
        let frames = input.len() / 2;
        if frames == 0 {
            return;
        }
        // Frame `i` of the virtual sequence `prev, input[0], input[1], ...`.
        let frame = |i: usize| -> [f32; 2] {
            if i == 0 {
                self.prev
            } else {
                [input[(i - 1) * 2], input[(i - 1) * 2 + 1]]
            }
        };
        while self.pos + 1.0 <= frames as f64 {
            let i = self.pos.floor() as usize;
            let t = (self.pos - i as f64) as f32;
            let (a, b) = (frame(i), frame(i + 1));
            out.push(a[0] + (b[0] - a[0]) * t);
            out.push(a[1] + (b[1] - a[1]) * t);
            self.pos += self.step;
        }
        self.pos -= frames as f64;
        self.prev = frame(frames);
    }
}

#[cfg(test)]
mod tests {
    use super::Resampler;

    #[test]
    fn same_rate_is_a_copy() {
        let mut r = Resampler::new(48_000, 48_000);
        let mut out = Vec::new();
        r.process(&[0.1, 0.2, 0.3, 0.4], &mut out);
        assert_eq!(out, [0.1, 0.2, 0.3, 0.4]);
    }

    #[test]
    fn output_length_follows_the_ratio() {
        let mut r = Resampler::new(44_100, 48_000);
        let mut out = Vec::new();
        // One second in chunks of 441 frames.
        for _ in 0..100 {
            r.process(&[0.5; 441 * 2], &mut out);
        }
        let frames = out.len() / 2;
        assert!((47_990..=48_010).contains(&frames), "{frames}");
        // A constant signal stays constant (after the first frame, which starts from 0).
        assert!(out[200..].iter().all(|&s| (s - 0.5).abs() < 1e-6));
    }
}
