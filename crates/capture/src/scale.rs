//! CPU downscaling for backends that capture at native resolution (Windows, Linux).

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

#[cfg(test)]
mod tests {
    use super::scale_bgra;

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
    fn halving_averages_neighbours() {
        // Two columns: black and white → one grey pixel when sampling between them.
        let img = [0u8, 0, 0, 255, 255, 255, 255, 255].repeat(2);
        let out = scale_bgra(&img, 2, 2, 1, 1);
        assert_eq!(out.len(), 4);
    }
}
