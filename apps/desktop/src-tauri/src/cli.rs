//! Text mode (ADR 0078): `kronos-remote` in a terminal shows this device's id and password
//! and asks there whether to accept incoming connections.
//!
//! The app (window open or hidden in the tray) listens on a local TCP port; the port and a
//! random token are in `cli.json` in its data folder, readable by the user only. The command
//! is the app's own executable started with `--cli`: it reads that file, connects and talks
//! JSON lines. If the app is not running, the command starts it hidden with `--cli-owned`
//! and the app quits when the command does (unless its window was opened meanwhile).
//!
//! The command is installed from Preferences: a small script that runs the executable with
//! `--cli` (`/usr/local/bin` on macOS, `~/.local/bin` on Linux, a folder added to the user's
//! PATH on Windows).

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use signaling_proto::SessionId;
use tauri::{AppHandle, Manager};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::sync::{broadcast, mpsc};

use crate::service::{Service, UiEvent};

/// First argument of the executable for text mode.
pub(crate) const CLI_ARG: &str = "--cli";
/// The app was started by the command: it quits when the command does.
pub(crate) const OWNED_ARG: &str = "--cli-owned";
/// Name of the installed command.
pub(crate) const COMMAND_NAME: &str = "kronos-remote";
/// In the app's data folder: where the command finds the app.
const CONTROL_FILE: &str = "cli.json";
/// How long the command waits for an app it started.
const START_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Serialize, Deserialize)]
struct Control {
    port: u16,
    token: String,
}

/// Commands from the terminal.
#[derive(Deserialize, Serialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
enum Request {
    Approve {
        session: SessionId,
        accept: bool,
    },
    NewPassword,
    StopHosting {
        session: SessionId,
    },
    VirtualMonitor {
        install: bool,
    },
    /// macOS: "Computer usato solo da remoto", the virtual screen as the main one (ADR 0064).
    RemoteOnly {
        enabled: bool,
    },
}

static EVENTS: OnceLock<broadcast::Sender<UiEvent>> = OnceLock::new();
/// Terminals connected right now.
static CLIENTS: AtomicUsize = AtomicUsize::new(0);
/// Connections waiting for approval (session, partner, deadline), for a terminal that
/// connects while one is pending.
static PENDING: Mutex<Vec<(SessionId, String, Instant)>> = Mutex::new(Vec::new());
/// Partners watching this screen (session, partner).
static HOSTING: Mutex<Vec<(SessionId, String)>> = Mutex::new(Vec::new());

/// A terminal is connected: approvals are answered there and the window stays hidden.
pub(crate) fn attached() -> bool {
    CLIENTS.load(Ordering::Relaxed) > 0
}

/// Called for every UI event: keeps the pending approvals and forwards the few events the
/// terminal shows.
pub(crate) fn publish(event: &UiEvent) {
    match event {
        UiEvent::ApprovalRequest {
            session,
            peer,
            timeout_secs,
        } => PENDING.lock().unwrap().push((
            *session,
            peer.clone(),
            Instant::now() + Duration::from_secs(*timeout_secs),
        )),
        UiEvent::ApprovalClosed { session } => PENDING.lock().unwrap().retain(|p| p.0 != *session),
        UiEvent::HostingStarted { session, peer } => {
            HOSTING.lock().unwrap().push((*session, peer.clone()))
        }
        UiEvent::HostingEnded { session, .. } => {
            HOSTING.lock().unwrap().retain(|h| h.0 != *session)
        }
        UiEvent::Status(_) | UiEvent::Log { .. } => {}
        _ => return,
    }
    if attached()
        && let Some(events) = EVENTS.get()
    {
        let _ = events.send(event.clone());
    }
}

/// Starts listening for the command (full app and Quick Support, not the host before login).
pub(crate) fn start_server(app: &AppHandle, data_dir: PathBuf, owned: bool) {
    let (events, _) = broadcast::channel(256);
    let _ = EVENTS.set(events);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = serve(app, data_dir.join(CONTROL_FILE), owned).await {
            tracing::warn!("text mode: {e:#}");
        }
    });
}

async fn serve(app: AppHandle, control: PathBuf, owned: bool) -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await?;
    let token: String = rand::random::<[u8; 16]>()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    let control_json = serde_json::to_vec(&Control {
        port: listener.local_addr()?.port(),
        token: token.clone(),
    })?;
    crate::store::private_file(&control)?.write_all(&control_json)?;
    loop {
        let (stream, _) = listener.accept().await?;
        let app = app.clone();
        let token = token.clone();
        tauri::async_runtime::spawn(async move {
            client(&app, stream, &token).await;
            let window_open = app
                .get_webview_window("main")
                .is_some_and(|w| w.is_visible().unwrap_or(false));
            tracing::info!(owned, window_open, "text mode: terminal gone");
            if owned && !attached() && !window_open {
                tracing::info!("text mode: the command that started the app ended");
                app.exit(0);
            }
        });
    }
}

async fn client(app: &AppHandle, stream: tokio::net::TcpStream, token: &str) {
    let (read, mut write) = stream.into_split();
    let mut lines = tokio::io::BufReader::new(read).lines();
    // Same user only: the token is in a file only this user can read.
    match tokio::time::timeout(Duration::from_secs(5), lines.next_line()).await {
        Ok(Ok(Some(line))) if line.trim() == token => {}
        _ => return,
    }
    let Some(events) = EVENTS.get() else {
        return;
    };
    // Subscribed before the greeting: nothing that happens meanwhile is lost.
    let mut events = events.subscribe();
    CLIENTS.fetch_add(1, Ordering::Relaxed);
    tracing::info!("text mode: terminal connected");
    let (replies, mut reply_rx) = mpsc::unbounded_channel::<Value>();
    let mut out = vec![hello(app).await];
    'session: loop {
        for message in out.drain(..) {
            let mut line = message.to_string();
            line.push('\n');
            if write.write_all(line.as_bytes()).await.is_err() {
                break 'session;
            }
        }
        tokio::select! {
            event = events.recv() => match event {
                Ok(event) => out.push(serde_json::to_value(&event).unwrap_or(Value::Null)),
                Err(broadcast::error::RecvError::Lagged(_)) => {}
                Err(broadcast::error::RecvError::Closed) => break,
            },
            reply = reply_rx.recv() => out.extend(reply),
            line = lines.next_line() => match line {
                Ok(Some(line)) => match serde_json::from_str::<Request>(&line) {
                    Ok(request) => handle(app, request, &replies),
                    Err(e) => tracing::warn!("text mode: bad request: {e}"),
                },
                _ => break,
            },
        }
    }
    CLIENTS.fetch_sub(1, Ordering::Relaxed);
    tracing::info!("text mode: terminal disconnected");
}

async fn hello(app: &AppHandle) -> Value {
    let status = app.state::<Service>().status();
    let now = Instant::now();
    let pending: Vec<Value> = PENDING
        .lock()
        .unwrap()
        .iter()
        .map(|(session, peer, deadline)| {
            json!({
                "session": session,
                "peer": peer,
                "timeout_secs": deadline.saturating_duration_since(now).as_secs(),
            })
        })
        .collect();
    let hosting: Vec<Value> = HOSTING
        .lock()
        .unwrap()
        .iter()
        .map(|(session, peer)| json!({ "session": session, "peer": peer }))
        .collect();
    let monitor = crate::virtual_monitor::virtual_monitor_status(app.clone()).await;
    json!({
        "kind": "hello",
        "version": app.package_info().version.to_string(),
        "os": std::env::consts::OS,
        "remote_only": app.state::<Service>().remote_only(),
        "status": status,
        "pending": pending,
        "hosting": hosting,
        "screens": screen_count().await,
        "virtual_monitor": monitor,
    })
}

/// How many screens this computer can share (0: nobody would see anything without the
/// virtual monitor on Windows, or the virtual screen on macOS). Linux: unknown.
async fn screen_count() -> Option<usize> {
    #[cfg(any(windows, target_os = "macos"))]
    {
        tauri::async_runtime::spawn_blocking(|| capture::displays().ok().map(|d| d.len()))
            .await
            .ok()
            .flatten()
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    None
}

fn handle(app: &AppHandle, request: Request, replies: &mpsc::UnboundedSender<Value>) {
    let service = app.state::<Service>();
    match request {
        Request::Approve { session, accept } => service.respond(session, accept),
        Request::NewPassword => {
            service.new_password();
        }
        Request::StopHosting { session } => service.stop_hosting(session),
        Request::VirtualMonitor { install } => {
            let app = app.clone();
            let replies = replies.clone();
            tauri::async_runtime::spawn(async move {
                let result = crate::virtual_monitor::set_virtual_monitor(app, install).await;
                let _ = replies.send(match result {
                    Ok(monitor) => json!({ "kind": "virtual_monitor", "monitor": monitor }),
                    Err(error) => json!({ "kind": "virtual_monitor", "error": error }),
                });
            });
        }
        Request::RemoteOnly { enabled } => {
            let app = app.clone();
            let replies = replies.clone();
            tauri::async_runtime::spawn(async move {
                let result = app.state::<Service>().set_remote_only(enabled).await;
                let _ = replies.send(match result {
                    Ok(enabled) => json!({ "kind": "remote_only", "enabled": enabled }),
                    Err(error) => json!({ "kind": "remote_only", "error": error }),
                });
            });
        }
    }
}

// ---------------------------------------------------------------------------------------
// The command in the terminal.

/// Runs the text mode; returns the exit code.
pub(crate) fn run(args: &[String]) -> i32 {
    #[cfg(windows)]
    console::attach();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        println!("{}", usage());
        return 0;
    }
    if let Some(other) = args.iter().find(|a| !a.is_empty()) {
        eprintln!("Opzione sconosciuta: {other}\n\n{}", usage());
        return 2;
    }
    let Some(control) = control_path() else {
        eprintln!("Cartella dei dati dell'app non trovata.");
        return 1;
    };
    let connection = match connect(&control) {
        Some(connection) => connection,
        None => match start_app(&control) {
            Ok(connection) => connection,
            Err(message) => {
                eprintln!("{message}");
                return 1;
            }
        },
    };
    Terminal::new(connection).run()
}

fn usage() -> String {
    format!(
        "Kronos Remote Support in modalità testo.\n\n\
         Uso: {COMMAND_NAME}\n\n\
         Mostra l'ID e la password di questo computer e chiede qui se accettare chi si collega.\n\
         Se l'app è aperta si aggancia a lei; altrimenti la avvia nascosta e la chiude all'uscita.\n\n\
         Comandi mentre gira:\n  \
         s / n   accetta o rifiuta la richiesta in attesa\n  \
         p       nuova password\n  \
         i       mostra di nuovo ID e password\n  \
         d       disconnetti chi è collegato\n  \
         m       monitor virtuale: su Windows lo installa o lo rimuove, sul Mac usa lo schermo\n          \
         virtuale come principale (\"Computer usato solo da remoto\")\n  \
         q       esci"
    )
}

fn control_path() -> Option<PathBuf> {
    let identifier = crate::context().config().identifier.clone();
    Some(dirs::data_dir()?.join(identifier).join(CONTROL_FILE))
}

struct Connection {
    stream: TcpStream,
    reader: BufReader<TcpStream>,
    hello: Value,
}

/// Connects to the running app, `None` if there is none.
fn connect(control: &Path) -> Option<Connection> {
    let control: Control = serde_json::from_slice(&std::fs::read(control).ok()?).ok()?;
    let address = SocketAddr::from((Ipv4Addr::LOCALHOST, control.port));
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(1)).ok()?;
    stream
        .write_all(format!("{}\n", control.token).as_bytes())
        .ok()?;
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .ok()?;
    let mut reader = BufReader::new(stream.try_clone().ok()?);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    let hello: Value = serde_json::from_str(&line).ok()?;
    (hello["kind"] == "hello").then_some(())?;
    stream.set_read_timeout(None).ok()?;
    Some(Connection {
        stream,
        reader,
        hello,
    })
}

/// Starts the app hidden, owned by this command, and connects to it.
fn start_app(control: &Path) -> Result<Connection, String> {
    println!("Avvio di Kronos Remote Support...");
    let exe = std::env::current_exe().map_err(|e| format!("Percorso dell'app: {e}"))?;
    let mut command = std::process::Command::new(exe);
    command
        .args([crate::HIDDEN_ARG, OWNED_ARG])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    // Its own process group: Ctrl+C here ends the command, and the app then quits by itself
    // (restoring what it changed) instead of being killed with it.
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    command
        .spawn()
        .map_err(|e| format!("Avvio dell'app non riuscito: {e}"))?;
    let deadline = Instant::now() + START_TIMEOUT;
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(300));
        if let Some(connection) = connect(control) {
            return Ok(connection);
        }
    }
    Err(
        "L'app non risponde. Se è aperta una versione precedente alla 2.4.0, chiudila e riprova."
            .into(),
    )
}

/// A question waiting for "s" or "n".
enum Question {
    Approval { session: SessionId, peer: String },
    VirtualMonitor { install: bool },
    RemoteOnly { enable: bool },
}

#[derive(Default)]
struct State {
    questions: VecDeque<Question>,
    hosting: Vec<(SessionId, String)>,
    /// Last id/password/link shown, to print only changes.
    shown: Option<(Value, Value, Value)>,
    monitor_supported: bool,
    monitor_installed: bool,
    /// macOS: the virtual screen needs no install, "m" switches it to the main screen.
    mac: bool,
    remote_only: bool,
}

struct Terminal {
    stream: TcpStream,
    reader: Option<BufReader<TcpStream>>,
    hello: Value,
    state: Arc<Mutex<State>>,
}

impl Terminal {
    fn new(connection: Connection) -> Self {
        Self {
            stream: connection.stream,
            reader: Some(connection.reader),
            hello: connection.hello,
            state: Arc::default(),
        }
    }

    fn run(mut self) -> i32 {
        self.greet();
        let reader = self.reader.take().expect("reader");
        let state = self.state.clone();
        std::thread::spawn(move || {
            for line in reader.lines() {
                let Ok(line) = line else { break };
                if let Ok(message) = serde_json::from_str::<Value>(&line) {
                    on_message(&state, &message);
                }
            }
            println!("\nKronos Remote Support si è chiuso.");
            std::process::exit(0);
        });
        let stdin = std::io::stdin();
        for line in stdin.lock().lines() {
            let Ok(line) = line else { break };
            if !self.on_input(line.trim()) {
                return 0;
            }
        }
        // No keyboard (e.g. input from a file): keep showing events until the app quits.
        loop {
            std::thread::park();
        }
    }

    fn greet(&self) {
        let hello = &self.hello;
        let version = hello["version"].as_str().unwrap_or("?");
        println!("Kronos Remote Support {version} — modalità testo");
        let mut state = self.state.lock().unwrap();
        state.monitor_supported = hello["virtual_monitor"]["supported"] == true;
        state.monitor_installed = hello["virtual_monitor"]["installed"] == true;
        state.mac = hello["os"] == "macos";
        state.remote_only = hello["remote_only"] == true;
        show_status(&mut state, &hello["status"]);
        for h in hello["hosting"].as_array().into_iter().flatten() {
            if let Some(session) = h["session"].as_u64() {
                let peer = h["peer"].as_str().unwrap_or("?").to_owned();
                println!("● {peer} è collegato a questo computer.");
                state.hosting.push((session, peer));
            }
        }
        println!("{}", commands_hint(&state));
        let no_screens = hello["screens"].as_u64() == Some(0);
        if state.mac && no_screens && !state.remote_only {
            println!(
                "\n? Nessuno schermo collegato: a ogni collegamento verrà creato uno schermo virtuale.\n  \
                 Usarlo come schermo principale (\"Computer usato solo da remoto\")? [s/n]"
            );
            state
                .questions
                .push_back(Question::RemoteOnly { enable: true });
        }
        if state.monitor_supported && !state.monitor_installed && no_screens {
            println!(
                "\n? Nessuno schermo collegato: chi si collega non vedrebbe nulla.\n  \
                 Installare il monitor virtuale? Windows chiederà la conferma di amministratore. [s/n]"
            );
            state
                .questions
                .push_back(Question::VirtualMonitor { install: true });
        }
        for p in hello["pending"].as_array().into_iter().flatten() {
            ask_approval(&mut state, p);
        }
    }

    /// Handles a line typed by the user; false to quit.
    fn on_input(&mut self, input: &str) -> bool {
        let input = input.to_lowercase();
        let answer = match input.as_str() {
            "s" | "si" | "sì" | "y" | "yes" => Some(true),
            "n" | "no" => Some(false),
            _ => None,
        };
        let mut state = self.state.lock().unwrap();
        if let Some(accept) = answer {
            match state.questions.pop_front() {
                Some(Question::Approval { session, peer }) => {
                    println!(
                        "{}",
                        if accept {
                            format!("Accettata la connessione da {peer}.")
                        } else {
                            format!("Rifiutata la connessione da {peer}.")
                        }
                    );
                    drop(state);
                    self.send(&Request::Approve { session, accept });
                }
                Some(Question::VirtualMonitor { install }) => {
                    drop(state);
                    if accept {
                        println!(
                            "{} del monitor virtuale... (conferma la richiesta di Windows)",
                            if install {
                                "Installazione"
                            } else {
                                "Rimozione"
                            }
                        );
                        self.send(&Request::VirtualMonitor { install });
                    } else {
                        println!("Monitor virtuale lasciato com'è.");
                    }
                }
                Some(Question::RemoteOnly { enable }) => {
                    drop(state);
                    if accept {
                        self.send(&Request::RemoteOnly { enabled: enable });
                    } else {
                        println!("Schermo virtuale lasciato com'è.");
                    }
                }
                None => println!("Nessuna richiesta in attesa. {}", commands_hint(&state)),
            }
            return true;
        }
        match input.as_str() {
            "" => {}
            "q" | "quit" | "esci" => return false,
            "p" => {
                drop(state);
                self.send(&Request::NewPassword);
            }
            "d" => {
                if state.hosting.is_empty() {
                    println!("Nessuno è collegato.");
                }
                let sessions: Vec<_> = state.hosting.iter().map(|h| h.0).collect();
                drop(state);
                for session in sessions {
                    self.send(&Request::StopHosting { session });
                }
            }
            "m" if state.monitor_supported => {
                let install = !state.monitor_installed;
                println!(
                    "? {} il monitor virtuale? [s/n]",
                    if install { "Installare" } else { "Rimuovere" }
                );
                state
                    .questions
                    .push_front(Question::VirtualMonitor { install });
            }
            "m" if state.mac => {
                let enable = !state.remote_only;
                if enable {
                    println!(
                        "Sul Mac lo schermo virtuale non si installa: l'app lo crea da sola quando manca un monitor.\n\
                         ? Usarlo sempre come schermo principale (\"Computer usato solo da remoto\")? [s/n]"
                    );
                } else {
                    println!(
                        "? Tornare agli schermi reali (spegnere \"Computer usato solo da remoto\")? [s/n]"
                    );
                }
                state.questions.push_front(Question::RemoteOnly { enable });
            }
            "m" => println!("Il monitor virtuale non è disponibile su questo computer."),
            "i" | "id" => {
                if let Some((id, password, _)) = &state.shown {
                    print_credentials(id, password);
                }
            }
            _ => println!("{}", commands_hint(&state)),
        }
        true
    }

    fn send(&mut self, request: &Request) {
        let mut line = serde_json::to_string(request).unwrap_or_default();
        line.push('\n');
        if self.stream.write_all(line.as_bytes()).is_err() {
            println!("Collegamento con l'app perso.");
            std::process::exit(1);
        }
    }
}

fn commands_hint(state: &State) -> String {
    let mut hint = String::from("Comandi: p nuova password · i mostra ID · d disconnetti");
    if state.monitor_supported || state.mac {
        hint.push_str(" · m monitor virtuale");
    }
    hint.push_str(" · q esci");
    hint
}

fn print_credentials(id: &Value, password: &Value) {
    println!(
        "  ID:        {}\n  Password:  {}",
        id.as_str().unwrap_or("(in registrazione...)"),
        password.as_str().unwrap_or("")
    );
}

fn show_status(state: &mut State, status: &Value) {
    let shown = (
        status["id"].clone(),
        status["password"].clone(),
        status["link"].clone(),
    );
    if state.shown.as_ref() == Some(&shown) {
        return;
    }
    let previous = state.shown.replace(shown.clone());
    let (id, password, link) = &shown;
    let changed = |i: usize| match &previous {
        None => true,
        Some(p) => [&p.0, &p.1, &p.2][i] != [id, password, link][i],
    };
    if changed(0) || changed(1) {
        if previous.is_some() && !changed(0) {
            println!("Nuova password:");
        }
        print_credentials(id, password);
    }
    if changed(2) {
        let text = match link.as_str() {
            Some("online") => "in linea, pronto a ricevere connessioni",
            Some("connecting") => "connessione al server...",
            _ => "non in linea (raggiungibile solo in rete locale)",
        };
        let error = status["error"]
            .as_str()
            .map(|e| format!(" — {e}"))
            .unwrap_or_default();
        println!("  Stato:     {text}{error}");
    }
}

fn ask_approval(state: &mut State, request: &Value) {
    let Some(session) = request["session"].as_u64() else {
        return;
    };
    let peer = request["peer"].as_str().unwrap_or("?").to_owned();
    let secs = request["timeout_secs"].as_u64().unwrap_or(0);
    println!(
        "\n? {peer} vuole collegarsi a questo computer. Accettare? [s/n] (scade tra {secs} s)"
    );
    state
        .questions
        .push_back(Question::Approval { session, peer });
}

fn on_message(state: &Mutex<State>, message: &Value) {
    let mut state = state.lock().unwrap();
    match message["kind"].as_str().unwrap_or("") {
        "status" => show_status(&mut state, message),
        "approval_request" => ask_approval(&mut state, message),
        "approval_closed" => {
            let session = message["session"].as_u64();
            let before = state.questions.len();
            let mut peer = None;
            state.questions.retain(|q| match q {
                Question::Approval {
                    session: s,
                    peer: p,
                } if Some(*s) == session => {
                    peer = Some(p.clone());
                    false
                }
                _ => true,
            });
            if state.questions.len() != before {
                println!(
                    "La richiesta di {} è scaduta o ha avuto risposta dalla finestra.",
                    peer.unwrap_or_default()
                );
            }
        }
        "hosting_started" => {
            let peer = message["peer"].as_str().unwrap_or("?").to_owned();
            println!("● Connesso: {peer} vede e controlla questo computer. (d per disconnettere)");
            if let Some(session) = message["session"].as_u64() {
                state.hosting.push((session, peer));
            }
        }
        "hosting_ended" => {
            let peer = message["peer"].as_str().unwrap_or("?");
            let secs = message["duration_secs"].as_u64().unwrap_or(0);
            println!("○ {peer} si è scollegato dopo {}.", duration(secs));
            let session = message["session"].as_u64();
            state.hosting.retain(|h| Some(h.0) != session);
        }
        "log" => {
            if let Some(text) = message["message"].as_str() {
                println!("  · {text}");
            }
        }
        "remote_only" => {
            if let Some(error) = message["error"].as_str() {
                println!("Schermo virtuale: {error}");
            } else {
                state.remote_only = message["enabled"] == true;
                println!(
                    "{}",
                    if state.remote_only {
                        "Schermo virtuale come principale: acceso (Computer usato solo da remoto)."
                    } else {
                        "Schermo virtuale come principale: spento."
                    }
                );
            }
        }
        "virtual_monitor" => {
            if let Some(error) = message["error"].as_str() {
                println!("Monitor virtuale: {error}");
            } else {
                let monitor = &message["monitor"];
                state.monitor_installed = monitor["installed"] == true;
                let text = monitor["message"]
                    .as_str()
                    .unwrap_or(if state.monitor_installed {
                        "installato"
                    } else {
                        "rimosso"
                    });
                println!("Monitor virtuale: {text}");
            }
        }
        _ => {}
    }
}

fn duration(secs: u64) -> String {
    match secs {
        0..60 => format!("{secs} s"),
        60..3600 => format!("{} min {} s", secs / 60, secs % 60),
        _ => format!("{} h {} min", secs / 3600, secs % 3600 / 60),
    }
}

#[cfg(windows)]
mod console {
    //! The app is a GUI program on Windows: borrow the terminal it was started from.

    use std::ffi::c_void;
    use std::os::windows::io::IntoRawHandle;

    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    const STD_INPUT_HANDLE: u32 = -10i32 as u32;
    const STD_OUTPUT_HANDLE: u32 = -11i32 as u32;
    const STD_ERROR_HANDLE: u32 = -12i32 as u32;

    unsafe extern "system" {
        fn AttachConsole(process: u32) -> i32;
        fn AllocConsole() -> i32;
        fn GetStdHandle(which: u32) -> *mut c_void;
        fn SetStdHandle(which: u32, handle: *mut c_void) -> i32;
    }

    pub fn attach() {
        // SAFETY: plain Win32 calls; the handles opened below live until the process ends.
        unsafe {
            if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
                AllocConsole();
            }
            for (which, name, write) in [
                (STD_INPUT_HANDLE, "CONIN$", false),
                (STD_OUTPUT_HANDLE, "CONOUT$", true),
                (STD_ERROR_HANDLE, "CONOUT$", true),
            ] {
                let current = GetStdHandle(which);
                // Redirected (pipe, file) handles stay as they are.
                if !current.is_null() && current as isize != -1 {
                    continue;
                }
                if let Ok(file) = std::fs::OpenOptions::new()
                    .read(true)
                    .write(write)
                    .open(name)
                {
                    SetStdHandle(which, file.into_raw_handle());
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------------------
// Installing the command (Preferences).

#[derive(Debug, Clone, Default, Serialize)]
pub struct CliCommand {
    /// This build can install the command (not Quick Support, which runs from anywhere).
    pub supported: bool,
    /// The command is there and starts this copy of the app.
    pub installed: bool,
    pub path: Option<String>,
    /// Outcome of the last install, for the user.
    pub message: Option<String>,
}

fn script_path() -> Option<PathBuf> {
    if crate::QUICK {
        return None;
    }
    #[cfg(target_os = "macos")]
    return Some(PathBuf::from("/usr/local/bin").join(COMMAND_NAME));
    #[cfg(target_os = "linux")]
    return Some(dirs::home_dir()?.join(".local/bin").join(COMMAND_NAME));
    #[cfg(windows)]
    return Some(
        dirs::data_local_dir()?
            .join(&crate::context().config().identifier)
            .join("bin")
            .join(format!("{COMMAND_NAME}.cmd")),
    );
    #[allow(unreachable_code)]
    None
}

fn script(exe: &Path) -> String {
    let exe = exe.display().to_string();
    if cfg!(windows) {
        format!("@echo off\r\n\"{exe}\" {CLI_ARG} %*\r\n")
    } else {
        format!(
            "#!/bin/sh\n# Kronos Remote Support in text mode.\nexec '{}' {CLI_ARG} \"$@\"\n",
            exe.replace('\'', r"'\''")
        )
    }
}

fn command_status() -> CliCommand {
    let Some(path) = script_path() else {
        return CliCommand::default();
    };
    let installed = std::env::current_exe()
        .ok()
        .zip(std::fs::read_to_string(&path).ok())
        .is_some_and(|(exe, text)| text == script(&exe));
    CliCommand {
        supported: true,
        installed,
        path: Some(path.display().to_string()),
        message: None,
    }
}

#[tauri::command]
pub fn cli_command_status() -> CliCommand {
    command_status()
}

/// Writes (or refreshes) the command; macOS asks for an administrator password when
/// `/usr/local/bin` is not writable.
#[tauri::command]
pub async fn install_cli_command() -> Result<CliCommand, String> {
    tauri::async_runtime::spawn_blocking(install)
        .await
        .map_err(|e| e.to_string())?
}

fn install() -> Result<CliCommand, String> {
    let path = script_path().ok_or("non disponibile in questa versione dell'app")?;
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    #[cfg(target_os = "macos")]
    if exe.to_string_lossy().contains("/AppTranslocation/") {
        return Err("sposta prima l'app nella cartella Applicazioni".into());
    }
    let text = script(&exe);
    let written = path
        .parent()
        .map(std::fs::create_dir_all)
        .unwrap_or(Ok(()))
        .and_then(|()| std::fs::write(&path, &text));
    match written {
        Ok(()) => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
                    .map_err(|e| e.to_string())?;
            }
        }
        #[cfg(target_os = "macos")]
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            install_as_admin(&path, &text)?
        }
        Err(e) => return Err(format!("{}: {e}", path.display())),
    }
    #[cfg(windows)]
    add_to_path(path.parent().ok_or("cartella del comando")?)?;
    tracing::info!("text mode command installed at {}", path.display());
    let mut status = command_status();
    status.message = Some(if cfg!(windows) {
        format!("Installato. Apri un nuovo terminale e scrivi {COMMAND_NAME}.")
    } else {
        format!("Installato. Scrivi {COMMAND_NAME} in un terminale.")
    });
    Ok(status)
}

/// `/usr/local/bin` belongs to root: copy the script there with the administrator prompt.
#[cfg(target_os = "macos")]
fn install_as_admin(path: &Path, text: &str) -> Result<(), String> {
    let temp = std::env::temp_dir().join(format!("{COMMAND_NAME}-{}", std::process::id()));
    std::fs::write(&temp, text).map_err(|e| e.to_string())?;
    let quote = |p: &Path| format!("'{}'", p.display().to_string().replace('\'', r"'\''"));
    let shell = format!(
        "mkdir -p {dir} && cp {temp} {path} && chmod 755 {path}",
        dir = quote(path.parent().unwrap_or(Path::new("/usr/local/bin"))),
        temp = quote(&temp),
        path = quote(path),
    );
    let apple_script = format!(
        "do shell script \"{}\" with administrator privileges",
        shell.replace('\\', "\\\\").replace('"', "\\\"")
    );
    let output = std::process::Command::new("osascript")
        .arg("-e")
        .arg(apple_script)
        .output();
    let _ = std::fs::remove_file(&temp);
    let output = output.map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(())
    } else if String::from_utf8_lossy(&output.stderr).contains("-128") {
        Err("installazione annullata".into())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).trim().to_owned())
    }
}

/// Adds the command's folder to the user's PATH (new terminals see it).
#[cfg(windows)]
fn add_to_path(dir: &Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let dir = dir.display().to_string().replace('\'', "''");
    let script = format!(
        "$d='{dir}'; $p=[Environment]::GetEnvironmentVariable('Path','User'); \
         if (-not (($p -split ';') -contains $d)) {{ \
         [Environment]::SetEnvironmentVariable('Path', ((@($p.TrimEnd(';'), $d) | Where-Object {{ $_ }}) -join ';'), 'User') }}"
    );
    let status = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|e| format!("PATH: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("aggiunta della cartella al PATH non riuscita".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_round_trip() {
        let text = serde_json::to_string(&Request::Approve {
            session: 7,
            accept: true,
        })
        .unwrap();
        assert_eq!(text, r#"{"cmd":"approve","session":7,"accept":true}"#);
        assert!(matches!(
            serde_json::from_str::<Request>(r#"{"cmd":"virtual_monitor","install":true}"#),
            Ok(Request::VirtualMonitor { install: true })
        ));
    }

    #[test]
    fn script_quotes_the_path() {
        let text = script(Path::new("/Applications/Kronos Remote Support.app/x'y"));
        if cfg!(windows) {
            assert!(text.contains("--cli %*"));
        } else {
            assert!(text.contains(r"exec '/Applications/Kronos Remote Support.app/x'\''y' --cli"));
        }
    }

    #[test]
    fn durations() {
        assert_eq!(duration(5), "5 s");
        assert_eq!(duration(125), "2 min 5 s");
        assert_eq!(duration(3720), "1 h 2 min");
    }
}
