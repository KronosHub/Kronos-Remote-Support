//! Does encoding a still picture again make it sharper? Scrolls a text-like image at a low
//! bitrate, then repeats the last frame and prints the PSNR of each decoded pass.
//! `cargo run --release -p codec --example refine -- [hw] [kbps]`
use codec::{EncoderSettings, VideoEncoder, new_encoder};

/// The page scrolled by `offset` rows: a real screenshot when `REFINE_IMAGE` names a raw
/// 1920x1080 BGRA file (`ffmpeg -i shot.png -vf crop=1920:1080:0:0 -f rawvideo -pix_fmt
/// bgra shot.bgra`), else synthetic glyphs.
fn page(w: u32, h: u32, offset: u32) -> Vec<u8> {
    if let Some(path) = std::env::var_os("REFINE_IMAGE") {
        let image = std::fs::read(path).unwrap();
        let row = (w * 4) as usize;
        let mut out = Vec::with_capacity(image.len());
        for y in 0..h {
            let src = ((y + offset) % h) as usize * row;
            out.extend_from_slice(&image[src..src + row]);
        }
        return out;
    }
    // Dark background, light "glyphs": 6x10 cells whose pixels follow a fixed hash.
    let mut out = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let (cx, cy) = (x / 7, (y + offset) / 14);
            let (gx, gy) = (x % 7, (y + offset) % 14);
            let seed = cx.wrapping_mul(2_654_435_761) ^ cy.wrapping_mul(40_503);
            let on = gx < 6
                && gy < 10
                && (seed >> ((gx + gy * 6) % 29)) & 1 == 1
                && (seed >> 7) % 5 != 0;
            let v = if on { 220 } else { 30 };
            let i = ((y * w + x) * 4) as usize;
            out[i..i + 4].copy_from_slice(&[v, v, v, 255]);
        }
    }
    out
}

/// PSNR between two decoded RGB pictures (the reference is the same picture encoded almost
/// losslessly, so the colour conversion does not count as error).
fn psnr(a: &[u8], b: &[u8]) -> f64 {
    let err: f64 = a
        .iter()
        .zip(b)
        .map(|(x, y)| (f64::from(*x) - f64::from(*y)).powi(2))
        .sum();
    let mse = err / a.len() as f64;
    10.0 * (255.0 * 255.0 / mse.max(1e-9)).log10()
}

/// `bgra` encoded alone at a very high bitrate (OpenH264, minimum quantizer) and decoded.
fn reference(bgra: &[u8], w: u32, h: u32) -> Vec<u8> {
    let settings = EncoderSettings {
        bitrate_bps: 200_000_000,
        ..EncoderSettings::new(w, h)
    };
    let (mut encoder, _) = new_encoder(&settings, false).unwrap();
    let encoded = encoder.encode_bgra(bgra, w, h, 0).unwrap().unwrap();
    let mut decoder = openh264::decoder::Decoder::new().unwrap();
    let yuv = decoder.decode(&encoded.data).unwrap().unwrap();
    let mut rgb = vec![0u8; (w * h * 3) as usize];
    yuv.write_rgb8(&mut rgb);
    rgb
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let hw = args.iter().any(|a| a == "hw");
    let kbps: u32 = args.iter().find_map(|a| a.parse().ok()).unwrap_or(2000);
    let (w, h) = (1920, 1080);
    let settings = EncoderSettings {
        fps: 30,
        bitrate_bps: kbps * 1000,
        keyframe_interval: 3600,
        ..EncoderSettings::new(w, h)
    };
    let (mut encoder, failure) = new_encoder(&settings, hw)?;
    println!("encoder {} {kbps} kbit/s {failure:?}", encoder.name());
    let mut decoder = openh264::decoder::Decoder::new()?;
    let mut rgb = vec![0u8; (w * h * 3) as usize];
    let mut ts = 0;
    let mut step =
        |frame: &[u8], refer: &[u8], label: &str, encoder: &mut Box<dyn VideoEncoder>| {
            ts += 33;
            let Some(encoded) = encoder.encode_bgra(frame, w, h, ts).unwrap() else {
                println!("{label}: no output");
                return;
            };
            // `REFINE_DUMP=<dir>`: stream and source frames for decoding with ffmpeg (the
            // OpenH264 decoder reads only the baseline profile, not the hardware encoder's High).
            if let Some(dir) = std::env::var_os("REFINE_DUMP") {
                use std::io::Write;
                let dir = std::path::PathBuf::from(dir);
                let append = |name: &str, data: &[u8]| {
                    std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(dir.join(name))
                        .unwrap()
                        .write_all(data)
                        .unwrap()
                };
                append("stream.h264", &encoded.data);
                append("source.bgra", frame);
                println!("{label}: {:>6} bytes", encoded.data.len());
                return;
            }
            if let Some(yuv) = decoder.decode(&encoded.data).unwrap() {
                yuv.write_rgb8(&mut rgb);
                println!(
                    "{label}: {:>6} bytes  PSNR {:.1} dB",
                    encoded.data.len(),
                    psnr(&rgb, refer)
                );
            }
        };
    for i in 0..15 {
        // `drop`: halfway through the scroll the link degrades to 250 kbit/s at 10 fps.
        if i == 7 && args.iter().any(|a| a == "drop") {
            encoder.set_rate(250_000, 10);
            println!("-- set_rate(250 kbit/s, 10 fps)");
        }
        let frame = page(w, h, i * 40);
        let refer = reference(&frame, w, h);
        step(&frame, &refer, &format!("scroll {i:>2}"), &mut encoder);
    }
    let still = page(w, h, 14 * 40);
    let still_ref = reference(&still, w, h);
    // `low`: plan the rate control for 2 fps, so each still pass may take a 15x larger share.
    if args.iter().any(|a| a == "low") {
        encoder.set_rate(kbps * 1000, 2);
    }
    for pass in 1..=6 {
        step(
            &still,
            &still_ref,
            &format!("still pass {pass}"),
            &mut encoder,
        );
    }
    Ok(())
}
