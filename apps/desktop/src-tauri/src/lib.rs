mod devices_file;
mod permissions;
mod privacy;
mod service;
mod store;
mod uac;
mod updates;
mod virtual_monitor;

use service::{Service, Status};
use signaling_proto::PeerId;
use tauri::ipc::{Channel, InvokeResponseBody};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, State, WindowEvent};
use tauri_plugin_autostart::ManagerExt;
use tracing_subscriber::Layer;

/// Command-line flag used by the login item: start without showing the window.
const HIDDEN_ARG: &str = "--hidden";

/// "Quick access" build (ADR 0031): runs without installation and only shares this screen.
/// No tray, login item, updater or saved devices; closing the window quits. It keeps its
/// id and an optional personal password (unattended access while it runs) in its own data
/// folder (ADR 0052).
pub(crate) const QUICK: bool = cfg!(feature = "quick-access");

/// The app's data folder name: `identifier` in tauri.conf.json (checked by a test). The
/// Windows service copies the device's id and personal password from there (ADR 0056).
#[cfg_attr(not(windows), allow(dead_code))]
const APP_IDENTIFIER: &str = "tech.kronos.desktopremoto";

/// Windows: the data folder of the host that runs before anyone logs in (ADR 0056), when
/// this process is that host. It runs as SYSTEM with no windows, tray or updates, and only
/// accepts the personal password: nobody is there to approve a connection.
static LOGON_HOST: std::sync::OnceLock<std::path::PathBuf> = std::sync::OnceLock::new();

pub(crate) fn logon_host() -> bool {
    LOGON_HOST.get().is_some()
}

/// File name of the installed app on Windows (Cargo binary name), as it runs in the user's
/// session; the pre-login host runs as the service's protected copy instead.
#[cfg(windows)]
pub(crate) const APP_EXE: &str = "desktop-remoto.exe";

/// The pre-login host's data folder (ADR 0056).
#[cfg(windows)]
pub(crate) fn logon_host_dir() -> Option<&'static std::path::Path> {
    LOGON_HOST.get().map(|dir| dir.as_path())
}

/// Product tag for the Windows agent and service (separate pipe/service per flavour).
pub(crate) fn agent_product() -> &'static str {
    if QUICK { "quick" } else { "remote" }
}

/// Windows service name per product (stable across versions).
#[cfg(windows)]
fn service_name() -> &'static str {
    if QUICK {
        "KronosQuickSupportAgent"
    } else {
        "KronosRemoteSupportAgent"
    }
}

/// Which flavour of the app this is, for the UI.
#[tauri::command]
fn app_mode() -> &'static str {
    if QUICK { "quick" } else { "full" }
}

/// Quick Support, Windows: relaunch elevated so it can control admin windows. On success
/// this instance quits (the elevated one takes over); `false` if the user declined UAC.
#[tauri::command]
fn relaunch_elevated(app: AppHandle) -> Result<bool, String> {
    #[cfg(windows)]
    {
        match agent::windows::spawn_self_elevated(&[]) {
            Ok(true) => {
                app.exit(0);
                Ok(true)
            }
            Ok(false) => Ok(false),
            Err(e) => Err(format!("{e:#}")),
        }
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        Err("non disponibile".into())
    }
}

#[tauri::command]
fn get_status(service: State<'_, Service>) -> Status {
    service.status()
}

fn parse_target(service: &Service, target: &str) -> Result<PeerId, String> {
    let target: PeerId = target
        .parse()
        .map_err(|_| "ID non valido: servono 9 cifre".to_owned())?;
    if service.status().id.as_deref() == Some(target.to_string().as_str()) {
        return Err("questo è il tuo ID".into());
    }
    Ok(target)
}

/// Connects with the partner's temporary password, or with its personal password
/// (`personal`, no approval); `save_as` also remembers the device. False when a session
/// to that device was already open: its window is brought forward instead.
#[tauri::command]
async fn connect_to(
    app: AppHandle,
    service: State<'_, Service>,
    target: String,
    password: String,
    personal: bool,
    save_as: Option<String>,
) -> Result<bool, String> {
    let target = parse_target(&service, &target)?;
    if show_existing_view(&app, &service, &target) {
        return Ok(false);
    }
    if personal {
        if password.is_empty() {
            return Err("inserisci la password personale del dispositivo".into());
        }
        let save_as = save_as
            .map(|n| n.trim().to_owned())
            .filter(|n| !n.is_empty());
        return service
            .connect_personal(target, &password, save_as)
            .map(|()| true);
    }
    let password = transport::normalize_password(&password);
    if password.is_empty() || !password.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err("password non valida: solo lettere minuscole e cifre".into());
    }
    service.connect(target, password);
    Ok(true)
}

/// Already viewing `target`: brings that session's window forward instead of opening a
/// second session to the same device. True if there was one.
fn show_existing_view(app: &AppHandle, service: &Service, target: &PeerId) -> bool {
    let Some(session) = service.viewing(target) else {
        return false;
    };
    if let Some(window) = app.get_webview_window(&format!("view-{session}")) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
    true
}

#[tauri::command]
fn cancel_connect(service: State<'_, Service>) {
    service.cancel_connect();
}

/// Online state of the saved devices (id digits → online; `None` while unknown); also
/// asks the server again.
#[tauri::command]
fn device_presence(service: State<'_, Service>) -> Option<std::collections::HashMap<String, bool>> {
    service.presence()
}

/// Devices of this app on the local network (connectable without the server).
#[tauri::command]
fn lan_peers(service: State<'_, Service>) -> Vec<service::LanPeerView> {
    service.lan_peers()
}

#[tauri::command]
fn set_lan_name(
    service: State<'_, Service>,
    id: String,
    name: String,
) -> Result<Vec<service::LanPeerView>, String> {
    service.set_lan_name(&id, &name)
}

#[tauri::command]
fn connect_device(app: AppHandle, service: State<'_, Service>, id: String) -> Result<bool, String> {
    if let Ok(target) = id
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>()
        .parse()
        && show_existing_view(&app, &service, &target)
    {
        return Ok(false);
    }
    service.connect_device(&id).map(|()| true)
}

#[tauri::command]
fn remove_device(
    service: State<'_, Service>,
    id: String,
) -> Result<Vec<service::DeviceView>, String> {
    service.remove_device(&id)?;
    Ok(service.devices())
}

/// Saved devices and their folders (ADR 0052), for the tree in "I miei dispositivi".
#[derive(serde::Serialize)]
struct DeviceTree {
    devices: Vec<service::DeviceView>,
    groups: Vec<store::DeviceGroup>,
}

fn device_tree(service: &Service) -> DeviceTree {
    DeviceTree {
        devices: service.devices(),
        groups: service.groups(),
    }
}

#[tauri::command]
fn list_device_tree(service: State<'_, Service>) -> DeviceTree {
    device_tree(&service)
}

/// Writes "my devices" to a file in the Downloads folder (ADR 0057); with `password` the
/// saved passwords go too, encrypted. Returns the file's path.
#[tauri::command]
async fn export_devices(
    app: AppHandle,
    service: State<'_, Service>,
    password: Option<String>,
) -> Result<String, String> {
    let list = devices_file::DeviceList {
        devices: service.store().devices(),
        groups: service.groups(),
    };
    let text = tauri::async_runtime::spawn_blocking(move || {
        devices_file::export(list, password.as_deref())
    })
    .await
    .map_err(|e| e.to_string())??;
    let dir = app
        .path()
        .download_dir()
        .or_else(|_| app.path().home_dir())
        .map_err(|e| format!("cartella Download non trovata: {e}"))?;
    let path = (1..1000)
        .map(|n| {
            let name = if n == 1 {
                "dispositivi-kronos.json".to_owned()
            } else {
                format!("dispositivi-kronos ({n}).json")
            };
            dir.join(name)
        })
        .find(|p| !p.exists())
        .ok_or("troppi file esportati nella cartella Download")?;
    std::fs::write(&path, text).map_err(|e| format!("impossibile scrivere il file: {e}"))?;
    Ok(path.display().to_string())
}

/// Whether an exported file needs its password to be imported.
#[tauri::command]
fn devices_file_locked(content: String) -> Result<bool, String> {
    devices_file::needs_password(&content)
}

#[derive(serde::Serialize)]
struct Imported {
    devices: usize,
    groups: usize,
    tree: DeviceTree,
}

/// Adds the devices and folders of an exported file to "my devices".
#[tauri::command]
async fn import_devices(
    service: State<'_, Service>,
    content: String,
    password: Option<String>,
) -> Result<Imported, String> {
    let list = tauri::async_runtime::spawn_blocking(move || {
        devices_file::import(&content, password.as_deref())
    })
    .await
    .map_err(|e| e.to_string())??;
    let (devices, groups) = service.store().import(list.devices, list.groups)?;
    let _ = service.presence();
    Ok(Imported {
        devices,
        groups,
        tree: device_tree(&service),
    })
}

#[tauri::command]
fn create_group(
    service: State<'_, Service>,
    name: String,
    parent: Option<String>,
) -> Result<DeviceTree, String> {
    service.store().create_group(&name, parent.as_deref())?;
    Ok(device_tree(&service))
}

#[tauri::command]
fn rename_group(
    service: State<'_, Service>,
    id: String,
    name: String,
) -> Result<DeviceTree, String> {
    service.store().rename_group(&id, &name)?;
    Ok(device_tree(&service))
}

#[tauri::command]
fn move_group(
    service: State<'_, Service>,
    id: String,
    parent: Option<String>,
) -> Result<DeviceTree, String> {
    service.store().move_group(&id, parent.as_deref())?;
    Ok(device_tree(&service))
}

#[tauri::command]
fn set_group_collapsed(
    service: State<'_, Service>,
    id: String,
    collapsed: bool,
) -> Result<(), String> {
    service.store().set_group_collapsed(&id, collapsed)
}

/// Removes a folder; what it held moves to the folder above.
#[tauri::command]
fn delete_group(service: State<'_, Service>, id: String) -> Result<DeviceTree, String> {
    service.store().delete_group(&id)?;
    Ok(device_tree(&service))
}

#[tauri::command]
fn move_device(
    service: State<'_, Service>,
    id: String,
    group: Option<String>,
) -> Result<DeviceTree, String> {
    let id: String = id.chars().filter(|c| !c.is_whitespace()).collect();
    service.store().move_device(&id, group.as_deref())?;
    Ok(device_tree(&service))
}

/// Sets (or with `None` disables) this device's personal password for unattended access.
#[tauri::command]
async fn set_personal_password(
    service: State<'_, Service>,
    password: Option<String>,
) -> Result<Status, String> {
    service.set_personal_password(password.as_deref())
}

#[tauri::command]
fn get_autostart(app: AppHandle) -> bool {
    !QUICK && app.autolaunch().is_enabled().unwrap_or(false)
}

#[tauri::command]
fn set_autostart(app: AppHandle, enabled: bool) -> Result<bool, String> {
    if QUICK {
        return Err("non disponibile in Quick access".into());
    }
    let launcher = app.autolaunch();
    let result = if enabled {
        launcher.enable()
    } else {
        launcher.disable()
    };
    result.map_err(|e| format!("impossibile modificare l'avvio automatico: {e}"))?;
    Ok(launcher.is_enabled().unwrap_or(enabled))
}

#[tauri::command]
fn new_password(service: State<'_, Service>) -> Status {
    service.new_password()
}

#[tauri::command]
fn respond_approval(service: State<'_, Service>, session: u64, accept: bool) {
    service.respond(session, accept);
}

/// The viewer screen is ready to receive frames for `session`.
#[tauri::command]
fn attach_view(
    service: State<'_, Service>,
    session: u64,
    frames: Channel<InvokeResponseBody>,
) -> Option<Vec<service::UiEvent>> {
    service.attach_view(session, frames)
}

#[tauri::command]
fn set_clipboard(service: State<'_, Service>, session: u64, enabled: bool) {
    service.set_clipboard(session, enabled);
}

#[tauri::command]
fn send_chat(
    app: AppHandle,
    window: tauri::WebviewWindow,
    service: State<'_, Service>,
    session: u64,
    text: String,
) -> bool {
    service.send_chat(&app, session, text, window.label().to_owned())
}

#[tauri::command]
fn host_chat_history(service: State<'_, Service>, session: u64) -> Vec<service::ChatLine> {
    service.host_chat_history(session)
}

/// Size of the floating chat window on the shared computer.
const CHAT_POPUP_SIZE: (f64, f64) = (360.0, 320.0);

/// Shows the floating chat of a shared session, top right and above everything, without
/// taking the keyboard from the user (the page knows the session from `chat-<session>`).
pub(crate) fn open_chat_popup(app: &AppHandle, session: u64, peer: &str) {
    // Before login nobody would read it (and the host opens no windows).
    if logon_host() {
        return;
    }
    let label = format!("chat-{session}");
    if let Some(window) = app.get_webview_window(&label) {
        let _ = window.unminimize();
        let _ = window.show();
        return;
    }
    let (width, height) = CHAT_POPUP_SIZE;
    // Top right of the main screen, below the macOS menu bar.
    let position = app.primary_monitor().ok().flatten().map(|monitor| {
        let scale = monitor.scale_factor();
        let screen = monitor.size().to_logical::<f64>(scale);
        let origin = monitor.position().to_logical::<f64>(scale);
        (origin.x + screen.width - width - 16.0, origin.y + 48.0)
    });
    let url = tauri::WebviewUrl::App("index.html".into());
    let mut builder = tauri::WebviewWindowBuilder::new(app, &label, url)
        .title(format!("Chat con {peer}"))
        .inner_size(width, height)
        .min_inner_size(280.0, 220.0)
        .always_on_top(true)
        .visible_on_all_workspaces(true)
        .skip_taskbar(true)
        .focused(false);
    if let Some((x, y)) = position {
        builder = builder.position(x, y);
    }
    if let Err(e) = builder.build() {
        tracing::warn!(session, "cannot open the chat window: {e}");
    }
}

pub(crate) fn close_chat_popup(app: &AppHandle, session: u64) {
    if let Some(window) = app.get_webview_window(&format!("chat-{session}")) {
        let _ = window.close();
    }
}

/// Opens (or focuses) the window for a remote session's screen. The page knows which
/// session it shows from the window label (`view-<session>`).
pub(crate) fn open_view_window(app: &AppHandle, session: u64, peer: &str) {
    let label = format!("view-{session}");
    if let Some(window) = app.get_webview_window(&label) {
        let _ = window.set_focus();
        return;
    }
    let url = tauri::WebviewUrl::App("index.html".into());
    let built = tauri::WebviewWindowBuilder::new(app, &label, url)
        .title(format!("{peer} — Kronos Remote Support"))
        .inner_size(1280.0, 800.0)
        .min_inner_size(640.0, 420.0)
        .focused(true)
        .build();
    match built {
        Ok(window) => {
            let _ = window.set_focus();
        }
        Err(e) => tracing::warn!(session, "cannot open the viewer window: {e}"),
    }
}

/// Opens (or brings forward) the file manager of a session we view (ADR 0053): this
/// computer's folders on one side, the partner's on the other.
/// Async on purpose: a window built from a synchronous command deadlocks the whole app on
/// Windows (Tauri runs those on the main thread, which the new window needs too).
#[tauri::command]
async fn open_files_window(app: AppHandle, session: u64, peer: String) -> Result<(), String> {
    let label = format!("files-{session}");
    if let Some(window) = app.get_webview_window(&label) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }
    let url = tauri::WebviewUrl::App("index.html".into());
    tauri::WebviewWindowBuilder::new(&app, &label, url)
        .title(format!("Trasferimento file — {peer}"))
        .inner_size(1100.0, 680.0)
        .min_inner_size(720.0, 420.0)
        .focused(true)
        .build()
        .map(|_| ())
        .map_err(|e| format!("impossibile aprire il trasferimento file: {e}"))
}

/// A folder of this computer (`None`: the home folder).
#[tauri::command]
async fn fs_local_list(path: Option<String>) -> Result<stream::fs::Listing, String> {
    tauri::async_runtime::spawn_blocking(move || stream::fs::list_dir(path.as_deref()))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn fs_local_mkdir(path: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || stream::fs::make_dir(&path))
        .await
        .map_err(|e| e.to_string())?
}

/// A folder of the partner (`None`: its user's home folder).
#[tauri::command]
async fn fs_remote_list(
    service: State<'_, Service>,
    session: u64,
    path: Option<String>,
) -> Result<stream::fs::Listing, String> {
    let client = service.file_manager(session)?;
    client.list(path).await
}

#[tauri::command]
async fn fs_remote_mkdir(
    service: State<'_, Service>,
    session: u64,
    path: String,
) -> Result<(), String> {
    let client = service.file_manager(session)?;
    client.make_dir(path).await
}

/// Copies `paths` into the folder `dest` on the other side (`upload`: from this computer
/// to the partner); returns the copy's number. Progress and outcome come as events.
#[tauri::command]
fn fs_copy(
    service: State<'_, Service>,
    session: u64,
    upload: bool,
    paths: Vec<String>,
    dest: String,
    replace: Option<bool>,
) -> Result<u32, String> {
    service.copy_files(session, upload, paths, dest, replace.unwrap_or(false))
}

#[tauri::command]
async fn fs_cancel(service: State<'_, Service>, session: u64, req: u32) -> Result<(), String> {
    let client = service.file_manager(session)?;
    client.cancel(req).await;
    Ok(())
}

/// The viewer UI processed `count` video frames (flow control, see service.rs).
#[tauri::command]
fn frames_consumed(service: State<'_, Service>, session: u64, count: u32) {
    service.frames_consumed(session, count);
}

#[tauri::command]
fn request_keyframe(service: State<'_, Service>, session: u64) {
    service.request_keyframe(session);
}

#[tauri::command]
fn send_input(service: State<'_, Service>, session: u64, event: protocol::InputEvent) {
    service.send_input(session, event);
}

#[tauri::command]
fn display_name(service: State<'_, Service>) -> Option<String> {
    service.display_name()
}

#[tauri::command]
fn set_display_name(service: State<'_, Service>, name: String) -> Option<String> {
    service.set_display_name(&name)
}

#[tauri::command]
fn remote_only(service: State<'_, Service>) -> bool {
    service.remote_only()
}

#[tauri::command]
async fn set_remote_only(service: State<'_, Service>, enabled: bool) -> Result<bool, String> {
    service.set_remote_only(enabled).await
}

#[tauri::command]
fn host_permissions(service: State<'_, Service>) -> store::HostPermissions {
    service.host_permissions()
}

#[tauri::command]
fn set_host_permissions(
    service: State<'_, Service>,
    allowed: store::HostPermissions,
) -> Result<store::HostPermissions, String> {
    service.set_host_permissions(allowed)
}

#[tauri::command]
fn set_block_input(service: State<'_, Service>, session: u64, enabled: bool) {
    service.set_block_input(session, enabled);
}

#[tauri::command]
fn match_resolution(
    service: State<'_, Service>,
    session: u64,
    width: u32,
    height: u32,
    scale: f32,
) {
    service.match_resolution(session, width, height, scale);
}

#[tauri::command]
fn cancel_transfer(service: State<'_, Service>, session: u64) {
    service.cancel_transfer(session);
}

#[tauri::command]
fn set_audio_volume(service: State<'_, Service>, session: u64, volume: f32) {
    service.set_audio_volume(session, volume);
}

#[tauri::command]
fn set_audio(service: State<'_, Service>, session: u64, enabled: bool) {
    service.set_audio(session, enabled);
}

/// This machine can play a partner's sound (macOS, Windows).
#[tauri::command]
fn audio_supported() -> bool {
    audio::playback_supported()
}

#[tauri::command]
fn set_high_resolution(service: State<'_, Service>, session: u64, enabled: bool) {
    service.set_high_resolution(session, enabled);
}

#[tauri::command]
fn select_display(service: State<'_, Service>, session: u64, id: u32) {
    service.select_display(session, id);
}

#[tauri::command]
fn close_view(service: State<'_, Service>, session: u64) {
    service.close_view(session);
}

/// The end-of-session summary was acknowledged: make sure nothing of the session is left
/// (connection, sharing, the remote screen's window).
#[tauri::command]
fn end_session(app: AppHandle, service: State<'_, Service>, session: u64) {
    service.end_session(session);
    if let Some(window) = app.get_webview_window(&format!("view-{session}")) {
        let _ = window.close();
    }
}

#[tauri::command]
fn stop_hosting(service: State<'_, Service>, session: u64) {
    service.stop_hosting(session);
}

/// Logs to stdout and, when possible, to daily files in the app log directory
/// (macOS: ~/Library/Logs/tech.kronos.desktopremoto), kept for 7 days.
fn init_logging(dir: Option<std::path::PathBuf>) {
    init_logging_as(dir, "kronos-remote-support");
}

/// The app's configuration; the pre-login host opens no window at all.
fn context() -> tauri::Context<tauri::Wry> {
    let mut context = tauri::generate_context!();
    if logon_host() {
        context.config_mut().app.windows.clear();
    }
    context
}

fn init_logging_as(dir: Option<std::path::PathBuf>, prefix: &str) {
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;

    let filter = || {
        tracing_subscriber::EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| "info,rtc_ice=error,rtc_dtls=error,rtc_mdns=error".into())
    };
    let file = dir.and_then(|dir| {
        tracing_appender::rolling::RollingFileAppender::builder()
            .rotation(tracing_appender::rolling::Rotation::DAILY)
            .filename_prefix(prefix)
            .filename_suffix("log")
            .max_log_files(7)
            .build(dir)
            .ok()
    });
    let file_layer = file.map(|appender| {
        let (writer, guard) = tracing_appender::non_blocking(appender);
        // Lives for the whole process: dropping it would stop file logging.
        std::mem::forget(guard);
        tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .with_writer(writer)
            .with_filter(filter())
    });
    let _ = tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer().with_filter(filter()))
        .with(file_layer)
        .try_init();
}

/// Brings the main window back (it is hidden, never destroyed, while the app runs).
pub(crate) fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// Tray icon: the app keeps accepting connections with its window hidden, like TeamViewer.
fn build_tray(app: &tauri::App) -> tauri::Result<()> {
    let show = MenuItem::with_id(
        app,
        "show",
        "Mostra Kronos Remote Support",
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", "Esci", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;
    let mut tray = TrayIconBuilder::with_id("main")
        .tooltip("Kronos Remote Support")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => show_main_window(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

/// Debug builds only: `KRS_TEST_CONNECT=<id>:<password>` connects once online, so the
/// viewer window can be exercised without typing into the UI.
fn test_connect(app: &AppHandle) {
    if !cfg!(debug_assertions) {
        return;
    }
    let Some((id, password)) = std::env::var("KRS_TEST_CONNECT")
        .ok()
        .and_then(|v| v.split_once(':').map(|(a, b)| (a.to_owned(), b.to_owned())))
    else {
        return;
    };
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let service = app.state::<Service>();
        while !matches!(service.status().link, service::Link::Online) {
            tokio::time::sleep(std::time::Duration::from_millis(200)).await;
        }
        if let Ok(target) = id.parse() {
            service.connect(target, password);
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
/// Async worker threads of the app (sessions, signaling, WebRTC networking).
const ASYNC_WORKERS: usize = 4;

pub fn run() {
    // Windows helper modes, before any Tauri/window setup.
    #[cfg(windows)]
    {
        let args: Vec<String> = std::env::args().collect();
        match args.get(1).map(String::as_str) {
            // Agent: runs as SYSTEM in the user's session, serves the app over a pipe.
            Some("--agent") => {
                let product = args
                    .get(2)
                    .cloned()
                    .unwrap_or_else(|| agent_product().to_owned());
                let app = args.get(3).map(std::path::PathBuf::from);
                init_logging(agent::windows::log_dir());
                if let Err(e) = agent::windows::run_agent(&product, app.as_deref()) {
                    tracing::error!("agent exited: {e:#}");
                }
                return;
            }
            // Install/remove the service (already elevated by the UAC prompt we requested).
            Some("--install-service") => {
                let exe = std::env::current_exe().expect("exe path");
                let r = agent::windows::install_service(
                    service_name(),
                    &format!(
                        "{} (controllo UAC)",
                        if QUICK {
                            "Kronos Quick Support"
                        } else {
                            "Kronos Remote Support"
                        }
                    ),
                    &exe,
                    agent_product(),
                    true,
                );
                // A remote-only computer stays one across reinstalls (service updates).
                let r = r.and_then(|()| {
                    if args.iter().any(|a| a == "--remote-only") {
                        agent::windows::set_remote_only(service_name(), true)
                    } else {
                        Ok(())
                    }
                });
                std::process::exit(if r.is_ok() { 0 } else { 1 });
            }
            // Mark this computer remote-only for the user logged on here, or clear the mark
            // (already elevated by the UAC prompt we requested).
            Some("--set-remote-only") => {
                let enabled = args.get(2).is_some_and(|a| a == "on");
                let r = agent::windows::set_remote_only(service_name(), enabled);
                std::process::exit(if r.is_ok() { 0 } else { 1 });
            }
            // Started by the service in the console session (remote-only computer).
            Some("--to-console") => {
                init_logging(agent::windows::log_dir());
                let r = args
                    .get(2)
                    .and_then(|s| s.parse().ok())
                    .ok_or_else(|| anyhow::anyhow!("missing session"))
                    .and_then(agent::windows::move_session_to_console);
                if let Err(e) = &r {
                    tracing::warn!("cannot move the session to the console: {e:#}");
                }
                std::process::exit(if r.is_ok() { 0 } else { 1 });
            }
            Some("--uninstall-service") => {
                let r = agent::windows::uninstall_service(service_name());
                std::process::exit(if r.is_ok() { 0 } else { 1 });
            }
            // Service: supervises the agent in the active console session.
            Some("--service") => {
                let name = args
                    .get(2)
                    .cloned()
                    .unwrap_or_else(|| service_name().to_owned());
                let product = args
                    .get(3)
                    .cloned()
                    .unwrap_or_else(|| agent_product().to_owned());
                let app = args.get(4).map(std::path::PathBuf::from);
                init_logging(agent::windows::log_dir());
                let identifier = (!QUICK).then_some(APP_IDENTIFIER);
                if let Err(e) =
                    agent::windows::run_service(&name, &product, app.as_deref(), identifier)
                {
                    tracing::error!("service exited: {e:#}");
                }
                return;
            }
            // Host before login, started by the service as SYSTEM (ADR 0056).
            Some("--logon-host") if !QUICK => {
                let Some(dir) = args.get(2).map(std::path::PathBuf::from) else {
                    return;
                };
                agent::windows::prepare_system_host();
                let _ = LOGON_HOST.set(dir);
            }
            _ => {
                // Quick Support is opt-out for admin: try to gain admin at startup so admin
                // windows work; if the user clicks "No" on the UAC prompt, continue as user.
                if QUICK
                    && !agent::windows::is_elevated()
                    && let Ok(true) = agent::windows::spawn_self_elevated(&[])
                {
                    // The elevated copy takes over; this one exits.
                    return;
                }
            }
        }
    }

    // Two instances would share the device key and fight over the same id. The pre-login
    // host has its own copy of the key and hands over to the user's app (ADR 0056).
    // A few async workers instead of one per core: capture, encoding, decoding, file work and
    // other heavy jobs run on their own threads or `spawn_blocking`.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(ASYNC_WORKERS)
        .enable_all()
        .build()
        .expect("async runtime");
    tauri::async_runtime::set(runtime.handle().clone());
    // The runtime lives as long as the app (`run` only returns at exit).
    std::mem::forget(runtime);
    let mut builder = tauri::Builder::default();
    if !logon_host() {
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_main_window(app);
        }));
    }
    if !QUICK && !logon_host() {
        builder = builder
            // Launched at login: start hidden in the tray, ready for unattended access.
            .plugin(tauri_plugin_autostart::init(
                tauri_plugin_autostart::MacosLauncher::LaunchAgent,
                Some(vec![HIDDEN_ARG]),
            ))
            .plugin(tauri_plugin_updater::Builder::new().build());
    }
    let app = builder
        .setup(|app| {
            if let Some(dir) = LOGON_HOST.get() {
                #[cfg(windows)]
                init_logging_as(agent::windows::log_dir(), "kronos-logon-host");
                tracing::info!("host before login starting");
                app.manage(Service::start(app.handle().clone(), dir.clone())?);
                return Ok(());
            }
            if QUICK {
                // Logs to the console only; the id and the personal password are kept in its
                // own data folder (not shared with the full app), so the id stays the same.
                init_logging(None);
                // Elevated Quick: a temporary agent service so admin windows can be shared;
                // removed on exit. Non-elevated Quick offers to relaunch elevated from the UI.
                #[cfg(windows)]
                if agent::windows::is_elevated()
                    && let Ok(exe) = std::env::current_exe()
                {
                    let _ = agent::windows::install_service(
                        service_name(),
                        "Kronos Quick Support (controllo UAC)",
                        &exe,
                        agent_product(),
                        // Removed on exit: nothing may start it again later.
                        false,
                    );
                }
                let data_dir = app.path().app_data_dir()?;
                app.manage(Service::start(app.handle().clone(), data_dir)?);
                show_main_window(app.handle());
                return Ok(());
            }
            init_logging(app.path().app_log_dir().ok());
            let data_dir = app.path().app_data_dir()?;
            let service = Service::start(app.handle().clone(), data_dir)?;
            // Admin-windows (UAC) agent is opt-out: offer to install the service once, unless
            // the user declined before or it is already installed.
            #[cfg(windows)]
            if !service.uac_agent_declined()
                && !agent::windows::is_service_installed(service_name())
            {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn_blocking(move || {
                    match agent::windows::run_self_elevated(&["--install-service"]) {
                        Ok(true) => {}
                        // Declined or failed: do not ask again (the button still enables it).
                        _ => handle.state::<Service>().set_uac_agent_declined(true),
                    }
                });
            } else if uac::service_outdated()
                && service.service_update_declined() != Some(agent::windows::SERVICE_REVISION)
            {
                // The service runs its own copy of the app from when it was installed: refresh
                // it when the service changed (e.g. the host before login, ADR 0056). Asked
                // once per service revision; the button stays.
                let handle = app.handle().clone();
                let args = uac::install_args(service.remote_only());
                tauri::async_runtime::spawn_blocking(move || {
                    if !matches!(agent::windows::run_self_elevated(&args), Ok(true)) {
                        handle
                            .state::<Service>()
                            .set_service_update_declined(agent::windows::SERVICE_REVISION);
                    }
                });
            }
            app.manage(service);
            test_connect(app.handle());
            build_tray(app)?;
            if !std::env::args().any(|a| a == HIDDEN_ARG) {
                show_main_window(app.handle());
            }
            Ok(())
        })
        // Closing the window hides it: the device must stay reachable and able to show
        // the approval popup. Quit from the tray menu (or Cmd+Q on macOS).
        // Quick access has no tray: closing the window quits and ends any sharing.
        .on_window_event(|window, event| {
            let label = window.label();
            if let Some(session) = label.strip_prefix("view-").and_then(|s| s.parse().ok()) {
                // Closing a remote screen's window ends that session, and its file manager.
                if let WindowEvent::Destroyed = event {
                    window.app_handle().state::<Service>().close_view(session);
                    if let Some(files) = window
                        .app_handle()
                        .get_webview_window(&format!("files-{session}"))
                    {
                        let _ = files.close();
                    }
                }
                return;
            }
            if let WindowEvent::CloseRequested { api, .. } = event
                && !QUICK
                && label == "main"
            {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .invoke_handler(tauri::generate_handler![
            app_mode,
            get_status,
            connect_to,
            new_password,
            respond_approval,
            cancel_connect,
            device_presence,
            lan_peers,
            set_lan_name,
            connect_device,
            remove_device,
            set_personal_password,
            list_device_tree,
            open_files_window,
            fs_local_list,
            fs_local_mkdir,
            fs_remote_list,
            fs_remote_mkdir,
            fs_copy,
            fs_cancel,
            create_group,
            rename_group,
            move_group,
            set_group_collapsed,
            delete_group,
            move_device,
            get_autostart,
            set_autostart,
            export_devices,
            devices_file_locked,
            import_devices,
            permissions::get_permissions,
            permissions::open_permission_settings,
            uac::uac_support,
            uac::set_uac_support,
            relaunch_elevated,
            updates::app_version,
            updates::check_update,
            updates::install_update,
            virtual_monitor::virtual_monitor_status,
            virtual_monitor::set_virtual_monitor,
            attach_view,
            set_clipboard,
            send_chat,
            host_chat_history,
            request_keyframe,
            frames_consumed,
            send_input,
            select_display,
            set_high_resolution,
            set_audio,
            set_audio_volume,
            cancel_transfer,
            set_block_input,
            match_resolution,
            display_name,
            set_display_name,
            remote_only,
            set_remote_only,
            host_permissions,
            set_host_permissions,
            audio_supported,
            close_view,
            end_session,
            stop_hosting
        ])
        .build(context())
        .expect("error while building tauri application");

    app.run(|app, event| {
        // macOS: clicking the Dock icon re-opens the hidden window.
        #[cfg(target_os = "macos")]
        if let tauri::RunEvent::Reopen { .. } = event {
            show_main_window(app);
        }
        // Elevated Quick removes its temporary agent service when it quits.
        #[cfg(windows)]
        if QUICK && matches!(event, tauri::RunEvent::Exit) && agent::windows::is_elevated() {
            let _ = agent::windows::uninstall_service(service_name());
        }
        let _ = (app, event);
    });
}

#[cfg(test)]
mod tests {
    #[test]
    fn app_identifier_matches_the_tauri_config() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).unwrap();
        assert_eq!(config["identifier"], super::APP_IDENTIFIER);
    }
}
