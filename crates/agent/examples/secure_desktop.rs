//! Live check of Ctrl+Alt+Del and of capturing the secure desktop (ADR 0070), without a
//! release. Run as SYSTEM in session 0 (e.g. `prlctl exec`): it starts itself as SYSTEM in
//! the console session to capture there like the agent, sends Ctrl+Alt+Del from session 0
//! after 3 s, and the child writes the first picture after each desktop change to
//! `C:\Windows\Temp\secure-desktop-<n>-<desktop>.bmp`. Log: `C:\Windows\Temp\secure-desktop.log`.

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    use std::time::{Duration, Instant};

    let child = std::env::args().nth(1).as_deref() == Some("child");
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(r"C:\Windows\Temp\secure-desktop.log")?;
    tracing_subscriber::fmt()
        .with_writer(std::sync::Mutex::new(log))
        .with_ansi(false)
        .init();

    if !child {
        tracing::info!(session = ?agent::windows::current_session(), "parent");
        let exe = std::env::current_exe()?;
        let console =
            unsafe { windows_sys::Win32::System::RemoteDesktop::WTSGetActiveConsoleSessionId() };
        let process = agent::windows::launch_in_session(console, &exe, "child")?;
        std::thread::sleep(Duration::from_secs(3));
        let code = agent::windows::run_send_sas();
        tracing::info!(code, "Ctrl+Alt+Del from session 0");
        unsafe { windows_sys::Win32::System::Threading::WaitForSingleObject(process, 20_000) };
        tracing::info!("parent done");
        return Ok(());
    }

    tracing::info!(session = ?agent::windows::current_session(), "child");
    windesk::enable();
    let config = capture::CaptureConfig {
        max_fps: 30,
        ..Default::default()
    };
    let mut capturer = capture::start(&config)?;
    let mut desktop = windesk::input_desktop_name();
    let mut want_shot = true;
    let mut shots = 0;
    let mut frames = 0u32;
    let mut last: Option<capture::Frame> = None;
    let start = Instant::now();
    while start.elapsed() < Duration::from_secs(12) {
        let now_desktop = windesk::input_desktop_name();
        if now_desktop != desktop {
            tracing::info!(from = ?desktop, to = ?now_desktop, "desktop changed");
            desktop = now_desktop;
            want_shot = true;
        }
        match capturer.next_frame(Duration::from_millis(100)) {
            Ok(Some(frame)) => {
                frames += 1;
                tracing::info!(at_ms = start.elapsed().as_millis(), "frame");
                if want_shot {
                    want_shot = false;
                    shots += 1;
                    let name = desktop.clone().unwrap_or_else(|| "unknown".into());
                    let path = format!(r"C:\Windows\Temp\secure-desktop-{shots}-{name}.bmp");
                    write_bmp(&path, frame.width, frame.height, &frame.data)?;
                    tracing::info!(path, at_ms = start.elapsed().as_millis(), "picture saved");
                }
                last = Some(frame);
            }
            Ok(None) => {}
            Err(e) => {
                tracing::warn!("capture error: {e}");
                break;
            }
        }
    }
    if let Some(frame) = last {
        write_bmp(
            r"C:\Windows\Temp\secure-desktop-last.bmp",
            frame.width,
            frame.height,
            &frame.data,
        )?;
    }
    tracing::info!(frames, "child done");
    Ok(())
}

#[cfg(windows)]
fn write_bmp(path: &str, width: u32, height: u32, bgra: &[u8]) -> std::io::Result<()> {
    let size = width * height * 4;
    let mut out = Vec::with_capacity(54 + size as usize);
    out.extend_from_slice(b"BM");
    out.extend_from_slice(&(54 + size).to_le_bytes());
    out.extend_from_slice(&[0; 4]);
    out.extend_from_slice(&54u32.to_le_bytes());
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(width as i32).to_le_bytes());
    out.extend_from_slice(&(-(height as i32)).to_le_bytes()); // top-down
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&32u16.to_le_bytes());
    out.extend_from_slice(&[0; 24]);
    out.extend_from_slice(&bgra[..size as usize]);
    std::fs::write(path, out)
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Windows only");
}
