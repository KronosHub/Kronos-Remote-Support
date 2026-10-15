//! Shared clipboard (text, files and pictures) between viewer and host.
//!
//! A dedicated thread owns the OS clipboard (on X11 the owner must stay alive for pasted
//! content to remain available) and polls it for changes; the session forwards changes to
//! the other side and writes what the other side copied.

use std::path::PathBuf;
use std::sync::mpsc as std_mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use tokio::sync::mpsc;

/// How often the local clipboard is checked for changes.
const POLL: Duration = Duration::from_millis(500);
/// Texts up to this size may go on the control channel (peers without a files channel);
/// longer ones are chunked on the files channel. Data channel messages fail beyond ~64 KiB.
pub const MAX_CONTROL_TEXT_BYTES: usize = 12 * 1024;

/// Largest picture shared, in pixels (a 6K screen is about 20 million).
const MAX_IMAGE_PIXELS: usize = 40_000_000;

enum Command {
    Set(String),
    SetFiles(Vec<PathBuf>),
    /// A PNG copied on the other side, put on the clipboard as a picture.
    SetImage(Vec<u8>),
}

/// Something copied on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Text(String),
    Files(Vec<PathBuf>),
    /// A picture (e.g. a screenshot taken to the clipboard), as PNG.
    Image(Vec<u8>),
}

/// What the clipboard holds, as far as sharing is concerned.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Content {
    Text(String),
    Files(Vec<PathBuf>),
    /// A picture, told apart by the clipboard's change count when it was read.
    Image(u64),
    Other,
}

fn read(clipboard: &mut arboard::Clipboard, stamp: Option<u64>) -> Content {
    // Files first: a copied file also shows up as text (its name) on some systems.
    if let Ok(files) = clipboard.get().file_list()
        && !files.is_empty()
    {
        return Content::Files(files);
    }
    match clipboard.get_text() {
        Ok(text) if !text.is_empty() => return Content::Text(text),
        _ => {}
    }
    // Decoding a picture is costly: only where the change count says it is a new one.
    match stamp {
        Some(stamp) if clipboard.get_image().is_ok() => Content::Image(stamp),
        _ => Content::Other,
    }
}

impl Content {
    /// The change to send; a picture is read again and encoded here.
    fn into_change(self, clipboard: &mut arboard::Clipboard) -> Option<Change> {
        match self {
            Content::Text(t) => Some(Change::Text(t)),
            Content::Files(f) => Some(Change::Files(f)),
            Content::Image(_) => match clipboard
                .get_image()
                .map_err(anyhow::Error::from)
                .and_then(encode_png)
            {
                Ok(png) => Some(Change::Image(png)),
                Err(e) => {
                    tracing::warn!("copied picture not shared: {e:#}");
                    None
                }
            },
            Content::Other => None,
        }
    }
}

fn encode_png(image: arboard::ImageData<'_>) -> anyhow::Result<Vec<u8>> {
    anyhow::ensure!(
        image.width.saturating_mul(image.height) <= MAX_IMAGE_PIXELS,
        "picture too large ({}x{})",
        image.width,
        image.height
    );
    let rgba = image::RgbaImage::from_raw(
        image.width as u32,
        image.height as u32,
        image.bytes.into_owned(),
    )
    .ok_or_else(|| anyhow::anyhow!("picture with a wrong size"))?;
    let mut png = std::io::Cursor::new(Vec::new());
    rgba.write_to(&mut png, image::ImageFormat::Png)?;
    Ok(png.into_inner())
}

fn decode_png(png: &[u8]) -> anyhow::Result<arboard::ImageData<'static>> {
    let mut reader =
        image::ImageReader::with_format(std::io::Cursor::new(png), image::ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_alloc = Some((MAX_IMAGE_PIXELS * 4) as u64);
    reader.limits(limits);
    let rgba = reader.decode()?.into_rgba8();
    let (width, height) = rgba.dimensions();
    Ok(arboard::ImageData {
        width: width as usize,
        height: height as usize,
        bytes: rgba.into_raw().into(),
    })
}

/// Counter that moves whenever anything is copied (macOS, Windows): a picture is read only
/// when it changes. `None` elsewhere: pictures are not shared there.
#[cfg(target_os = "macos")]
fn change_stamp() -> Option<u64> {
    let pasteboard = objc2_app_kit::NSPasteboard::generalPasteboard();
    Some(pasteboard.changeCount() as u64)
}

#[cfg(windows)]
fn change_stamp() -> Option<u64> {
    // SAFETY: no arguments; returns 0 when the counter is not available.
    let n = unsafe { windows_sys::Win32::System::DataExchange::GetClipboardSequenceNumber() };
    (n != 0).then_some(u64::from(n))
}

#[cfg(not(any(target_os = "macos", windows)))]
fn change_stamp() -> Option<u64> {
    None
}

pub struct ClipboardSync {
    commands: Option<std_mpsc::Sender<Command>>,
    /// What was copied locally since the sync started.
    pub changes: mpsc::UnboundedReceiver<Change>,
    thread: Option<JoinHandle<()>>,
}

impl ClipboardSync {
    /// Starts watching the clipboard. With `include_current`, the text already on the
    /// clipboard is reported as a first change (the viewer shares what it copied before
    /// connecting; the host does not volunteer its clipboard).
    pub fn start(include_current: bool) -> Option<Self> {
        let (commands, rx) = std_mpsc::channel::<Command>();
        let (changes_tx, changes) = mpsc::unbounded_channel();
        let (ready_tx, ready_rx) = std_mpsc::channel::<bool>();
        let thread = std::thread::Builder::new()
            .name("clipboard".into())
            .spawn(move || {
                let mut clipboard = match arboard::Clipboard::new() {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::warn!("clipboard unavailable: {e}");
                        let _ = ready_tx.send(false);
                        return;
                    }
                };
                let _ = ready_tx.send(true);
                let mut stamp = change_stamp();
                let mut last = read(&mut clipboard, stamp);
                if include_current
                    && let Some(change) = last.clone().into_change(&mut clipboard)
                    && changes_tx.send(change).is_err()
                {
                    return;
                }
                loop {
                    match rx.recv_timeout(POLL) {
                        // What we write is not echoed back as a local change.
                        Ok(Command::Set(text)) => {
                            if let Err(e) = clipboard.set_text(text.clone()) {
                                tracing::warn!("cannot write the clipboard: {e}");
                            }
                            stamp = change_stamp();
                            last = Content::Text(text);
                        }
                        Ok(Command::SetFiles(files)) => {
                            if let Err(e) = clipboard.set().file_list(&files) {
                                tracing::warn!("cannot put files on the clipboard: {e}");
                            }
                            stamp = change_stamp();
                            last = read(&mut clipboard, stamp);
                        }
                        Ok(Command::SetImage(png)) => {
                            match decode_png(&png) {
                                Ok(image) => {
                                    if let Err(e) = clipboard.set_image(image) {
                                        tracing::warn!(
                                            "cannot put the picture on the clipboard: {e}"
                                        );
                                    }
                                }
                                Err(e) => tracing::warn!("received picture unreadable: {e:#}"),
                            }
                            stamp = change_stamp();
                            last = read(&mut clipboard, stamp);
                        }
                        Err(std_mpsc::RecvTimeoutError::Timeout) => {
                            let now_stamp = change_stamp();
                            // Nothing copied since the last look (where the system says so).
                            if now_stamp.is_some() && now_stamp == stamp {
                                continue;
                            }
                            stamp = now_stamp;
                            let now = read(&mut clipboard, stamp);
                            if now != last {
                                last = now.clone();
                                if let Some(change) = now.into_change(&mut clipboard)
                                    && changes_tx.send(change).is_err()
                                {
                                    break;
                                }
                            }
                        }
                        Err(std_mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
            })
            .ok()?;
        if !ready_rx.recv().unwrap_or(false) {
            let _ = thread.join();
            return None;
        }
        Some(Self {
            commands: Some(commands),
            changes,
            thread: Some(thread),
        })
    }

    /// Puts text copied on the other side on this machine's clipboard.
    pub fn set(&self, text: String) {
        if let Some(commands) = &self.commands {
            let _ = commands.send(Command::Set(text));
        }
    }

    /// Puts a picture (PNG) copied on the other side on this machine's clipboard.
    pub fn set_image(&self, png: Vec<u8>) {
        if let Some(commands) = &self.commands {
            let _ = commands.send(Command::SetImage(png));
        }
    }

    /// Puts files received from the other side on this machine's clipboard.
    pub fn set_files(&self, files: Vec<PathBuf>) {
        if let Some(commands) = &self.commands {
            let _ = commands.send(Command::SetFiles(files));
        }
    }
}

impl Drop for ClipboardSync {
    fn drop(&mut self) {
        // Closing the command channel ends the thread within one poll interval; it is not
        // joined here so dropping never blocks the async runtime.
        self.commands.take();
        self.thread.take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_picture_survives_png() {
        let bytes: Vec<u8> = (0..3 * 2 * 4).map(|i| i as u8).collect();
        let image = arboard::ImageData {
            width: 3,
            height: 2,
            bytes: bytes.clone().into(),
        };
        let back = decode_png(&encode_png(image).unwrap()).unwrap();
        assert_eq!((back.width, back.height), (3, 2));
        assert_eq!(back.bytes.as_ref(), bytes.as_slice());
    }

    #[test]
    fn huge_pictures_are_refused() {
        let image = arboard::ImageData {
            width: 100_000,
            height: 1_000,
            bytes: Vec::new().into(),
        };
        assert!(encode_png(image).is_err());
    }
}
