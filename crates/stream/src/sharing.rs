//! Clipboard sharing within one session: text goes on the control channel, copied files on
//! the `files` channel (see [`crate::files`]). Used by both the host and the viewer.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use protocol::ControlMsg;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use transport::{DataChannel, DataChannelEvent};

use crate::clipboard::{self, Change, ClipboardSync};
use crate::files::{self, FileReceiver, Incoming, ProgressReporter, TransferProgress};

pub(crate) enum ClipEvent {
    Local(Option<Change>),
    FilesChannel(Option<Arc<dyn DataChannel>>),
    Received(Option<anyhow::Result<Incoming>>),
    /// A file transfer moved on (for the progress bar; not passed to `handle`).
    Progress(TransferProgress),
}

pub(crate) struct ClipboardShare {
    sync: Option<ClipboardSync>,
    files_pending: Option<oneshot::Receiver<Arc<dyn DataChannel>>>,
    files: Option<Arc<dyn DataChannel>>,
    incoming: Option<mpsc::UnboundedReceiver<anyhow::Result<Incoming>>>,
    reader: Option<JoinHandle<()>>,
    sending: Option<JoinHandle<()>>,
    next_transfer: u32,
    /// Private folder holding the received files until the session ends, once created by
    /// the reader (only when a transfer arrives while sharing is on).
    received_dir: Arc<Mutex<Option<PathBuf>>>,
    /// Whether the reader keeps what the peer sends: only while sharing is on here.
    accepting: Arc<AtomicBool>,
    /// Progress of the files we send and receive.
    progress_tx: mpsc::UnboundedSender<TransferProgress>,
    progress: mpsc::UnboundedReceiver<TransferProgress>,
}

fn megabytes(bytes: u64) -> String {
    format!("{:.1} MB", bytes as f64 / 1_000_000.0).replace('.', ",")
}

fn items(n: usize) -> String {
    if n == 1 {
        "1 elemento".into()
    } else {
        format!("{n} elementi")
    }
}

impl ClipboardShare {
    pub(crate) fn new(files: oneshot::Receiver<Arc<dyn DataChannel>>) -> Self {
        let (progress_tx, progress) = mpsc::unbounded_channel();
        Self {
            progress_tx,
            progress,
            sync: None,
            files_pending: Some(files),
            files: None,
            incoming: None,
            reader: None,
            sending: None,
            next_transfer: 1,
            received_dir: Arc::default(),
            accepting: Arc::default(),
        }
    }

    pub(crate) fn enabled(&self) -> bool {
        self.sync.is_some()
    }

    /// Starts or stops watching the clipboard (touched off the async runtime). With
    /// `include_current`, what is already copied is shared too.
    pub(crate) async fn set_enabled(&mut self, enabled: bool, include_current: bool) {
        if !enabled {
            self.sync = None;
            if let Some(task) = self.sending.take() {
                task.abort();
            }
        } else if self.sync.is_none() {
            // Accept from now: what the peer sends right after turning sharing on is kept.
            self.accepting.store(true, Ordering::Relaxed);
            self.sync = tokio::task::spawn_blocking(move || ClipboardSync::start(include_current))
                .await
                .ok()
                .flatten();
            tracing::info!(available = self.sync.is_some(), "clipboard sharing on");
        }
        self.update_accepting();
    }

    fn update_accepting(&self) {
        let on = self.sync.is_some();
        self.accepting.store(on, Ordering::Relaxed);
    }

    /// Text copied on the other side.
    pub(crate) fn remote_text(&self, text: String) {
        tracing::debug!(
            bytes = text.len(),
            on = self.sync.is_some(),
            "clipboard text received"
        );
        if let Some(sync) = &self.sync {
            sync.set(text);
        }
    }

    pub(crate) async fn next(&mut self) -> ClipEvent {
        let (sync, pending, incoming, progress) = (
            &mut self.sync,
            &mut self.files_pending,
            &mut self.incoming,
            &mut self.progress,
        );
        tokio::select! {
            // Never closes: this struct keeps a sender.
            Some(progress) = progress.recv() => ClipEvent::Progress(progress),
            change = async {
                match sync {
                    Some(sync) => sync.changes.recv().await,
                    None => std::future::pending().await,
                }
            } => ClipEvent::Local(change),
            channel = async {
                match pending {
                    Some(rx) => rx.await.ok(),
                    None => std::future::pending().await,
                }
            } => ClipEvent::FilesChannel(channel),
            received = async {
                match incoming {
                    Some(rx) => rx.recv().await,
                    None => std::future::pending().await,
                }
            } => ClipEvent::Received(received),
        }
    }

    /// Acts on an event; returns a message for the user, if any.
    pub(crate) async fn handle(
        &mut self,
        event: ClipEvent,
        control: &Arc<dyn DataChannel>,
    ) -> Option<String> {
        match event {
            ClipEvent::Local(None) => {
                self.sync = None;
                self.update_accepting();
                None
            }
            ClipEvent::Local(Some(Change::Text(text))) => {
                tracing::debug!(bytes = text.len(), "clipboard text sent");
                if text.len() > files::MAX_TEXT_BYTES {
                    return Some("testo copiato troppo lungo per gli appunti condivisi".into());
                }
                match self.files.clone() {
                    Some(channel) => {
                        let transfer = self.take_transfer_id();
                        if let Some(previous) = self.sending.take() {
                            previous.abort();
                        }
                        self.sending = Some(tokio::spawn(async move {
                            if let Err(e) = files::send_text(channel, transfer, text).await {
                                tracing::warn!("clipboard text not sent: {e:#}");
                            }
                        }));
                    }
                    // Peer without a files channel: only short texts fit in one message.
                    None if text.len() <= clipboard::MAX_CONTROL_TEXT_BYTES => {
                        let msg = ControlMsg::Clipboard { text };
                        let _ = control.send_text(&msg.to_json()).await;
                    }
                    None => return Some("testo copiato troppo lungo per il partner".into()),
                }
                None
            }
            ClipEvent::Local(Some(Change::Files(paths))) => {
                tracing::debug!(count = paths.len(), "clipboard files copied");
                self.send_files(paths).await
            }
            ClipEvent::FilesChannel(None) => {
                self.files_pending = None;
                None
            }
            ClipEvent::FilesChannel(Some(channel)) => {
                self.files_pending = None;
                self.start_reader(channel.clone());
                self.files = Some(channel);
                None
            }
            ClipEvent::Received(None) => {
                self.incoming = None;
                None
            }
            ClipEvent::Received(Some(Err(e))) => {
                Some(format!("file degli appunti non ricevuti: {e:#}"))
            }
            ClipEvent::Received(Some(Ok(Incoming::Text(text)))) => {
                self.remote_text(text);
                None
            }
            ClipEvent::Progress(_) => None,
            ClipEvent::Received(Some(Ok(Incoming::Files(received)))) => {
                let sync = self.sync.as_ref()?;
                let n = received.items.len();
                sync.set_files(received.items);
                Some(format!(
                    "{} ricevuti negli appunti ({}): incolla dove vuoi",
                    items(n),
                    megabytes(received.total_bytes)
                ))
            }
        }
    }

    async fn send_files(&mut self, paths: Vec<PathBuf>) -> Option<String> {
        let Some(channel) = self.files.clone() else {
            return Some("il partner non può ricevere file: va aggiornata la sua app".into());
        };
        let transfer = self.take_transfer_id();
        let outgoing =
            match tokio::task::spawn_blocking(move || files::collect(transfer, &paths)).await {
                Ok(Ok(outgoing)) => outgoing,
                Ok(Err(e)) => return Some(format!("file non condivisi: {e:#}")),
                Err(_) => return None,
            };
        let message = format!(
            "invio di {} ({}) negli appunti del partner",
            items(outgoing.manifest.items()),
            megabytes(outgoing.manifest.total_bytes())
        );
        if let Some(previous) = self.sending.take() {
            previous.abort();
        }
        let progress = ProgressReporter::new(
            self.progress_tx.clone(),
            true,
            outgoing.manifest.total_bytes(),
        );
        self.sending = Some(tokio::spawn(async move {
            if let Err(e) = files::send(channel, outgoing, Some(progress)).await {
                tracing::warn!("clipboard files not sent: {e:#}");
            }
        }));
        Some(message)
    }

    fn take_transfer_id(&mut self) -> u32 {
        let id = self.next_transfer;
        self.next_transfer = self.next_transfer.wrapping_add(1);
        id
    }

    fn start_reader(&mut self, channel: Arc<dyn DataChannel>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let mut receiver = FileReceiver::private(self.received_dir.clone())
            .with_progress(self.progress_tx.clone());
        let accepting = self.accepting.clone();
        self.incoming = Some(rx);
        self.reader = Some(tokio::spawn(async move {
            while let Some(event) = channel.poll().await {
                match event {
                    DataChannelEvent::OnMessage(message) => {
                        // Sharing off here: nothing the peer sends is kept, not even on disk.
                        if !accepting.load(Ordering::Relaxed) {
                            receiver.discard();
                            continue;
                        }
                        // Chunks are small: writing one blocks only briefly.
                        let report = match receiver.on_message(&message.data) {
                            Ok(Some(incoming)) => Ok(incoming),
                            Ok(None) => continue,
                            Err(e) => Err(e),
                        };
                        if tx.send(report).is_err() {
                            break;
                        }
                    }
                    DataChannelEvent::OnClose => break,
                    _ => {}
                }
            }
        }));
    }
}

impl Drop for ClipboardShare {
    fn drop(&mut self) {
        for task in [self.reader.take(), self.sending.take()]
            .into_iter()
            .flatten()
        {
            task.abort();
        }
        if let Ok(mut dir) = self.received_dir.lock()
            && let Some(path) = dir.take()
        {
            let _ = std::fs::remove_dir_all(path);
        }
    }
}
