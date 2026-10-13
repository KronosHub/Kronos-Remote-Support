//! CI smoke test: moves the pointer to a normalized position and types a key.
//!
//!   cargo run -p input --example smoke -- 0.25 0.5
//!
//! The caller checks the resulting pointer position with OS tools.

use std::time::Duration;

use protocol::InputEvent;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().with_env_filter("debug").init();
    let args: Vec<f32> = std::env::args()
        .skip(1)
        .map(|a| a.parse())
        .collect::<Result<_, _>>()?;
    let (x, y) = match args[..] {
        [x, y] => (x, y),
        _ => anyhow::bail!("usage: smoke <x 0..1> <y 0..1>"),
    };
    let sink = input::InputSink::start()?;
    sink.send(InputEvent::MouseMove { x, y });
    sink.send(InputEvent::Key {
        code: "ShiftLeft".into(),
        down: true,
    });
    // Dropping the sink must release the held Shift.
    std::thread::sleep(Duration::from_millis(300));
    drop(sink);
    println!("moved to ({x}, {y})");
    Ok(())
}
