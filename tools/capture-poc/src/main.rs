//! Phase 1 proof of concept: capture the screen, encode it to H.264 and write a raw
//! Annex B file, printing throughput and latency statistics.
//!
//! Play the result with: `ffplay -f h264 capture.h264`

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use capture::CaptureConfig;
use clap::Parser;
use codec::{EncoderSettings, H264Encoder};

#[derive(Parser)]
#[command(about = "Capture the screen and encode it to a raw H.264 file")]
struct Args {
    /// List displays and exit.
    #[arg(long)]
    list: bool,
    /// Display id to capture (default: main display).
    #[arg(long)]
    display: Option<u32>,
    /// Capture duration in seconds.
    #[arg(long, default_value_t = 10)]
    seconds: u64,
    #[arg(long, default_value_t = 30)]
    fps: u32,
    /// Downscale to fit this width (0 = native, capped at the encoder limit).
    #[arg(long, default_value_t = 1920)]
    max_width: u32,
    /// Downscale to fit this height (0 = native, capped at the encoder limit).
    #[arg(long, default_value_t = 1080)]
    max_height: u32,
    /// Target bitrate in kbit/s.
    #[arg(long, default_value_t = 4000)]
    bitrate: u32,
    #[arg(long, short, default_value = "capture.h264")]
    output: PathBuf,
}

#[derive(Default)]
struct Stats {
    frames_in: u64,
    frames_out: u64,
    keyframes: u64,
    bytes: u64,
    encode_time: Duration,
    /// Capture timestamp to encoded bitstream ready.
    pipeline_latency: Duration,
    max_pipeline_latency: Duration,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    let args = Args::parse();

    if args.list {
        for d in capture::displays().context("listing displays")? {
            println!(
                "{:>10}  {}x{}  scale {:.1}  at ({}, {}) size {}x{}",
                d.id,
                d.width,
                d.height,
                d.scale,
                d.bounds.x,
                d.bounds.y,
                d.bounds.width,
                d.bounds.height
            );
        }
        return Ok(());
    }

    let config = CaptureConfig {
        display_id: args.display,
        max_fps: args.fps,
        max_size: Some(encoder_box(args.max_width, args.max_height)),
        show_cursor: true,
    };
    let mut capturer = capture::start(&config).context(
        "starting capture (on macOS grant Screen Recording permission to the terminal app)",
    )?;
    let (width, height) = capturer.output_size();

    let mut encoder = H264Encoder::new(&EncoderSettings {
        fps: args.fps,
        bitrate_bps: args.bitrate * 1000,
        ..EncoderSettings::new(width, height)
    })?;
    let mut out = BufWriter::new(File::create(&args.output)?);

    println!(
        "Capturing {width}x{height} @ {} fps for {}s -> {}",
        args.fps,
        args.seconds,
        args.output.display()
    );

    let start = Instant::now();
    let deadline = start + Duration::from_secs(args.seconds);
    let mut stats = Stats::default();

    while Instant::now() < deadline {
        let Some(frame) = capturer.next_frame(Duration::from_millis(100))? else {
            continue; // static screen: nothing new to encode
        };
        stats.frames_in += 1;

        // ScreenCaptureKit may change the output size (e.g. display reconfigured).
        if (frame.width, frame.height) != encoder.size() {
            tracing::warn!(size = ?(frame.width, frame.height), "frame size changed, skipping");
            continue;
        }

        let t0 = Instant::now();
        let ts = frame.captured_at.duration_since(start).as_millis() as u64;
        let encoded = encoder.encode_bgra(&frame.data, frame.width, frame.height, ts)?;
        let done = Instant::now();
        stats.encode_time += done - t0;

        if let Some(encoded) = encoded {
            let latency = done - frame.captured_at;
            stats.pipeline_latency += latency;
            stats.max_pipeline_latency = stats.max_pipeline_latency.max(latency);
            stats.frames_out += 1;
            stats.keyframes += u64::from(encoded.keyframe);
            stats.bytes += encoded.data.len() as u64;
            out.write_all(&encoded.data)?;
        }
    }
    out.flush()?;
    drop(capturer);

    report(&stats, start.elapsed(), &args);
    Ok(())
}

/// User-requested size box, never larger than what the encoder accepts.
fn encoder_box(max_width: u32, max_height: u32) -> (u32, u32) {
    let limit = |v: u32, max: u32| if v == 0 { max } else { v.min(max) };
    (
        limit(max_width, codec::MAX_WIDTH),
        limit(max_height, codec::MAX_HEIGHT),
    )
}

fn report(s: &Stats, elapsed: Duration, args: &Args) {
    let secs = elapsed.as_secs_f64();
    let avg_ms = |total: Duration, n: u64| {
        if n == 0 {
            0.0
        } else {
            total.as_secs_f64() * 1000.0 / n as f64
        }
    };
    let fps_out = s.frames_out as f64 / secs;

    println!("\n--- results ({secs:.1}s) ---");
    println!(
        "frames captured   {:>8}  ({:.1} fps)",
        s.frames_in,
        s.frames_in as f64 / secs
    );
    println!(
        "frames encoded    {:>8}  ({fps_out:.1} fps, {} keyframes)",
        s.frames_out, s.keyframes
    );
    println!(
        "avg encode time   {:>8.2} ms",
        avg_ms(s.encode_time, s.frames_in)
    );
    println!(
        "avg pipeline lat. {:>8.2} ms (max {:.2})",
        avg_ms(s.pipeline_latency, s.frames_out),
        s.max_pipeline_latency.as_secs_f64() * 1000.0
    );
    println!(
        "output            {:>8.1} KiB  ({:.0} kbit/s)",
        s.bytes as f64 / 1024.0,
        s.bytes as f64 * 8.0 / 1000.0 / secs
    );
    println!("\nplay:   ffplay -f h264 {}", args.output.display());
    println!(
        "to mp4: ffmpeg -r {:.0} -i {} -c copy capture.mp4",
        fps_out.max(1.0),
        args.output.display()
    );
}
