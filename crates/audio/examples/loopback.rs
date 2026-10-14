//! Records this computer's sound for a few seconds and plays it back through the full
//! pipeline (capture → Opus → wire packet → decode → playback). Manual check:
//! `cargo run -p audio --example loopback -- 5`
#[cfg(any(target_os = "macos", windows))]
fn main() -> anyhow::Result<()> {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    let seconds: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(5);
    let player = Arc::new(audio::Player::start()?);
    let packets = Arc::new(AtomicUsize::new(0));
    let bytes = Arc::new(AtomicUsize::new(0));
    let capture = {
        let (player, packets, bytes) = (player.clone(), packets.clone(), bytes.clone());
        audio::Capture::start(move |packet| {
            packets.fetch_add(1, Ordering::Relaxed);
            bytes.fetch_add(packet.len(), Ordering::Relaxed);
            player.push(&packet);
        })?
    };
    std::thread::sleep(std::time::Duration::from_secs(seconds));
    drop(capture);
    let (n, b) = (
        packets.load(Ordering::Relaxed),
        bytes.load(Ordering::Relaxed),
    );
    println!(
        "{n} packets ({} per second), {} kbit/s",
        n as u64 / seconds,
        b as u64 * 8 / seconds / 1000
    );
    Ok(())
}

#[cfg(not(any(target_os = "macos", windows)))]
fn main() {
    eprintln!("sound is supported on macOS and Windows only");
}
