//! Prints the pointer shape changes for a while and saves the pictures next to the log.
//! `cursor_probe <dir> [seconds]`. On Windows it also draws every system cursor.
use std::time::{Duration, Instant};

use stream::cursor::{Shape, Watcher};

fn save(dir: &std::path::Path, name: &str, shape: &Shape) -> String {
    match shape {
        Shape::Named(css) => format!("named {css}"),
        Shape::Image {
            png,
            hot_x,
            hot_y,
            scale,
        } => {
            let _ = std::fs::write(dir.join(format!("{name}.png")), png);
            format!(
                "picture {} bytes, hot {hot_x},{hot_y}, x{scale} -> {name}.png",
                png.len()
            )
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let dir = std::path::PathBuf::from(args.first().map_or(".", String::as_str));
    let seconds: u64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(10);
    std::fs::create_dir_all(&dir).unwrap();
    #[cfg(windows)]
    println!(
        "on the user's desktop: {}",
        stream::cursor::on_user_desktop()
    );
    #[cfg(windows)]
    for (name, shape) in stream::cursor::system_pictures() {
        match shape {
            Some(shape) => println!(
                "system {name}: {}",
                save(&dir, &format!("system-{name}"), &shape)
            ),
            None => println!("system {name}: not drawn"),
        }
    }
    let mut watcher = Watcher::new();
    let start = Instant::now();
    let mut n = 0;
    while start.elapsed() < Duration::from_secs(seconds) {
        if let Some(shape) = watcher.changed() {
            n += 1;
            println!(
                "{:>6} ms: {}",
                start.elapsed().as_millis(),
                save(&dir, &format!("change-{n}"), &shape)
            );
        }
        std::thread::sleep(stream::cursor::SAMPLE_INTERVAL);
    }
}
