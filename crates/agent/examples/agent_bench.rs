//! Frames per second from agent to app over a real named pipe, with the pixels through the
//! pipe or through shared memory (ADR 0075). 2560x1440 test pattern; run on the slow PC.
#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use agent::client::AgentCapturer;
    use agent::server::{Backend, Injector, serve_peer};
    use capture::{CaptureConfig, Capturer, TestPattern};
    use tokio::net::windows::named_pipe::{ClientOptions, ServerOptions};

    struct Pattern;
    impl Backend for Pattern {
        fn start_capture(&self, config: &CaptureConfig) -> capture::Result<Box<dyn Capturer>> {
            let (w, h) = config.max_size.unwrap();
            Ok(Box::new(TestPattern::new(w, h, config.max_fps)))
        }
        fn start_input(&self) -> anyhow::Result<Box<dyn Injector>> {
            anyhow::bail!("no input")
        }
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    for (label, peer) in [("pipe", None), ("shared memory", Some(std::process::id()))] {
        let name = format!(r"\\.\pipe\kronos-agent-bench-{}", label.len());
        let server = rt.block_on(async { ServerOptions::new().create(&name) })?;
        rt.spawn(async move {
            if server.connect().await.is_ok() {
                serve_peer(server, Arc::new(Pattern), peer).await;
            }
        });
        let config = CaptureConfig {
            max_fps: 120,
            max_size: Some((2560, 1440)),
            ..Default::default()
        };
        let pipe = name.clone();
        let mut capturer = AgentCapturer::start(
            move || async move { ClientOptions::new().open(&pipe) },
            &config,
        )?;
        let start = Instant::now();
        let mut frames = 0u32;
        while start.elapsed() < Duration::from_secs(5) {
            if capturer.next_frame(Duration::from_millis(200))?.is_some() {
                frames += 1;
            }
        }
        println!(
            "{label}: {:.1} fps",
            frames as f64 / start.elapsed().as_secs_f64()
        );
    }
    Ok(())
}

#[cfg(not(windows))]
fn main() {}
