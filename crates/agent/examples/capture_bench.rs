//! Times the agent's screen capture where it runs (SYSTEM in the console session), without a
//! release. Run as SYSTEM in session 0 (e.g. `prlctl exec`): it starts itself in the console
//! session, paints a changing square on the screen so the duplication has frames to give,
//! and logs per-second capture figures to `C:\Windows\Temp\capture-bench.log`.

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    use std::time::{Duration, Instant};

    let child = std::env::args().nth(1).as_deref() == Some("child");
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(r"C:\Windows\Temp\capture-bench.log")?;
    tracing_subscriber::fmt()
        .with_writer(std::sync::Mutex::new(log))
        .with_ansi(false)
        .init();

    if !child {
        let exe = std::env::current_exe()?;
        let console =
            unsafe { windows_sys::Win32::System::RemoteDesktop::WTSGetActiveConsoleSessionId() };
        let process = agent::windows::launch_in_session(console, &exe, "child")?;
        unsafe { windows_sys::Win32::System::Threading::WaitForSingleObject(process, 30_000) };
        return Ok(());
    }

    windesk::enable();
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let painter = {
        let stop = stop.clone();
        std::thread::spawn(move || {
            use windows_sys::Win32::Graphics::Gdi::*;
            let mut n = 0u32;
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                unsafe {
                    let dc = GetDC(std::ptr::null_mut());
                    let brush = CreateSolidBrush((n * 40) & 0xFF_FFFF);
                    let rect = windows_sys::Win32::Foundation::RECT {
                        left: 200,
                        top: 200,
                        right: 600,
                        bottom: 600,
                    };
                    FillRect(dc, &rect, brush);
                    DeleteObject(brush);
                    ReleaseDC(std::ptr::null_mut(), dc);
                }
                n += 1;
                std::thread::sleep(Duration::from_millis(10));
            }
        })
    };
    for (label, max_size) in [("2560x1440", Some((2560, 1440))), ("native", None)] {
        let config = capture::CaptureConfig {
            max_fps: 60,
            max_size,
            ..Default::default()
        };
        let mut capturer = capture::start(&config)?;
        let start = Instant::now();
        let (mut frames, mut busy, mut worst) = (0u32, Duration::ZERO, Duration::ZERO);
        while start.elapsed() < Duration::from_secs(8) {
            let t = Instant::now();
            if let Some(frame) = capturer.next_frame(Duration::from_millis(100))? {
                let took = t.elapsed();
                frames += 1;
                busy += took;
                worst = worst.max(took);
                std::hint::black_box(frame);
            }
        }
        tracing::info!(
            label,
            output = ?capturer.output_size(),
            fps = frames as f64 / start.elapsed().as_secs_f64(),
            avg_ms = busy.as_secs_f64() * 1000.0 / frames.max(1) as f64,
            worst_ms = worst.as_millis(),
            "capture"
        );
    }
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    painter.join().ok();
    Ok(())
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Windows only");
}
