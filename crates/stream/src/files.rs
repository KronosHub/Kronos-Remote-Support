//! Files copied to the clipboard, sent to the other side on the `files` data channel and
//! put on its clipboard once complete (paste in Finder / Explorer copies them).
//!
//! Wire format, one binary message each, in order on the reliable `files` channel:
//! - `m`… `M` + JSON [`Manifest`] — a new transfer (replaces any unfinished one); the JSON
//!   is split in chunks, `m` for all but the last;
//! - `C` + transfer `u32` LE + entry index `u32` LE + bytes — next chunk of that file;
//! - `D` + transfer `u32` LE — all chunks sent;
//! - `t` / `T` + transfer `u32` LE + UTF-8 bytes — copied text in chunks, `T` for the last
//!   one (data channel messages are limited well below what a clipboard can hold).

use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use bytes::BytesMut;
use serde::{Deserialize, Serialize};
use tokio::io::AsyncReadExt;
use transport::DataChannel;

/// Largest copied text shared (chunked on the files channel).
pub const MAX_TEXT_BYTES: usize = 1024 * 1024;

/// Upper bound for one copy: files are sent as soon as they are copied, so huge selections
/// would hog the link for nothing if never pasted.
pub const MAX_TOTAL_BYTES: u64 = 500 * 1024 * 1024;
const MAX_ENTRIES: usize = 10_000;
/// Chunk payload: data channel messages stay well below 16 KiB.
const CHUNK: usize = 15 * 1024;
/// Pause reading while this much is queued on the channel, so a big file is not read into
/// memory faster than the network drains it.
const MAX_QUEUED: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Entry {
    /// Relative path with `/` separators; the first component is a copied item's name.
    pub path: String,
    pub size: u64,
    pub dir: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    pub transfer: u32,
    pub entries: Vec<Entry>,
}

impl Manifest {
    /// Saturates instead of wrapping: hostile sizes cannot add up to a small total.
    pub fn total_bytes(&self) -> u64 {
        self.entries
            .iter()
            .fold(0u64, |total, e| total.saturating_add(e.size))
    }

    /// Number of copied items (top-level entries).
    pub fn items(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| !e.path.contains('/'))
            .count()
    }
}

/// What to send: the manifest plus the local source of each entry.
#[derive(Debug)]
pub struct Outgoing {
    pub manifest: Manifest,
    sources: Vec<PathBuf>,
}

/// Walks the copied files and folders (symbolic links are skipped).
pub fn collect(transfer: u32, paths: &[PathBuf]) -> Result<Outgoing> {
    let mut entries = Vec::new();
    let mut sources = Vec::new();
    let mut total = 0u64;
    let mut stack: Vec<(PathBuf, String)> = paths
        .iter()
        .rev()
        .filter_map(|p| Some((p.clone(), p.file_name()?.to_string_lossy().into_owned())))
        .collect();
    while let Some((source, path)) = stack.pop() {
        let meta = std::fs::symlink_metadata(&source)
            .with_context(|| format!("reading {}", source.display()))?;
        if meta.file_type().is_symlink() {
            continue;
        }
        if entries.len() >= MAX_ENTRIES {
            bail!("troppi file (massimo {MAX_ENTRIES})");
        }
        if meta.is_dir() {
            let mut children: Vec<_> = std::fs::read_dir(&source)?
                .filter_map(|e| e.ok())
                .map(|e| {
                    (
                        e.path(),
                        format!("{path}/{}", e.file_name().to_string_lossy()),
                    )
                })
                .collect();
            children.sort();
            stack.extend(children.into_iter().rev());
            entries.push(Entry {
                path,
                size: 0,
                dir: true,
            });
        } else {
            total += meta.len();
            if total > MAX_TOTAL_BYTES {
                bail!(
                    "file troppo grandi (massimo {} MB)",
                    MAX_TOTAL_BYTES / 1024 / 1024
                );
            }
            entries.push(Entry {
                path,
                size: meta.len(),
                dir: false,
            });
        }
        sources.push(source);
    }
    if entries.is_empty() {
        bail!("nessun file da inviare");
    }
    Ok(Outgoing {
        manifest: Manifest { transfer, entries },
        sources,
    })
}

/// The manifest as `m`… `M` messages, each within the data channel message limit.
fn manifest_messages(manifest: &Manifest) -> Result<Vec<BytesMut>> {
    let json = serde_json::to_vec(manifest)?;
    let chunks: Vec<&[u8]> = json.chunks(CHUNK).collect();
    Ok(chunks
        .iter()
        .enumerate()
        .map(|(i, chunk)| {
            let mut message =
                BytesMut::from(&[if i + 1 == chunks.len() { b'M' } else { b'm' }][..]);
            message.extend_from_slice(chunk);
            message
        })
        .collect())
}

fn frame(kind: u8, transfer: u32, index: Option<u32>, data: &[u8]) -> BytesMut {
    let mut out = BytesMut::with_capacity(9 + data.len());
    out.extend_from_slice(&[kind]);
    out.extend_from_slice(&transfer.to_le_bytes());
    if let Some(index) = index {
        out.extend_from_slice(&index.to_le_bytes());
    }
    out.extend_from_slice(data);
    out
}

async fn wait_for_room(channel: &Arc<dyn DataChannel>) {
    while channel.outstanding_bytes().await.unwrap_or(0) > MAX_QUEUED {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Progress of a file transfer, for a progress bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransferProgress {
    /// We send (else we receive).
    pub sending: bool,
    pub done: u64,
    pub total: u64,
    /// Recent speed.
    pub bytes_per_sec: u64,
    /// Last report of this transfer (completed, failed or cancelled).
    pub finished: bool,
}

/// How often progress is reported at most.
const PROGRESS_EVERY: Duration = Duration::from_millis(250);

/// Counts the bytes of one transfer and reports them now and then; reports `finished`
/// when dropped (also on failure or cancellation).
pub struct ProgressReporter {
    tx: tokio::sync::mpsc::UnboundedSender<TransferProgress>,
    sending: bool,
    done: u64,
    total: u64,
    /// Bytes and time at the previous report, for the speed.
    last: (u64, std::time::Instant),
    bytes_per_sec: u64,
}

impl ProgressReporter {
    pub fn new(
        tx: tokio::sync::mpsc::UnboundedSender<TransferProgress>,
        sending: bool,
        total: u64,
    ) -> Self {
        let reporter = Self {
            tx,
            sending,
            done: 0,
            total,
            last: (0, std::time::Instant::now()),
            bytes_per_sec: 0,
        };
        reporter.report(false);
        reporter
    }

    pub fn advance(&mut self, bytes: u64) {
        self.done = (self.done + bytes).min(self.total);
        let elapsed = self.last.1.elapsed();
        if elapsed >= PROGRESS_EVERY {
            let rate = (self.done - self.last.0) as f64 / elapsed.as_secs_f64();
            // Smoothed, so the number does not jump at every report.
            self.bytes_per_sec = if self.bytes_per_sec == 0 {
                rate as u64
            } else {
                (self.bytes_per_sec as f64 * 0.6 + rate * 0.4) as u64
            };
            self.last = (self.done, std::time::Instant::now());
            self.report(false);
        }
    }

    fn report(&self, finished: bool) {
        let _ = self.tx.send(TransferProgress {
            sending: self.sending,
            done: self.done,
            total: self.total,
            bytes_per_sec: self.bytes_per_sec,
            finished,
        });
    }
}

impl Drop for ProgressReporter {
    fn drop(&mut self) {
        self.report(true);
    }
}

/// Sends a whole transfer. Abort the task to cancel (the receiver drops unfinished ones).
pub async fn send(
    channel: Arc<dyn DataChannel>,
    outgoing: Outgoing,
    mut progress: Option<ProgressReporter>,
) -> Result<()> {
    let transfer = outgoing.manifest.transfer;
    for part in manifest_messages(&outgoing.manifest)? {
        channel.send(part).await?;
    }
    let mut buffer = vec![0u8; CHUNK];
    for (index, (entry, source)) in outgoing
        .manifest
        .entries
        .iter()
        .zip(&outgoing.sources)
        .enumerate()
    {
        if entry.dir {
            continue;
        }
        let mut file = tokio::fs::File::open(source)
            .await
            .with_context(|| format!("opening {}", source.display()))?;
        let mut left = entry.size;
        while left > 0 {
            let n = file.read(&mut buffer[..CHUNK.min(left as usize)]).await?;
            if n == 0 {
                bail!("{} changed while being sent", source.display());
            }
            left -= n as u64;
            wait_for_room(&channel).await;
            channel
                .send(frame(b'C', transfer, Some(index as u32), &buffer[..n]))
                .await?;
            if let Some(progress) = &mut progress {
                progress.advance(n as u64);
            }
        }
    }
    channel.send(frame(b'D', transfer, None, &[])).await?;
    Ok(())
}

/// Sends copied text in chunks.
pub async fn send_text(channel: Arc<dyn DataChannel>, transfer: u32, text: String) -> Result<()> {
    let bytes = text.as_bytes();
    let mut chunks = bytes.chunks(CHUNK).peekable();
    while let Some(chunk) = chunks.next() {
        let kind = if chunks.peek().is_some() { b't' } else { b'T' };
        wait_for_room(&channel).await;
        channel.send(frame(kind, transfer, None, chunk)).await?;
    }
    Ok(())
}

/// What a completed incoming message sequence delivers.
#[derive(Debug, PartialEq, Eq)]
pub enum Incoming {
    Files(Received),
    Text(String),
}

/// A finished incoming transfer.
#[derive(Debug, PartialEq, Eq)]
pub struct Received {
    /// The copied items (top-level files/folders) to put on the clipboard.
    pub items: Vec<PathBuf>,
    pub total_bytes: u64,
}

struct IncomingFiles {
    manifest: Manifest,
    dir: PathBuf,
    /// Destination of each entry, already validated.
    targets: Vec<PathBuf>,
    written: Vec<u64>,
    open: Option<(u32, std::fs::File)>,
}

/// Receiving side: writes transfers under `root`, one subfolder per transfer.
pub struct FileReceiver {
    /// Given, or a private folder created when the first transfer starts.
    root: Option<PathBuf>,
    /// Reports the private folder once created, for the owner to delete it.
    created: Arc<Mutex<Option<PathBuf>>>,
    current: Option<IncomingFiles>,
    /// Text being received: transfer id and bytes so far.
    text: Option<(u32, Vec<u8>)>,
    /// Manifest JSON received so far (`m` messages).
    manifest: Vec<u8>,
    /// Where to report the progress of incoming files, if anyone listens.
    progress_tx: Option<tokio::sync::mpsc::UnboundedSender<TransferProgress>>,
    progress: Option<ProgressReporter>,
}

/// A remote-supplied relative path as safe components (no `..`, no absolute or drive
/// prefixes, no separators or NUL inside a name), or `None`.
fn safe_relative(path: &str) -> Option<PathBuf> {
    let mut out = PathBuf::new();
    for part in path.split('/') {
        if part.is_empty() || part == "." || part == ".." || part.contains(['\\', ':', '\0']) {
            return None;
        }
        let mut components = Path::new(part).components();
        match (components.next(), components.next()) {
            (Some(Component::Normal(_)), None) => out.push(part),
            _ => return None,
        }
    }
    (!out.as_os_str().is_empty()).then_some(out)
}

/// A folder for one session's received files that only this user can enter: created anew
/// (never reused, so nobody can plant it first) under `$XDG_RUNTIME_DIR/kronos-clipboard`
/// when usable (Linux), else in the per-user (macOS, Windows) or shared (`/tmp`) temp folder.
pub fn private_dir() -> std::io::Result<PathBuf> {
    match runtime_base() {
        Some(base) => create_unique(&base, ""),
        None => create_unique(&std::env::temp_dir(), "kronos-clipboard-"),
    }
}

/// `$XDG_RUNTIME_DIR/kronos-clipboard`, if the runtime folder is private to this user and the
/// subfolder is a real folder (not a link) owned by the same user and closed to others.
#[cfg(unix)]
fn runtime_base() -> Option<PathBuf> {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};
    let runtime = PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?);
    if !runtime.is_absolute() {
        return None;
    }
    let meta = std::fs::symlink_metadata(&runtime).ok()?;
    if !meta.is_dir() || meta.mode() & 0o077 != 0 {
        return None;
    }
    let owner = meta.uid();
    let base = runtime.join("kronos-clipboard");
    match create_private(&base) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return None,
    }
    let meta = std::fs::symlink_metadata(&base).ok()?;
    if !meta.is_dir() || meta.uid() != owner {
        return None;
    }
    if meta.mode() & 0o077 != 0 {
        let private = std::fs::Permissions::from_mode(0o700);
        std::fs::set_permissions(&base, private).ok()?;
    }
    Some(base)
}

#[cfg(not(unix))]
fn runtime_base() -> Option<PathBuf> {
    None
}

#[cfg(unix)]
fn create_private(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new().mode(0o700).create(dir)
}

/// Windows: the temp folder is per user and new folders inherit its permissions.
#[cfg(not(unix))]
fn create_private(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir(dir)
}

/// Creates a new private folder in `base` with an unpredictable name; fails rather than
/// reuse one that already exists.
fn create_unique(base: &Path, prefix: &str) -> std::io::Result<PathBuf> {
    use std::hash::{BuildHasher, RandomState};
    let pid = std::process::id();
    for attempt in 0..16u32 {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let random = RandomState::new().hash_one((pid, attempt, nanos));
        let dir = base.join(format!("{prefix}{pid}-{random:016x}"));
        match create_private(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::AlreadyExists,
        "no free folder name for received files",
    ))
}

/// Creates the folders and empty files of a transfer whose paths were already validated.
fn create_targets(dir: &Path, entries: &[Entry], relatives: Vec<PathBuf>) -> Result<Vec<PathBuf>> {
    std::fs::create_dir_all(dir)?;
    let mut targets = Vec::with_capacity(entries.len());
    for (entry, relative) in entries.iter().zip(relatives) {
        let target = dir.join(relative);
        if entry.dir {
            std::fs::create_dir_all(&target)?;
        } else {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::File::create(&target)?;
        }
        targets.push(target);
    }
    Ok(targets)
}

impl FileReceiver {
    /// Writes under `root` (created if missing).
    pub fn new(root: PathBuf) -> Self {
        Self::with_root(Some(root), Arc::default())
    }

    /// Writes under a [`private_dir`] created only when a transfer starts; its path is then
    /// stored in `created`.
    pub fn private(created: Arc<Mutex<Option<PathBuf>>>) -> Self {
        Self::with_root(None, created)
    }

    fn with_root(root: Option<PathBuf>, created: Arc<Mutex<Option<PathBuf>>>) -> Self {
        Self {
            root,
            created,
            current: None,
            text: None,
            manifest: Vec::new(),
            progress_tx: None,
            progress: None,
        }
    }

    /// Reports the progress of incoming files to `tx`.
    pub fn with_progress(
        mut self,
        tx: tokio::sync::mpsc::UnboundedSender<TransferProgress>,
    ) -> Self {
        self.progress_tx = Some(tx);
        self
    }

    fn root(&mut self) -> Result<PathBuf> {
        if let Some(root) = &self.root {
            return Ok(root.clone());
        }
        let root = match private_dir() {
            Ok(root) => root,
            Err(e) => bail!("cannot create the folder for received files: {e}"),
        };
        if let Ok(mut created) = self.created.lock() {
            *created = Some(root.clone());
        }
        self.root = Some(root.clone());
        Ok(root)
    }

    /// Forgets anything partly received and deletes the files of the last transfer
    /// (sharing was turned off here).
    pub fn discard(&mut self) {
        self.progress = None;
        if let Some(previous) = self.current.take() {
            let _ = std::fs::remove_dir_all(&previous.dir);
        }
        self.text = None;
        self.manifest.clear();
    }

    /// Handles one message; returns the transfer once complete.
    pub fn on_message(&mut self, data: &[u8]) -> Result<Option<Incoming>> {
        let (&kind, rest) = data.split_first().context("empty message")?;
        match kind {
            b't' | b'T' => {
                let (transfer, bytes) = split_u32(rest)?;
                let buffer = match &mut self.text {
                    Some((id, buffer)) if *id == transfer => buffer,
                    _ => &mut self.text.insert((transfer, Vec::new())).1,
                };
                if buffer.len() + bytes.len() > MAX_TEXT_BYTES {
                    self.text = None;
                    bail!("copied text too large");
                }
                buffer.extend_from_slice(bytes);
                if kind == b't' {
                    return Ok(None);
                }
                let (_, buffer) = self.text.take().expect("just filled");
                Ok(Some(Incoming::Text(
                    String::from_utf8_lossy(&buffer).into_owned(),
                )))
            }
            b'm' => {
                if self.manifest.len() + rest.len() > MAX_ENTRIES * 1024 {
                    self.manifest.clear();
                    bail!("file list too large");
                }
                self.manifest.extend_from_slice(rest);
                Ok(None)
            }
            b'M' => {
                let mut json = std::mem::take(&mut self.manifest);
                json.extend_from_slice(rest);
                let manifest: Manifest = serde_json::from_slice(&json)?;
                self.start(manifest)?;
                Ok(None)
            }
            b'C' => {
                let (transfer, rest) = split_u32(rest)?;
                let (index, bytes) = split_u32(rest)?;
                self.chunk(transfer, index, bytes)?;
                Ok(None)
            }
            b'D' => {
                let (transfer, _) = split_u32(rest)?;
                Ok(self.finish(transfer)?.map(Incoming::Files))
            }
            other => bail!("unknown file message {other}"),
        }
    }

    fn start(&mut self, manifest: Manifest) -> Result<()> {
        // Only the newest copy matters: drop what came before.
        if let Some(previous) = self.current.take() {
            let _ = std::fs::remove_dir_all(&previous.dir);
        }
        if manifest.entries.len() > MAX_ENTRIES || manifest.total_bytes() > MAX_TOTAL_BYTES {
            bail!("transfer too large");
        }
        // Every path is checked before anything is written.
        let mut relatives = Vec::with_capacity(manifest.entries.len());
        for entry in &manifest.entries {
            let relative = safe_relative(&entry.path)
                .with_context(|| format!("unsafe path {:?}", entry.path))?;
            relatives.push(relative);
        }
        let dir = self.root()?.join(manifest.transfer.to_string());
        let _ = std::fs::remove_dir_all(&dir);
        let targets = match create_targets(&dir, &manifest.entries, relatives) {
            Ok(targets) => targets,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&dir);
                return Err(e);
            }
        };
        self.progress = self
            .progress_tx
            .clone()
            .map(|tx| ProgressReporter::new(tx, false, manifest.total_bytes()));
        self.current = Some(IncomingFiles {
            written: vec![0; manifest.entries.len()],
            manifest,
            dir,
            targets,
            open: None,
        });
        Ok(())
    }

    fn chunk(&mut self, transfer: u32, index: u32, bytes: &[u8]) -> Result<()> {
        use std::io::Write;
        let Some(incoming) = self
            .current
            .as_mut()
            .filter(|i| i.manifest.transfer == transfer)
        else {
            return Ok(()); // superseded transfer
        };
        let i = index as usize;
        let entry = incoming
            .manifest
            .entries
            .get(i)
            .context("bad entry index")?;
        if entry.dir || incoming.written[i] + bytes.len() as u64 > entry.size {
            bail!("more data than announced for {:?}", entry.path);
        }
        if incoming.open.as_ref().map(|(n, _)| *n) != Some(index) {
            let file = std::fs::OpenOptions::new()
                .append(true)
                .open(&incoming.targets[i])?;
            incoming.open = Some((index, file));
        }
        let (_, file) = incoming.open.as_mut().expect("just opened");
        file.write_all(bytes)?;
        incoming.written[i] += bytes.len() as u64;
        if let Some(progress) = &mut self.progress {
            progress.advance(bytes.len() as u64);
        }
        Ok(())
    }

    /// The completed transfer; `None` for one superseded or discarded meanwhile.
    fn finish(&mut self, transfer: u32) -> Result<Option<Received>> {
        // Done (or failing below): the bar goes away either way.
        self.progress = None;
        let Some(incoming) = self
            .current
            .as_mut()
            .filter(|i| i.manifest.transfer == transfer)
        else {
            return Ok(None);
        };
        incoming.open = None;
        for (entry, written) in incoming.manifest.entries.iter().zip(&incoming.written) {
            if !entry.dir && *written != entry.size {
                bail!("{:?} incomplete", entry.path);
            }
        }
        let items = incoming
            .manifest
            .entries
            .iter()
            .zip(&incoming.targets)
            .filter(|(e, _)| !e.path.contains('/'))
            .map(|(_, t)| t.clone())
            .collect();
        Ok(Some(Received {
            items,
            total_bytes: incoming.manifest.total_bytes(),
        }))
    }
}

fn split_u32(data: &[u8]) -> Result<(u32, &[u8])> {
    let (head, rest) = data.split_at_checked(4).context("truncated message")?;
    Ok((u32::from_le_bytes(head.try_into().expect("4 bytes")), rest))
}

/// Encodes a transfer into the messages [`send`] would produce (tests, no channel).
#[cfg(test)]
fn encode(outgoing: &Outgoing) -> Vec<BytesMut> {
    let transfer = outgoing.manifest.transfer;
    let mut out = manifest_messages(&outgoing.manifest).unwrap();
    for (index, (entry, source)) in outgoing
        .manifest
        .entries
        .iter()
        .zip(&outgoing.sources)
        .enumerate()
    {
        if entry.dir {
            continue;
        }
        let data = std::fs::read(source).unwrap();
        for chunk in data.chunks(CHUNK) {
            out.push(frame(b'C', transfer, Some(index as u32), chunk));
        }
    }
    out.push(frame(b'D', transfer, None, &[]));
    out
}

#[cfg(test)]
mod tests {

    #[test]
    fn progress_reports_start_and_finish() {
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let mut progress = ProgressReporter::new(tx, true, 100);
        progress.advance(60);
        progress.advance(80); // capped at the total
        drop(progress);
        let reports: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
        assert_eq!(
            reports.first().map(|p| (p.done, p.finished)),
            Some((0, false))
        );
        let last = reports.last().unwrap();
        assert!(last.finished && last.sending);
        assert_eq!((last.done, last.total), (100, 100));
    }
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("krs-files-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn folder_and_file_roundtrip() {
        let src = temp("src");
        std::fs::write(src.join("note.txt"), b"ciao").unwrap();
        std::fs::create_dir_all(src.join("cartella/sotto")).unwrap();
        let big: Vec<u8> = (0..100_000u32).map(|i| i as u8).collect();
        std::fs::write(src.join("cartella/sotto/dati.bin"), &big).unwrap();
        std::fs::create_dir_all(src.join("cartella/vuota")).unwrap();

        let outgoing = collect(7, &[src.join("note.txt"), src.join("cartella")]).unwrap();
        assert_eq!(outgoing.manifest.items(), 2);
        assert_eq!(outgoing.manifest.total_bytes(), 100_004);

        let dst = temp("dst");
        let mut receiver = FileReceiver::new(dst.clone());
        let mut done = None;
        for message in encode(&outgoing) {
            if let Some(Incoming::Files(received)) = receiver.on_message(&message).unwrap() {
                done = Some(received);
            }
        }
        let done = done.expect("transfer completed");
        assert_eq!(done.items, [dst.join("7/note.txt"), dst.join("7/cartella")]);
        assert_eq!(std::fs::read(dst.join("7/note.txt")).unwrap(), b"ciao");
        assert_eq!(
            std::fs::read(dst.join("7/cartella/sotto/dati.bin")).unwrap(),
            big
        );
        assert!(dst.join("7/cartella/vuota").is_dir());
        std::fs::remove_dir_all(src).unwrap();
        std::fs::remove_dir_all(dst).unwrap();
    }

    #[test]
    fn many_files_split_the_manifest() {
        let src = temp("many");
        for i in 0..600 {
            std::fs::write(
                src.join(format!("documento-con-un-nome-lungo-{i:04}.txt")),
                b"x",
            )
            .unwrap();
        }
        let outgoing = collect(3, std::slice::from_ref(&src)).unwrap();
        assert!(manifest_messages(&outgoing.manifest).unwrap().len() > 1);
        let dst = temp("many-dst");
        let mut receiver = FileReceiver::new(dst.clone());
        let mut done = None;
        for message in encode(&outgoing) {
            if let Some(Incoming::Files(received)) = receiver.on_message(&message).unwrap() {
                done = Some(received);
            }
        }
        assert_eq!(done.unwrap().total_bytes, 600);
        std::fs::remove_dir_all(src).unwrap();
        std::fs::remove_dir_all(dst).unwrap();
    }

    #[test]
    fn long_text_in_chunks() {
        let text: String = "àbc€ ".repeat(20_000); // multi-byte characters across chunks
        let mut receiver = FileReceiver::new(temp("text"));
        let bytes = text.as_bytes();
        let chunks: Vec<_> = bytes.chunks(CHUNK).collect();
        let mut got = None;
        for (i, chunk) in chunks.iter().enumerate() {
            let kind = if i + 1 == chunks.len() { b'T' } else { b't' };
            got = receiver.on_message(&frame(kind, 9, None, chunk)).unwrap();
        }
        assert_eq!(got, Some(Incoming::Text(text)));
    }

    #[test]
    fn hostile_paths_are_refused() {
        for bad in [
            "../evil",
            "a/../../evil",
            "/etc/passwd",
            "C:\\x",
            "a\\..\\b",
            "",
            "a//b",
            "./a",
        ] {
            assert_eq!(safe_relative(bad), None, "{bad:?}");
        }
        assert_eq!(
            safe_relative("a/b c.txt"),
            Some(PathBuf::from("a").join("b c.txt"))
        );

        let dst = temp("hostile");
        let mut receiver = FileReceiver::new(dst.clone());
        let manifest = Manifest {
            transfer: 1,
            entries: vec![Entry {
                path: "../../evil".into(),
                size: 1,
                dir: false,
            }],
        };
        let mut message = b"M".to_vec();
        message.extend(serde_json::to_vec(&manifest).unwrap());
        assert!(receiver.on_message(&message).is_err());
        assert!(!dst.parent().unwrap().join("evil").exists());
        std::fs::remove_dir_all(dst).unwrap();
    }

    #[test]
    fn more_data_than_announced_is_refused() {
        let dst = temp("overflow");
        let mut receiver = FileReceiver::new(dst.clone());
        let manifest = Manifest {
            transfer: 2,
            entries: vec![Entry {
                path: "a".into(),
                size: 3,
                dir: false,
            }],
        };
        let mut message = b"M".to_vec();
        message.extend(serde_json::to_vec(&manifest).unwrap());
        receiver.on_message(&message).unwrap();
        assert!(
            receiver
                .on_message(&frame(b'C', 2, Some(0), b"abcd"))
                .is_err()
        );
        std::fs::remove_dir_all(dst).unwrap();
    }

    fn manifest_message(manifest: &Manifest) -> Vec<u8> {
        let mut message = b"M".to_vec();
        message.extend(serde_json::to_vec(manifest).unwrap());
        message
    }

    #[test]
    fn nothing_is_written_when_a_later_path_is_unsafe() {
        let dst = temp("late-hostile");
        let mut receiver = FileReceiver::new(dst.clone());
        let manifest = Manifest {
            transfer: 4,
            entries: vec![
                Entry {
                    path: "ok.txt".into(),
                    size: 1,
                    dir: false,
                },
                Entry {
                    path: "a/../../evil".into(),
                    size: 1,
                    dir: false,
                },
            ],
        };
        let message = manifest_message(&manifest);
        assert!(receiver.on_message(&message).is_err());
        assert!(!dst.join("4").exists());
        std::fs::remove_dir_all(dst).unwrap();
    }

    #[test]
    fn sizes_cannot_wrap_around_the_limit() {
        let dst = temp("wrap");
        let mut receiver = FileReceiver::new(dst.clone());
        let manifest = Manifest {
            transfer: 5,
            entries: vec![
                Entry {
                    path: "a".into(),
                    size: u64::MAX,
                    dir: false,
                },
                Entry {
                    path: "b".into(),
                    size: 2,
                    dir: false,
                },
            ],
        };
        assert_eq!(manifest.total_bytes(), u64::MAX);
        let message = manifest_message(&manifest);
        assert!(receiver.on_message(&message).is_err());
        assert!(!dst.join("5").exists());
        std::fs::remove_dir_all(dst).unwrap();
    }

    #[test]
    fn discard_drops_the_transfer_and_its_files() {
        let dst = temp("discard");
        let mut receiver = FileReceiver::new(dst.clone());
        let manifest = Manifest {
            transfer: 6,
            entries: vec![Entry {
                path: "a".into(),
                size: 3,
                dir: false,
            }],
        };
        let message = manifest_message(&manifest);
        receiver.on_message(&message).unwrap();
        assert!(dst.join("6/a").exists());
        receiver.discard();
        assert!(!dst.join("6").exists());
        // Late chunks and the end of the discarded transfer are ignored.
        let chunk = frame(b'C', 6, Some(0), b"abc");
        assert_eq!(receiver.on_message(&chunk).unwrap(), None);
        let done = frame(b'D', 6, None, &[]);
        assert_eq!(receiver.on_message(&done).unwrap(), None);
        assert!(!dst.join("6").exists());
        std::fs::remove_dir_all(dst).unwrap();
    }

    #[test]
    fn private_folder_is_created_only_for_files() {
        let created = Arc::new(Mutex::new(None));
        let mut receiver = FileReceiver::private(created.clone());
        let text = frame(b'T', 1, None, b"ciao");
        let got = receiver.on_message(&text).unwrap();
        assert_eq!(got, Some(Incoming::Text("ciao".into())));
        assert_eq!(*created.lock().unwrap(), None);

        let manifest = Manifest {
            transfer: 2,
            entries: vec![Entry {
                path: "a".into(),
                size: 0,
                dir: false,
            }],
        };
        let message = manifest_message(&manifest);
        receiver.on_message(&message).unwrap();
        let root = created.lock().unwrap().clone().expect("folder created");
        assert!(root.join("2/a").is_file());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = std::fs::symlink_metadata(&root).unwrap();
            assert_eq!(meta.permissions().mode() & 0o777, 0o700);
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn private_folders_are_new_and_distinct() {
        let a = private_dir().unwrap();
        let b = private_dir().unwrap();
        assert_ne!(a, b);
        assert!(a.is_dir() && b.is_dir());
        std::fs::remove_dir_all(a).unwrap();
        std::fs::remove_dir_all(b).unwrap();
    }
}
