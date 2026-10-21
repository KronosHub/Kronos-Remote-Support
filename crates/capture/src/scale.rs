//! CPU downscaling: bilinear for backends that capture at native resolution (Windows,
//! Linux), area average for the stream's reduced resolution (text stays legible).

/// Bilinear resize of a tightly packed BGRA image.
pub fn scale_bgra(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    let (sw, sh, dw, dh) = (sw as usize, sh as usize, dw as usize, dh as usize);
    if sw == dw && sh == dh {
        return src.to_vec();
    }
    let mut dst = vec![0u8; dw * dh * 4];
    // Fixed point (16.16) source coordinates of each destination column, computed once.
    let x_step = ((sw as u64) << 16) / dw as u64;
    let y_step = ((sh as u64) << 16) / dh as u64;
    let cols: Vec<(usize, usize, u32)> = (0..dw)
        .map(|x| {
            let fx = (x as u64 * x_step) as usize;
            let x0 = (fx >> 16).min(sw - 1);
            (x0, (x0 + 1).min(sw - 1), (fx & 0xffff) as u32)
        })
        .collect();

    for y in 0..dh {
        let fy = (y as u64 * y_step) as usize;
        let y0 = (fy >> 16).min(sh - 1);
        let y1 = (y0 + 1).min(sh - 1);
        let wy = (fy & 0xffff) as u32;
        let row0 = &src[y0 * sw * 4..(y0 + 1) * sw * 4];
        let row1 = &src[y1 * sw * 4..(y1 + 1) * sw * 4];
        let out = &mut dst[y * dw * 4..(y + 1) * dw * 4];
        for (x, &(x0, x1, wx)) in cols.iter().enumerate() {
            for c in 0..4 {
                let p00 = row0[x0 * 4 + c] as u32;
                let p01 = row0[x1 * 4 + c] as u32;
                let p10 = row1[x0 * 4 + c] as u32;
                let p11 = row1[x1 * 4 + c] as u32;
                let top = p00 * (0x10000 - wx) + p01 * wx;
                let bottom = p10 * (0x10000 - wx) + p11 * wx;
                let v = ((top as u64) * (0x10000 - wy) as u64 + (bottom as u64) * wy as u64) >> 32;
                out[x * 4 + c] = v as u8;
            }
        }
    }
    dst
}

/// Fixed-point one for the area weights.
const ONE: u32 = 1 << 12;

/// For each destination pixel along one axis: the source pixels it covers and their
/// weights (summing to [`ONE`]).
fn area_taps(src: usize, dst: usize) -> Vec<Vec<(usize, u32)>> {
    let ratio = src as f64 / dst as f64;
    (0..dst)
        .map(|d| {
            let (start, end) = (d as f64 * ratio, (d + 1) as f64 * ratio);
            let first = start.floor() as usize;
            let last = (end.ceil() as usize).min(src);
            let mut taps: Vec<(usize, u32)> = (first..last)
                .map(|i| {
                    let overlap = end.min(i as f64 + 1.0) - start.max(i as f64);
                    (i, (overlap / ratio * f64::from(ONE)).round() as u32)
                })
                .filter(|&(_, w)| w > 0)
                .collect();
            // Rounding leftovers go to the heaviest tap, so a flat area stays flat.
            let sum: u32 = taps.iter().map(|&(_, w)| w).sum();
            if let Some(tap) = taps.iter_mut().max_by_key(|&&mut (_, w)| w) {
                tap.1 = (i64::from(tap.1) + i64::from(ONE) - i64::from(sum)) as u32;
            }
            taps
        })
        .collect()
}

/// Area-average resize for shrinking a tightly packed BGRA image: every source pixel
/// counts, weighted by how much of it each destination pixel covers. Bilinear sampling
/// skips source pixels when shrinking by more than ~1.5×, and thin text strokes then
/// break up or vanish. Enlarging falls back to [`scale_bgra`].
pub fn shrink_bgra(src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> Vec<u8> {
    if dw > sw || dh > sh || (dw, dh) == (sw, sh) {
        return scale_bgra(src, sw, sh, dw, dh);
    }
    Shrinker::default().shrink(src, sw, sh, dw, dh).to_vec()
}

/// [`shrink_bgra`] for a stream of frames: the weights and buffers are kept between frames
/// of the same sizes instead of being made again for each one.
#[derive(Default)]
pub struct Shrinker {
    sizes: (u32, u32, u32, u32),
    cols: Vec<Vec<(usize, u32)>>,
    rows: Vec<Vec<(usize, u32)>>,
    narrow: Vec<u16>,
    dst: Vec<u8>,
}

impl Shrinker {
    /// Shrinks `src` (`sw`×`sh`) to `dw`×`dh` (bilinear if that is larger).
    pub fn shrink(&mut self, src: &[u8], sw: u32, sh: u32, dw: u32, dh: u32) -> &[u8] {
        if dw > sw || dh > sh {
            // Not a shrink (never asked by the encoder): plain bilinear.
            self.sizes = (0, 0, 0, 0);
            self.dst = scale_bgra(src, sw, sh, dw, dh);
            return &self.dst;
        }
        if self.sizes != (sw, sh, dw, dh) || self.cols.is_empty() {
            self.sizes = (sw, sh, dw, dh);
            let (sw, sh, dw, dh) = (sw as usize, sh as usize, dw as usize, dh as usize);
            self.cols = area_taps(sw, dw);
            self.rows = area_taps(sh, dh);
            self.narrow = vec![0u16; dw * sh * 4];
            self.dst = vec![0u8; dw * dh * 4];
        }
        let (sw, sh, dw) = (sw as usize, sh as usize, dw as usize);
        let round = |acc: u32| ((acc + ONE / 2) / ONE).min(255) as u16;

        // Horizontal pass: every source row shrunk to the destination width.
        for y in 0..sh {
            let row = &src[y * sw * 4..(y + 1) * sw * 4];
            let out = &mut self.narrow[y * dw * 4..(y + 1) * dw * 4];
            for (x, taps) in self.cols.iter().enumerate() {
                let mut acc = [0u32; 4];
                for &(i, w) in taps {
                    for c in 0..4 {
                        acc[c] += u32::from(row[i * 4 + c]) * w;
                    }
                }
                for c in 0..4 {
                    out[x * 4 + c] = round(acc[c]);
                }
            }
        }

        // Vertical pass.
        for (y, taps) in self.rows.iter().enumerate() {
            let out = &mut self.dst[y * dw * 4..(y + 1) * dw * 4];
            for (i, value) in out.iter_mut().enumerate() {
                let acc: u32 = taps
                    .iter()
                    .map(|&(row, w)| u32::from(self.narrow[row * dw * 4 + i]) * w)
                    .sum();
                *value = round(acc) as u8;
            }
        }
        &self.dst
    }
}

#[cfg(test)]
mod tests {
    use super::{Shrinker, scale_bgra, shrink_bgra};

    #[test]
    fn same_size_is_a_copy() {
        let img = vec![1, 2, 3, 4, 5, 6, 7, 8];
        assert_eq!(scale_bgra(&img, 2, 1, 2, 1), img);
    }

    #[test]
    fn uniform_image_stays_uniform() {
        let img = [10u8, 20, 30, 255].repeat(64 * 48);
        let out = scale_bgra(&img, 64, 48, 40, 30);
        assert_eq!(out.len(), 40 * 30 * 4);
        assert!(out.chunks(4).all(|p| p == [10, 20, 30, 255]));
    }

    #[test]
    fn shrinking_keeps_flat_areas_flat() {
        let img = [10u8, 20, 30, 255].repeat(64 * 48);
        let out = shrink_bgra(&img, 64, 48, 37, 29);
        assert_eq!(out.len(), 37 * 29 * 4);
        assert!(out.chunks(4).all(|p| p == [10, 20, 30, 255]));
    }

    #[test]
    fn shrinking_keeps_thin_lines() {
        // A one-pixel white column every 3 columns: shrunk 3×, every output pixel holds a
        // third of it (bilinear sampling could land on black only and lose the line).
        let row: Vec<u8> = (0..30)
            .flat_map(|x| {
                if x % 3 == 1 {
                    [255u8; 4]
                } else {
                    [0, 0, 0, 255]
                }
            })
            .collect();
        let img = row.repeat(6);
        let out = shrink_bgra(&img, 30, 6, 10, 2);
        assert!(out.chunks(4).all(|p| p == [85, 85, 85, 255]));
    }

    #[test]
    fn halving_averages_neighbours() {
        // Two columns: black and white → one grey pixel when sampling between them.
        let img = [0u8, 0, 0, 255, 255, 255, 255, 255].repeat(2);
        let out = scale_bgra(&img, 2, 2, 1, 1);
        assert_eq!(out.len(), 4);
    }

    #[test]
    fn reused_shrinker_matches_a_fresh_shrink() {
        let a: Vec<u8> = (0..64 * 48 * 4).map(|i| (i * 7 % 251) as u8).collect();
        let b: Vec<u8> = (0..64 * 48 * 4).map(|i| (i * 13 % 241) as u8).collect();
        let mut shrinker = Shrinker::default();
        for (img, size) in [
            (&a, (40, 30)),
            (&b, (40, 30)),
            (&a, (32, 24)),
            (&b, (40, 30)),
        ] {
            let reused = shrinker.shrink(img, 64, 48, size.0, size.1).to_vec();
            assert_eq!(reused, shrink_bgra(img, 64, 48, size.0, size.1));
        }
    }
}
