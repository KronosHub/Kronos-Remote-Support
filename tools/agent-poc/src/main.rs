//! Windows-only harness for the Kronos agent (ADR 0033). Never runs in production.
//!
//!   agent-poc install [product]      install+start the SYSTEM service (needs admin)
//!   agent-poc uninstall [product]    stop+remove the service
//!   agent-poc --service <name> <p>   (service entry point)
//!   agent-poc --agent <product>      (agent entry point)
//!   agent-poc capture <secs> [out]   connect to agent, save the last frame + fps
//!   agent-poc movemouse <x> <y>      agent moves the pointer to x,y (0..1) of the screen
//!   agent-poc whoami                 print session/desktop (no agent)

fn main() {
    tracing_subscriber::fmt().with_env_filter("info").init();
    if let Err(e) = real_main() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

#[cfg(not(windows))]
fn real_main() -> anyhow::Result<()> {
    anyhow::bail!("agent-poc is Windows-only");
}

#[cfg(windows)]
fn real_main() -> anyhow::Result<()> {
    use capture::Capturer;
    use std::time::{Duration, Instant};
    let args: Vec<String> = std::env::args().collect();
    let service = "KronosAgentPoc";
    match args.get(1).map(String::as_str) {
        Some("install") => {
            let product = args.get(2).cloned().unwrap_or_else(|| "poc".into());
            let exe = std::env::current_exe()?;
            agent::windows::install_service(service, "Kronos Agent PoC", &exe, &product, false)?;
            println!("service installed and started");
        }
        Some("uninstall") => {
            agent::windows::uninstall_service(service)?;
            println!("service removed");
        }
        Some("--service") => {
            let name = args.get(2).cloned().unwrap_or_else(|| service.into());
            let product = args.get(3).cloned().unwrap_or_else(|| "poc".into());
            let app = args.get(4).map(std::path::PathBuf::from);
            agent::windows::run_service(&name, &product, app.as_deref())?;
        }
        Some("--agent") => {
            let product = args.get(2).cloned().unwrap_or_else(|| "poc".into());
            let app = args.get(3).map(std::path::PathBuf::from);
            agent::windows::run_agent(&product, app.as_deref())?;
        }
        Some("whoami") => {
            println!("session: {:?}", agent::windows::current_session());
            println!("input desktop: {:?}", windesk::input_desktop_name());
            println!(
                "agent available: {}",
                agent::windows::agent_available("poc")
            );
        }
        Some("capture") => {
            let secs: u64 = args.get(2).map_or(5, |s| s.parse().unwrap_or(5));
            let out = args
                .get(3)
                .cloned()
                .unwrap_or_else(|| "agent-capture.ppm".into());
            let config = capture::CaptureConfig {
                display_id: None,
                max_fps: 15,
                max_size: Some((1280, 720)),
                show_cursor: true,
            };
            let mut cap = agent::client::AgentCapturer::start(
                || async { agent::windows::connect("poc").await },
                &config,
            )
            .map_err(|e| anyhow::anyhow!("{e}"))?;
            println!("capture started {:?}", cap.output_size());
            let (mut frames, mut last) = (0u64, None);
            let deadline = Instant::now() + Duration::from_secs(secs);
            while Instant::now() < deadline {
                if let Some(f) = cap
                    .next_frame(Duration::from_millis(200))
                    .map_err(|e| anyhow::anyhow!("{e}"))?
                {
                    frames += 1;
                    last = Some(f);
                }
            }
            let fps = frames as f64 / secs as f64;
            println!("{frames} frames ({fps:.1} fps)");
            if let Some(f) = last {
                write_ppm(&out, &f)?;
                println!("last frame -> {out}");
            } else {
                anyhow::bail!("no frames from the agent");
            }
        }
        Some("click") => {
            use capture::Capturer;
            let x: f32 = args.get(2).map_or(0.5, |s| s.parse().unwrap_or(0.5));
            let y: f32 = args.get(3).map_or(0.5, |s| s.parse().unwrap_or(0.5));
            let cap = agent::client::AgentCapturer::start(
                || async { agent::windows::connect("poc").await },
                &capture::CaptureConfig {
                    display_id: None,
                    max_fps: 5,
                    max_size: Some((640, 360)),
                    show_cursor: true,
                },
            )
            .map_err(|e| anyhow::anyhow!("{e}"))?;
            let b = cap.bounds();
            let input = agent::client::AgentInput::start(|| agent::windows::connect("poc"))?;
            input.set_area(input::Area {
                x: b.x,
                y: b.y,
                width: b.width,
                height: b.height,
            });
            input.send(protocol::InputEvent::MouseMove { x, y });
            std::thread::sleep(Duration::from_millis(120));
            input.send(protocol::InputEvent::MouseButton {
                button: protocol::MouseButton::Left,
                down: true,
            });
            std::thread::sleep(Duration::from_millis(80));
            input.send(protocol::InputEvent::MouseButton {
                button: protocol::MouseButton::Left,
                down: false,
            });
            std::thread::sleep(Duration::from_millis(300));
            println!("clicked {x},{y}");
        }
        Some("key") => {
            let code = args.get(2).cloned().unwrap_or_else(|| "Escape".into());
            let input = agent::client::AgentInput::start(|| agent::windows::connect("poc"))?;
            input.send(protocol::InputEvent::Key {
                code: code.clone(),
                down: true,
            });
            std::thread::sleep(Duration::from_millis(60));
            input.send(protocol::InputEvent::Key {
                code: code.clone(),
                down: false,
            });
            std::thread::sleep(Duration::from_millis(300));
            println!("sent key {code}");
        }
        Some("movemouse") => {
            let x: f32 = args.get(2).map_or(0.5, |s| s.parse().unwrap_or(0.5));
            let y: f32 = args.get(3).map_or(0.5, |s| s.parse().unwrap_or(0.5));
            // Area from the agent's capture bounds, as the real app does.
            let cap = agent::client::AgentCapturer::start(
                || async { agent::windows::connect("poc").await },
                &capture::CaptureConfig {
                    display_id: None,
                    max_fps: 5,
                    max_size: Some((640, 360)),
                    show_cursor: true,
                },
            )
            .map_err(|e| anyhow::anyhow!("{e}"))?;
            let b = cap.bounds();
            println!("agent bounds {:?}", (b.x, b.y, b.width, b.height));
            let input = agent::client::AgentInput::start(|| agent::windows::connect("poc"))?;
            input.set_area(input::Area {
                x: b.x,
                y: b.y,
                width: b.width,
                height: b.height,
            });
            input.send(protocol::InputEvent::MouseMove { x, y });
            std::thread::sleep(Duration::from_millis(300));
            println!("moved pointer to {x},{y} of {}x{}", b.width, b.height);
        }
        other => anyhow::bail!("unknown command: {other:?}"),
    }
    Ok(())
}

#[cfg(windows)]
fn write_ppm(path: &str, f: &capture::Frame) -> anyhow::Result<()> {
    use std::io::Write;
    let mut out = Vec::with_capacity((f.width * f.height * 3) as usize + 32);
    write!(out, "P6\n{} {}\n255\n", f.width, f.height)?;
    for px in f.data.chunks_exact(4) {
        out.extend_from_slice(&[px[2], px[1], px[0]]); // BGRA -> RGB
    }
    std::fs::write(path, out)?;
    Ok(())
}
