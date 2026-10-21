//! Remote file manager (ADR 0053): the viewer browses the host's folders and copies files
//! in both directions on the `fs` data channel, apart from the clipboard's.
//!
//! Wire format, one binary message each on the reliable, ordered `fs` channel:
//! - `j`… `J` + JSON [`FsMsg`], split in chunks (`j` for all but the last);
//! - `C` + request `u32` LE + entry index `u32` LE + bytes — next chunk of a file;
//! - `D` + request `u32` LE — all files of that request sent.
//!
//! Files arrive in a hidden folder inside the destination and are moved into it once
//! complete, under a new name ("name (2).ext") or, when the user chose to overwrite,
//! replacing what has the same name.

use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use bytes::BytesMut;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use transport::{DataChannel, DataChannelEvent};

use crate::files::{self, Entry, ProgressReporter, TransferProgress};
use crate::sharing::{items, megabytes};

/// Most entries in one listing (the rest is left out and the listing marked `truncated`).
const MAX_LISTED: usize = 5_000;
/// Most files and folders in one copy.
const MAX_ENTRIES: usize = 100_000;
/// Largest JSON message (a copy of many files announces all their names).
const MAX_JSON: usize = 32 * 1024 * 1024;
/// Wait for the answer to a listing or a new folder.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(10);
/// Answer that never came: most likely a host whose app predates the file manager.
const NO_ANSWER: &str = "nessuna risposta dal computer controllato: aggiorna la sua app alla versione 1.8.1 o successiva";

/// A file or folder in a listing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FsEntry {
    pub name: String,
    /// Full path on the computer it is on.
    pub path: String,
    pub dir: bool,
    pub size: u64,
    /// Last change, in seconds since 1970.
    #[serde(default)]
    pub modified: Option<u64>,
    /// Creation, in seconds since 1970 (not on every file system; older hosts omit it).
    #[serde(default)]
    pub created: Option<u64>,
    /// Starts with a dot, or has the Windows "hidden" attribute.
    #[serde(default)]
    pub hidden: bool,
}

/// The contents of one folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listing {
    pub path: String,
    pub parent: Option<String>,
    /// Home folder of the user running the app.
    pub home: Option<String>,
    /// Drives (Windows) or `/`.
    pub roots: Vec<String>,
    /// Folders first, then files, by name.
    pub entries: Vec<FsEntry>,
    /// More than [`MAX_LISTED`] entries: only the first ones are listed.
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
enum FsMsg {
    /// Viewer: list a folder (`None`: the home folder).
    List {
        req: u32,
        path: Option<String>,
    },
    Listing {
        req: u32,
        listing: Listing,
    },
    /// Viewer: create this folder.
    Mkdir {
        req: u32,
        path: String,
    },
    /// Viewer: send me these files and folders.
    Download {
        req: u32,
        paths: Vec<String>,
    },
    /// Host: the files of a download follow.
    Sending {
        req: u32,
        entries: Vec<Entry>,
    },
    /// Viewer: these files follow, put them in `dest`; `replace`: items with the same name
    /// there are overwritten (otherwise the new ones get a free name). Older hosts ignore it.
    Upload {
        req: u32,
        dest: String,
        entries: Vec<Entry>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        replace: bool,
    },
    /// Done; for a copy, the names the items got in the destination.
    Done {
        req: u32,
        #[serde(default)]
        names: Vec<String>,
    },
    Error {
        req: u32,
        message: String,
    },
    /// Viewer: stop that copy.
    Cancel {
        req: u32,
    },
}

/// The home folder of the user running the app.
pub fn home_dir() -> Option<PathBuf> {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
}

/// Lists a folder (`None` or empty: the home folder).
pub fn list_dir(path: Option<&str>) -> Result<Listing, String> {
    let home = home_dir();
    let dir = match path.map(str::trim).filter(|p| !p.is_empty()) {
        Some(path) => PathBuf::from(path),
        None => home.clone().ok_or("cartella home non trovata")?,
    };
    let dir = std::path::absolute(&dir).unwrap_or(dir);
    let read = std::fs::read_dir(&dir)
        .map_err(|e| format!("impossibile aprire {}: {}", dir.display(), io_message(&e)))?;
    let mut entries = Vec::new();
    let mut truncated = false;
    for item in read.flatten() {
        if entries.len() >= MAX_LISTED {
            truncated = true;
            break;
        }
        let path = item.path();
        // Follows links, so a linked folder opens like a folder; broken links are skipped.
        let Ok(meta) = std::fs::metadata(&path) else {
            continue;
        };
        let name = item.file_name().to_string_lossy().into_owned();
        entries.push(FsEntry {
            hidden: is_hidden(&name, &meta),
            path: path.to_string_lossy().into_owned(),
            dir: meta.is_dir(),
            size: if meta.is_dir() { 0 } else { meta.len() },
            modified: meta.modified().ok().and_then(unix_secs),
            created: meta.created().ok().and_then(unix_secs),
            name,
        });
    }
    entries.sort_by(|a, b| {
        b.dir
            .cmp(&a.dir)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(Listing {
        path: dir.to_string_lossy().into_owned(),
        parent: dir.parent().map(|p| p.to_string_lossy().into_owned()),
        home: home.map(|h| h.to_string_lossy().into_owned()),
        roots: roots(),
        entries,
        truncated,
    })
}

fn unix_secs(time: std::time::SystemTime) -> Option<u64> {
    time.duration_since(std::time::UNIX_EPOCH)
        .ok()
        .map(|d| d.as_secs())
}

/// Creates a folder (not its parents: they are the folder being shown).
pub fn make_dir(path: &str) -> Result<(), String> {
    std::fs::create_dir(path).map_err(|e| format!("cartella non creata: {}", io_message(&e)))
}

fn io_message(e: &std::io::Error) -> String {
    match e.kind() {
        std::io::ErrorKind::PermissionDenied if cfg!(target_os = "macos") => {
            "permesso negato (su macOS consenti l'accesso a file e cartelle a Kronos Remote \
             Support in Impostazioni di Sistema > Privacy e sicurezza)"
                .into()
        }
        std::io::ErrorKind::PermissionDenied => "permesso negato".into(),
        std::io::ErrorKind::NotFound => "non esiste".into(),
        std::io::ErrorKind::AlreadyExists => "esiste già".into(),
        _ => e.to_string(),
    }
}

#[cfg(windows)]
fn is_hidden(name: &str, meta: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    name.starts_with('.') || meta.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0
}

#[cfg(not(windows))]
fn is_hidden(name: &str, _meta: &std::fs::Metadata) -> bool {
    name.starts_with('.')
}

#[cfg(windows)]
fn roots() -> Vec<String> {
    (b'C'..=b'Z')
        .map(|letter| format!("{}:\\", letter as char))
        .filter(|root| Path::new(root).is_dir())
        .collect()
}

#[cfg(not(windows))]
fn roots() -> Vec<String> {
    vec!["/".into()]
}

/// The messages carrying one JSON message.
fn json_messages(msg: &FsMsg) -> Result<Vec<BytesMut>> {
    let json = serde_json::to_vec(msg)?;
    let chunks: Vec<&[u8]> = json.chunks(files::CHUNK).collect();
    Ok(chunks
        .iter()
        .enumerate()
        .map(|(i, chunk)| {
            let mut message =
                BytesMut::from(&[if i + 1 == chunks.len() { b'J' } else { b'j' }][..]);
            message.extend_from_slice(chunk);
            message
        })
        .collect())
}

/// Sends a JSON message; `lock` keeps the parts of two messages from interleaving.
async fn send_msg(
    channel: &Arc<dyn DataChannel>,
    lock: &tokio::sync::Mutex<()>,
    msg: &FsMsg,
) -> Result<()> {
    let messages = json_messages(msg)?;
    let _parts_in_order = lock.lock().await;
    for message in messages {
        files::wait_for_room(channel).await;
        channel.send(message).await?;
    }
    Ok(())
}

enum Frame<'a> {
    Msg(FsMsg),
    Chunk {
        req: u32,
        index: u32,
        bytes: &'a [u8],
    },
    End(u32),
}

#[derive(Default)]
struct Decoder {
    json: Vec<u8>,
}

impl Decoder {
    fn feed<'a>(&mut self, data: &'a [u8]) -> Result<Option<Frame<'a>>> {
        let (&kind, rest) = data.split_first().context("empty message")?;
        match kind {
            b'j' => {
                if self.json.len() + rest.len() > MAX_JSON {
                    self.json.clear();
                    bail!("message too large");
                }
                self.json.extend_from_slice(rest);
                Ok(None)
            }
            b'J' => {
                let mut json = std::mem::take(&mut self.json);
                json.extend_from_slice(rest);
                Ok(Some(Frame::Msg(serde_json::from_slice(&json)?)))
            }
            b'C' => {
                let (req, rest) = files::split_u32(rest)?;
                let (index, bytes) = files::split_u32(rest)?;
                Ok(Some(Frame::Chunk { req, index, bytes }))
            }
            b'D' => Ok(Some(Frame::End(files::split_u32(rest)?.0))),
            other => bail!("unknown file manager message {other}"),
        }
    }
}

/// Files of one copy being received.
struct Incoming {
    req: u32,
    dest: PathBuf,
    /// Hidden folder in `dest` holding the files until they are complete.
    staging: PathBuf,
    entries: Vec<Entry>,
    targets: Vec<PathBuf>,
    written: Vec<u64>,
    open: Option<(u32, std::fs::File)>,
    progress: Option<ProgressReporter>,
    /// Overwrite items with the same name in `dest`.
    replace: bool,
}

impl Incoming {
    fn start(
        req: u32,
        dest: &Path,
        entries: Vec<Entry>,
        progress: Option<mpsc::UnboundedSender<TransferProgress>>,
        replace: bool,
    ) -> Result<Self> {
        if entries.is_empty() {
            bail!("nessun file da copiare");
        }
        if entries.len() > MAX_ENTRIES {
            bail!("troppi file (massimo {MAX_ENTRIES})");
        }
        if !std::fs::metadata(dest).is_ok_and(|m| m.is_dir()) {
            bail!("la cartella {} non esiste", dest.display());
        }
        // Every path is checked before anything is written.
        let mut relatives = Vec::with_capacity(entries.len());
        for entry in &entries {
            let relative = files::safe_relative(&entry.path)
                .with_context(|| format!("percorso non valido {:?}", entry.path))?;
            relatives.push(relative);
        }
        let staging = files::create_unique(dest, ".kronos-in-").map_err(|e| {
            anyhow::anyhow!(
                "impossibile scrivere in {}: {}",
                dest.display(),
                io_message(&e)
            )
        })?;
        let targets = match files::create_targets(&staging, &entries, relatives) {
            Ok(targets) => targets,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&staging);
                return Err(e);
            }
        };
        let total = total_bytes(&entries);
        Ok(Self {
            req,
            dest: dest.to_owned(),
            staging,
            written: vec![0; entries.len()],
            entries,
            targets,
            open: None,
            progress: progress.map(|tx| ProgressReporter::new(tx, false, total)),
            replace,
        })
    }

    fn chunk(&mut self, index: u32, bytes: &[u8]) -> Result<()> {
        use std::io::Write;
        let i = index as usize;
        let entry = self.entries.get(i).context("bad entry index")?;
        if entry.dir || self.written[i] + bytes.len() as u64 > entry.size {
            bail!("more data than announced for {:?}", entry.path);
        }
        if self.open.as_ref().map(|(n, _)| *n) != Some(index) {
            let file = std::fs::OpenOptions::new()
                .append(true)
                .open(&self.targets[i])?;
            self.open = Some((index, file));
        }
        let (_, file) = self.open.as_mut().expect("just opened");
        file.write_all(bytes)?;
        self.written[i] += bytes.len() as u64;
        if let Some(progress) = &mut self.progress {
            progress.advance(bytes.len() as u64);
        }
        Ok(())
    }

    /// Moves the copied items into the destination; returns the names they got there.
    fn finish(mut self) -> Result<Vec<String>> {
        self.open = None;
        for (entry, written) in self.entries.iter().zip(&self.written) {
            if !entry.dir && *written != entry.size {
                bail!("{:?} incompleto", entry.path);
            }
        }
        let mut names = Vec::new();
        for (entry, target) in self.entries.iter().zip(&self.targets) {
            if entry.path.contains('/') {
                continue;
            }
            let name = target.file_name().context("no name")?;
            let place = if self.replace {
                let place = self.dest.join(name);
                replace_with(target, &place)
                    .with_context(|| format!("impossibile sovrascrivere {}", place.display()))?;
                place
            } else {
                let place = free_name(&self.dest, name);
                std::fs::rename(target, &place)
                    .with_context(|| format!("impossibile salvare {}", place.display()))?;
                place
            };
            names.push(
                place
                    .file_name()
                    .unwrap_or(name)
                    .to_string_lossy()
                    .into_owned(),
            );
        }
        Ok(names)
    }
}

impl Drop for Incoming {
    fn drop(&mut self) {
        self.open = None;
        let _ = std::fs::remove_dir_all(&self.staging);
    }
}

fn total_bytes(entries: &[Entry]) -> u64 {
    entries
        .iter()
        .fold(0u64, |total, e| total.saturating_add(e.size))
}

/// `dest/name`, or `dest/name (2).ext` and so on if taken: nothing is replaced.
fn free_name(dest: &Path, name: &OsStr) -> PathBuf {
    let taken = |path: &Path| std::fs::symlink_metadata(path).is_ok();
    let first = dest.join(name);
    if !taken(&first) {
        return first;
    }
    let path = Path::new(name);
    let stem = path.file_stem().unwrap_or(name).to_string_lossy();
    let extension = path.extension().map(|e| e.to_string_lossy());
    for n in 2..10_000 {
        let candidate = match &extension {
            Some(ext) => format!("{stem} ({n}).{ext}"),
            None => format!("{stem} ({n})"),
        };
        let candidate = dest.join(candidate);
        if !taken(&candidate) {
            return candidate;
        }
    }
    dest.join(format!(
        "{} ({:08x})",
        name.to_string_lossy(),
        rand_suffix()
    ))
}

/// Puts `new` at `place`, replacing what is there (file or folder). The old item is first
/// moved aside, so a failure leaves it where it was.
fn replace_with(new: &Path, place: &Path) -> std::io::Result<()> {
    let Ok(old_meta) = std::fs::symlink_metadata(place) else {
        return std::fs::rename(new, place);
    };
    let dir = place.parent().unwrap_or(Path::new("."));
    let aside = dir.join(format!(".kronos-old-{:08x}", rand_suffix()));
    std::fs::rename(place, &aside)?;
    if let Err(e) = std::fs::rename(new, place) {
        let _ = std::fs::rename(&aside, place);
        return Err(e);
    }
    let removed = if old_meta.is_dir() {
        std::fs::remove_dir_all(&aside)
    } else {
        std::fs::remove_file(&aside)
    };
    if let Err(e) = removed {
        tracing::warn!("cannot delete the replaced {}: {e}", aside.display());
    }
    Ok(())
}

fn rand_suffix() -> u32 {
    use std::hash::{BuildHasher, RandomState};
    RandomState::new().hash_one(std::time::SystemTime::now()) as u32
}

/// Host side: answers the viewer's requests until the channel closes. `on_log` gets short
/// messages for the activity list, phrased to follow the partner's name ("ha copiato…").
pub async fn serve(channel: Arc<dyn DataChannel>, mut on_log: impl FnMut(String)) {
    let lock = Arc::new(tokio::sync::Mutex::new(()));
    let mut decoder = Decoder::default();
    let mut upload: Option<Incoming> = None;
    let mut sending: Option<(u32, JoinHandle<()>)> = None;
    let (log_tx, mut logs) = mpsc::unbounded_channel::<String>();
    let mut browsing = false;
    loop {
        let event = tokio::select! {
            event = channel.poll() => event,
            Some(message) = logs.recv() => {
                on_log(message);
                continue;
            }
        };
        let data = match event {
            Some(DataChannelEvent::OnMessage(message)) => message.data,
            Some(DataChannelEvent::OnClose) | None => break,
            Some(_) => continue,
        };
        let frame = match decoder.feed(&data) {
            Ok(Some(frame)) => frame,
            Ok(None) => continue,
            Err(e) => {
                tracing::warn!("file manager: {e:#}");
                continue;
            }
        };
        let reply = match frame {
            Frame::Msg(FsMsg::List { req, path }) => {
                if !browsing {
                    browsing = true;
                    on_log("sta sfogliando i file di questo computer".into());
                }
                match tokio::task::spawn_blocking(move || list_dir(path.as_deref())).await {
                    Ok(Ok(listing)) => Some(FsMsg::Listing { req, listing }),
                    Ok(Err(message)) => Some(FsMsg::Error { req, message }),
                    Err(_) => None,
                }
            }
            Frame::Msg(FsMsg::Mkdir { req, path }) => Some(match make_dir(&path) {
                Ok(()) => {
                    on_log(format!("ha creato la cartella {path}"));
                    FsMsg::Done { req, names: vec![] }
                }
                Err(message) => FsMsg::Error { req, message },
            }),
            Frame::Msg(FsMsg::Download { req, paths }) => {
                if sending
                    .as_ref()
                    .is_some_and(|(_, task)| !task.is_finished())
                {
                    Some(FsMsg::Error {
                        req,
                        message: "un'altra copia è in corso".into(),
                    })
                } else {
                    let task = tokio::spawn(send_download(
                        channel.clone(),
                        lock.clone(),
                        req,
                        paths,
                        log_tx.clone(),
                    ));
                    sending = Some((req, task));
                    None
                }
            }
            Frame::Msg(FsMsg::Upload {
                req,
                dest,
                entries,
                replace,
            }) => {
                upload = None;
                match Incoming::start(req, Path::new(&dest), entries, None, replace) {
                    Ok(incoming) => {
                        upload = Some(incoming);
                        None
                    }
                    Err(e) => Some(FsMsg::Error {
                        req,
                        message: format!("{e:#}"),
                    }),
                }
            }
            Frame::Chunk { req, index, bytes } => {
                let failed = match upload.as_mut().filter(|u| u.req == req) {
                    Some(incoming) => incoming.chunk(index, bytes).err(),
                    None => None,
                };
                failed.map(|e| {
                    upload = None;
                    FsMsg::Error {
                        req,
                        message: format!("{e:#}"),
                    }
                })
            }
            Frame::End(req) => match upload.take_if(|u| u.req == req) {
                Some(incoming) => {
                    let total = total_bytes(&incoming.entries);
                    let dest = incoming.dest.display().to_string();
                    Some(match incoming.finish() {
                        Ok(names) => {
                            on_log(format!(
                                "ha copiato {} ({}) in {dest}",
                                items(names.len()),
                                megabytes(total)
                            ));
                            FsMsg::Done { req, names }
                        }
                        Err(e) => FsMsg::Error {
                            req,
                            message: format!("{e:#}"),
                        },
                    })
                }
                None => None,
            },
            Frame::Msg(FsMsg::Cancel { req }) => {
                if let Some((sent, task)) = &sending
                    && *sent == req
                {
                    task.abort();
                }
                if upload.as_ref().is_some_and(|u| u.req == req) {
                    upload = None;
                }
                None
            }
            // Answers: only the viewer receives them.
            Frame::Msg(_) => None,
        };
        if let Some(reply) = reply
            && let Err(e) = send_msg(&channel, &lock, &reply).await
        {
            tracing::warn!("file manager reply not sent: {e:#}");
        }
    }
    if let Some((_, task)) = sending {
        task.abort();
    }
}

/// Host: sends the files of a download (`Sending`, then their contents), or `Error`.
async fn send_download(
    channel: Arc<dyn DataChannel>,
    lock: Arc<tokio::sync::Mutex<()>>,
    req: u32,
    paths: Vec<String>,
    log: mpsc::UnboundedSender<String>,
) {
    let sources: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
    let outcome = async {
        let outgoing = tokio::task::spawn_blocking(move || {
            files::collect_limited(req, &sources, MAX_ENTRIES, u64::MAX)
        })
        .await??;
        let entries = outgoing.manifest.entries.clone();
        send_msg(&channel, &lock, &FsMsg::Sending { req, entries }).await?;
        files::send_chunks(&channel, &outgoing, None).await?;
        anyhow::Ok(outgoing.manifest)
    }
    .await;
    match outcome {
        Ok(manifest) => {
            let _ = log.send(format!(
                "ha copiato {} ({}) da questo computer",
                items(manifest.items()),
                megabytes(manifest.total_bytes())
            ));
        }
        Err(e) => {
            let message = format!("{e:#}");
            let _ = send_msg(&channel, &lock, &FsMsg::Error { req, message }).await;
        }
    }
}

enum Answer {
    Listing(Listing),
    Done(Vec<String>),
}

type Waiter = oneshot::Sender<Result<Answer, String>>;

#[derive(Default)]
struct ClientState {
    /// Answers awaited, by request.
    waiting: HashMap<u32, Waiter>,
    /// Downloads under way: their local folder and where their progress goes.
    /// Destination, progress and "overwrite" of each download under way.
    downloads: HashMap<
        u32,
        (
            PathBuf,
            Option<mpsc::UnboundedSender<TransferProgress>>,
            bool,
        ),
    >,
    /// Uploads being sent, to stop on cancel.
    uploads: HashMap<u32, JoinHandle<()>>,
}

fn answer(state: &Mutex<ClientState>, req: u32, result: Result<Answer, String>) {
    if let Some(waiter) = state.lock().unwrap().waiting.remove(&req) {
        let _ = waiter.send(result);
    }
}

/// Viewer side: browses the host's folders and copies files to and from it.
pub struct FsClient {
    channel: Arc<dyn DataChannel>,
    lock: Arc<tokio::sync::Mutex<()>>,
    next: AtomicU32,
    state: Arc<Mutex<ClientState>>,
    /// Tells the reader to drop what it received of a cancelled download.
    cancelled: mpsc::UnboundedSender<u32>,
    reader: JoinHandle<()>,
}

impl FsClient {
    pub fn new(channel: Arc<dyn DataChannel>) -> Self {
        let lock = Arc::new(tokio::sync::Mutex::new(()));
        let state = Arc::new(Mutex::new(ClientState::default()));
        let (cancelled, cancels) = mpsc::unbounded_channel();
        let reader = tokio::spawn(read_answers(
            channel.clone(),
            lock.clone(),
            state.clone(),
            cancels,
        ));
        Self {
            channel,
            lock,
            next: AtomicU32::new(1),
            state,
            cancelled,
            reader,
        }
    }

    /// A new request number, to pass to [`Self::download`] or [`Self::upload`] (and to
    /// [`Self::cancel`] while it runs).
    pub fn new_request(&self) -> u32 {
        self.next.fetch_add(1, Ordering::Relaxed)
    }

    async fn ask(&self, req: u32, msg: FsMsg, timeout: Option<Duration>) -> Result<Answer, String> {
        let (tx, rx) = oneshot::channel();
        self.state.lock().unwrap().waiting.insert(req, tx);
        if let Err(e) = send_msg(&self.channel, &self.lock, &msg).await {
            self.state.lock().unwrap().waiting.remove(&req);
            return Err(format!("richiesta non inviata: {e:#}"));
        }
        let result = match timeout {
            Some(timeout) => tokio::time::timeout(timeout, rx).await.ok(),
            None => Some(rx.await),
        };
        self.state.lock().unwrap().waiting.remove(&req);
        match result {
            None => Err(NO_ANSWER.into()),
            Some(Ok(answer)) => answer,
            Some(Err(_)) => Err("connessione chiusa".into()),
        }
    }

    /// Lists a folder of the host (`None`: its user's home folder).
    pub async fn list(&self, path: Option<String>) -> Result<Listing, String> {
        let req = self.new_request();
        match self
            .ask(req, FsMsg::List { req, path }, Some(ANSWER_TIMEOUT))
            .await?
        {
            Answer::Listing(listing) => Ok(listing),
            Answer::Done(_) => Err("risposta inattesa".into()),
        }
    }

    /// Creates a folder on the host.
    pub async fn make_dir(&self, path: String) -> Result<(), String> {
        let req = self.new_request();
        self.ask(req, FsMsg::Mkdir { req, path }, Some(ANSWER_TIMEOUT))
            .await
            .map(|_| ())
    }

    /// Copies files and folders of the host into the local folder `dest`, overwriting items
    /// with the same name if `replace`; returns the names they got there.
    pub async fn download(
        &self,
        req: u32,
        paths: Vec<String>,
        dest: PathBuf,
        replace: bool,
        progress: Option<mpsc::UnboundedSender<TransferProgress>>,
    ) -> Result<Vec<String>, String> {
        self.state
            .lock()
            .unwrap()
            .downloads
            .insert(req, (dest, progress, replace));
        let result = self.ask(req, FsMsg::Download { req, paths }, None).await;
        self.state.lock().unwrap().downloads.remove(&req);
        match result? {
            Answer::Done(names) => Ok(names),
            Answer::Listing(_) => Err("risposta inattesa".into()),
        }
    }

    /// Copies local files and folders into the host's folder `dest`, overwriting items with
    /// the same name if `replace` (hosts before 1.9.2 make copies); returns the names they
    /// got there.
    pub async fn upload(
        &self,
        req: u32,
        paths: Vec<PathBuf>,
        dest: String,
        replace: bool,
        progress: Option<mpsc::UnboundedSender<TransferProgress>>,
    ) -> Result<Vec<String>, String> {
        let outgoing = tokio::task::spawn_blocking(move || {
            files::collect_limited(req, &paths, MAX_ENTRIES, u64::MAX)
        })
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| format!("{e:#}"))?;
        let (tx, rx) = oneshot::channel();
        self.state.lock().unwrap().waiting.insert(req, tx);
        let (channel, lock, state) = (self.channel.clone(), self.lock.clone(), self.state.clone());
        let task = tokio::spawn(async move {
            let total = outgoing.manifest.total_bytes();
            let sent = async {
                let entries = outgoing.manifest.entries.clone();
                let upload = FsMsg::Upload {
                    req,
                    dest,
                    entries,
                    replace,
                };
                send_msg(&channel, &lock, &upload).await?;
                let mut progress = progress.map(|tx| ProgressReporter::new(tx, true, total));
                files::send_chunks(&channel, &outgoing, progress.as_mut()).await
            }
            .await;
            if let Err(e) = sent {
                answer(&state, req, Err(format!("{e:#}")));
            }
        });
        self.state.lock().unwrap().uploads.insert(req, task);
        let result = rx.await;
        {
            let mut state = self.state.lock().unwrap();
            state.uploads.remove(&req);
            state.waiting.remove(&req);
        }
        match result {
            Ok(Ok(Answer::Done(names))) => Ok(names),
            Ok(Ok(Answer::Listing(_))) => Err("risposta inattesa".into()),
            Ok(Err(message)) => Err(message),
            Err(_) => Err("connessione chiusa".into()),
        }
    }

    /// Stops a copy in either direction; what arrived of it is deleted.
    pub async fn cancel(&self, req: u32) {
        let (waiter, upload) = {
            let mut state = self.state.lock().unwrap();
            state.downloads.remove(&req);
            (state.waiting.remove(&req), state.uploads.remove(&req))
        };
        if let Some(task) = upload {
            task.abort();
        }
        let _ = self.cancelled.send(req);
        if let Some(waiter) = waiter {
            let _ = waiter.send(Err("copia annullata".into()));
        }
        let _ = send_msg(&self.channel, &self.lock, &FsMsg::Cancel { req }).await;
    }
}

impl Drop for FsClient {
    fn drop(&mut self) {
        self.reader.abort();
        for (_, task) in self.state.lock().unwrap().uploads.drain() {
            task.abort();
        }
    }
}

/// Viewer: handles the host's answers and the files of downloads.
async fn read_answers(
    channel: Arc<dyn DataChannel>,
    lock: Arc<tokio::sync::Mutex<()>>,
    state: Arc<Mutex<ClientState>>,
    mut cancels: mpsc::UnboundedReceiver<u32>,
) {
    let mut decoder = Decoder::default();
    let mut incoming: Option<Incoming> = None;
    loop {
        let event = tokio::select! {
            event = channel.poll() => event,
            Some(req) = cancels.recv() => {
                // Deletes what arrived; later chunks of it find nothing to write into.
                if incoming.as_ref().is_some_and(|i| i.req == req) {
                    incoming = None;
                }
                continue;
            }
        };
        let data = match event {
            Some(DataChannelEvent::OnMessage(message)) => message.data,
            Some(DataChannelEvent::OnClose) | None => break,
            Some(_) => continue,
        };
        let frame = match decoder.feed(&data) {
            Ok(Some(frame)) => frame,
            Ok(None) => continue,
            Err(e) => {
                tracing::warn!("file manager: {e:#}");
                continue;
            }
        };
        // A download that fails here: the host is told to stop sending.
        let mut stop = None;
        match frame {
            Frame::Msg(FsMsg::Listing { req, listing }) => {
                answer(&state, req, Ok(Answer::Listing(listing)))
            }
            Frame::Msg(FsMsg::Done { req, names }) => answer(&state, req, Ok(Answer::Done(names))),
            Frame::Msg(FsMsg::Error { req, message }) => {
                if incoming.as_ref().is_some_and(|i| i.req == req) {
                    incoming = None;
                }
                answer(&state, req, Err(message));
            }
            Frame::Msg(FsMsg::Sending { req, entries }) => {
                incoming = None;
                let target = state.lock().unwrap().downloads.get(&req).cloned();
                if let Some((dest, progress, replace)) = target {
                    match Incoming::start(req, &dest, entries, progress, replace) {
                        Ok(started) => incoming = Some(started),
                        Err(e) => {
                            answer(&state, req, Err(format!("{e:#}")));
                            stop = Some(req);
                        }
                    }
                }
            }
            Frame::Chunk { req, index, bytes } => {
                if let Some(current) = incoming.as_mut().filter(|i| i.req == req) {
                    if !state.lock().unwrap().downloads.contains_key(&req) {
                        // Cancelled meanwhile.
                        incoming = None;
                    } else if let Err(e) = current.chunk(index, bytes) {
                        incoming = None;
                        answer(&state, req, Err(format!("{e:#}")));
                        stop = Some(req);
                    }
                }
            }
            Frame::End(req) => {
                let wanted = state.lock().unwrap().downloads.contains_key(&req);
                if let Some(done) = incoming.take_if(|i| i.req == req)
                    && wanted
                {
                    let result = done
                        .finish()
                        .map(Answer::Done)
                        .map_err(|e| format!("{e:#}"));
                    answer(&state, req, result);
                }
            }
            Frame::Msg(_) => {}
        }
        if let Some(req) = stop {
            let _ = send_msg(&channel, &lock, &FsMsg::Cancel { req }).await;
        }
    }
    // The channel closed: nobody waits for ever.
    let waiting: Vec<Waiter> = state
        .lock()
        .unwrap()
        .waiting
        .drain()
        .map(|(_, w)| w)
        .collect();
    for waiter in waiting {
        let _ = waiter.send(Err("connessione chiusa".into()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "kronos-fs-{name}-{}-{:08x}",
            std::process::id(),
            rand_suffix()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn folders_come_first_and_hidden_files_are_marked() {
        let dir = temp_dir("list");
        std::fs::write(dir.join("b.txt"), "ciao").unwrap();
        std::fs::write(dir.join(".nascosto"), "").unwrap();
        std::fs::create_dir(dir.join("Zeta")).unwrap();
        std::fs::create_dir(dir.join("alfa")).unwrap();
        let listing = list_dir(Some(dir.to_str().unwrap())).unwrap();
        let names: Vec<&str> = listing.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["alfa", "Zeta", ".nascosto", "b.txt"]);
        assert!(listing.entries[2].hidden);
        assert_eq!(listing.entries[3].size, 4);
        assert_eq!(
            listing.parent.as_deref(),
            dir.parent().and_then(|p| p.to_str())
        );
        assert!(!listing.roots.is_empty());
        assert!(list_dir(Some(dir.join("manca").to_str().unwrap())).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn no_path_lists_the_home_folder() {
        let listing = list_dir(None).unwrap();
        assert_eq!(Some(listing.path), listing.home);
    }

    #[test]
    fn json_messages_survive_chunking() {
        let entries: Vec<Entry> = (0..2_000)
            .map(|i| Entry {
                path: format!("cartella/file-{i}.txt"),
                size: i,
                dir: false,
            })
            .collect();
        let msg = FsMsg::Sending { req: 7, entries };
        let messages = json_messages(&msg).unwrap();
        assert!(messages.len() > 1);
        let mut decoder = Decoder::default();
        let mut decoded = None;
        for m in &messages {
            if let Some(Frame::Msg(msg)) = decoder.feed(m).unwrap() {
                decoded = Some(msg);
            }
        }
        assert_eq!(decoded, Some(msg));
    }

    #[test]
    fn a_copy_lands_under_free_names_and_leaves_no_staging_folder() {
        let dest = temp_dir("copy");
        std::fs::write(dest.join("nota.txt"), "vecchia").unwrap();
        let entries = vec![
            Entry {
                path: "nota.txt".into(),
                size: 5,
                dir: false,
            },
            Entry {
                path: "foto".into(),
                size: 0,
                dir: true,
            },
            Entry {
                path: "foto/a.jpg".into(),
                size: 3,
                dir: false,
            },
        ];
        let mut incoming = Incoming::start(1, &dest, entries, None, false).unwrap();
        incoming.chunk(0, b"nuova").unwrap();
        incoming.chunk(2, b"jpg").unwrap();
        assert!(incoming.chunk(2, b"x").is_err(), "more than announced");
        let names = incoming.finish().unwrap();
        assert_eq!(names, ["nota (2).txt", "foto"]);
        assert_eq!(std::fs::read(dest.join("nota.txt")).unwrap(), b"vecchia");
        assert_eq!(std::fs::read(dest.join("nota (2).txt")).unwrap(), b"nuova");
        assert_eq!(std::fs::read(dest.join("foto/a.jpg")).unwrap(), b"jpg");
        let left: Vec<_> = std::fs::read_dir(&dest).unwrap().flatten().collect();
        assert_eq!(left.len(), 3, "staging folder removed");
        std::fs::remove_dir_all(dest).unwrap();
    }

    #[test]
    fn overwriting_replaces_files_and_folders() {
        let dest = temp_dir("replace");
        std::fs::write(dest.join("nota.txt"), "vecchia").unwrap();
        std::fs::create_dir(dest.join("foto")).unwrap();
        std::fs::write(dest.join("foto/vecchia.jpg"), "x").unwrap();
        let entries = vec![
            Entry {
                path: "nota.txt".into(),
                size: 5,
                dir: false,
            },
            Entry {
                path: "foto".into(),
                size: 0,
                dir: true,
            },
            Entry {
                path: "foto/a.jpg".into(),
                size: 3,
                dir: false,
            },
        ];
        let mut incoming = Incoming::start(1, &dest, entries, None, true).unwrap();
        incoming.chunk(0, b"nuova").unwrap();
        incoming.chunk(2, b"jpg").unwrap();
        assert_eq!(incoming.finish().unwrap(), ["nota.txt", "foto"]);
        assert_eq!(std::fs::read(dest.join("nota.txt")).unwrap(), b"nuova");
        assert_eq!(std::fs::read(dest.join("foto/a.jpg")).unwrap(), b"jpg");
        assert!(
            !dest.join("foto/vecchia.jpg").exists(),
            "folder replaced, not merged"
        );
        let left: Vec<_> = std::fs::read_dir(&dest).unwrap().flatten().collect();
        assert_eq!(left.len(), 2, "nothing set aside or staged is left");
        std::fs::remove_dir_all(dest).unwrap();
    }

    #[test]
    fn unsafe_paths_and_unfinished_copies_leave_nothing() {
        let dest = temp_dir("unsafe");
        let entry = |path: &str| Entry {
            path: path.into(),
            size: 1,
            dir: false,
        };
        assert!(Incoming::start(1, &dest, vec![entry("../fuori")], None, false).is_err());
        assert!(Incoming::start(1, &dest, vec![entry("/etc/passwd")], None, false).is_err());
        let incoming = Incoming::start(1, &dest, vec![entry("a")], None, false).unwrap();
        // Incomplete: refused, and the partial file goes with the staging folder.
        assert!(incoming.finish().is_err());
        let partial = Incoming::start(2, &dest, vec![entry("b")], None, false).unwrap();
        drop(partial);
        assert_eq!(std::fs::read_dir(&dest).unwrap().count(), 0);
        assert!(Incoming::start(1, &dest.join("manca"), vec![entry("a")], None, false).is_err());
        std::fs::remove_dir_all(dest).unwrap();
    }
}
