//! Shared clipboard (text and files) between viewer and host.
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

enum Command {
    Set(String),
    SetFiles(Vec<PathBuf>),
}

/// Something copied on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Text(String),
    Files(Vec<PathBuf>),
}

/// What the clipboard holds, as far as sharing is concerned.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Content {
    Text(String),
    Files(Vec<PathBuf>),
    Other,
}

fn read(clipboard: &mut arboard::Clipboard) -> Content {
    // Files first: a copied file also shows up as text (its name) on some systems.
    if let Ok(files) = clipboard.get().file_list()
        && !files.is_empty()
    {
        return Content::Files(files);
    }
    match clipboard.get_text() {
        Ok(text) if !text.is_empty() => Content::Text(text),
        _ => Content::Other,
    }
}

impl Content {
    fn into_change(self) -> Option<Change> {
        match self {
            Content::Text(t) => Some(Change::Text(t)),
            Content::Files(f) => Some(Change::Files(f)),
            Content::Other => None,
        }
    }
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
                let mut last = read(&mut clipboard);
                if include_current
                    && let Some(change) = last.clone().into_change()
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
                            last = Content::Text(text);
                        }
                        Ok(Command::SetFiles(files)) => {
                            if let Err(e) = clipboard.set().file_list(&files) {
                                tracing::warn!("cannot put files on the clipboard: {e}");
                            }
                            last = read(&mut clipboard);
                        }
                        Err(std_mpsc::RecvTimeoutError::Timeout) => {
                            let now = read(&mut clipboard);
                            if now != last {
                                last = now.clone();
                                if let Some(change) = now.into_change()
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
