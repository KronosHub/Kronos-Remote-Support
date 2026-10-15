//! Kronos Virtual Monitor: gives a Windows PC without a monitor a 1920×1080 virtual one, so
//! Kronos Remote Support has a screen to share (ADR 0049).
//!
//! It installs Virtual Display Driver (MIT, signed by SignPath Foundation), bundled at build
//! time by `fetch-driver.sh`, and sets it to a single 1920×1080 monitor at 60 Hz.
//!
//! Usage: double-click for the menu, or `KronosVirtualMonitor install|uninstall`.

#[cfg(windows)]
mod win;

fn main() {
    #[cfg(windows)]
    std::process::exit(win::run());
    #[cfg(not(windows))]
    {
        eprintln!("Kronos Virtual Monitor funziona solo su Windows.");
        std::process::exit(1);
    }
}
