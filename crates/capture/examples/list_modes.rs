//! Prints the resolutions each display offers a viewer (ADR 0069).
fn main() {
    for d in capture::displays().expect("displays") {
        match capture::resolution::modes(d.id) {
            Ok((modes, current)) => {
                println!(
                    "{} ({}) now {}x{} @{}",
                    d.name, d.id, current.width, current.height, current.scale
                );
                for m in modes {
                    println!("  {}x{} scale {}", m.width, m.height, m.scale);
                }
            }
            Err(e) => println!("{} ({}): {e}", d.name, d.id),
        }
    }
}
