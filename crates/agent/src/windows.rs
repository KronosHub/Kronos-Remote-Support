//! Windows plumbing: the named pipe between app and agent (with mutual verification), the
//! agent process, and the SYSTEM service that keeps an agent running in the active session.

use std::ffi::{OsStr, OsString, c_void};
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use anyhow::{Context, Result, bail};
use tokio::net::windows::named_pipe::{
    ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
};
use windows_sys::Win32::Foundation::{
    CloseHandle, ERROR_PIPE_BUSY, HANDLE, HLOCAL, LocalFree, WAIT_OBJECT_0,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, GetSecurityInfo, SE_KERNEL_OBJECT,
};
use windows_sys::Win32::Security::{
    DuplicateTokenEx, IsWellKnownSid, OWNER_SECURITY_INFORMATION, PSID, SECURITY_ATTRIBUTES,
    SecurityImpersonation, SetTokenInformation, TOKEN_ALL_ACCESS, TokenPrimary, TokenSessionId,
    WinBuiltinAdministratorsSid, WinLocalSystemSid,
};
use windows_sys::Win32::System::Pipes::{GetNamedPipeClientProcessId, WaitNamedPipeW};
use windows_sys::Win32::System::RemoteDesktop::{
    ProcessIdToSessionId, WTSGetActiveConsoleSessionId,
};
use windows_sys::Win32::System::Threading::{
    CREATE_NO_WINDOW, CREATE_UNICODE_ENVIRONMENT, CreateProcessAsUserW, GetCurrentProcess,
    GetCurrentProcessId, OpenProcess, OpenProcessToken, PROCESS_INFORMATION,
    PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW, STARTUPINFOW, TerminateProcess,
    WaitForSingleObject,
};

use crate::server::{Backend, System, serve};

/// Only SYSTEM and interactively logged-on users may open the pipe (protected DACL).
const PIPE_SDDL: &str = "D:P(A;;GA;;;SY)(A;;GRGW;;;IU)";

/// File name of the service's own copy of the executable inside its protected home.
const AGENT_EXE: &str = "kronos-agent.exe";

/// `C:\Program Files`, from the shell rather than `%ProgramFiles%`: an elevated process
/// inherits the user's environment, which the user can change.
fn program_files() -> Result<PathBuf> {
    use std::os::windows::ffi::OsStringExt;
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{FOLDERID_ProgramFiles, SHGetKnownFolderPath};
    let mut raw: *mut u16 = std::ptr::null_mut();
    let hr =
        unsafe { SHGetKnownFolderPath(&FOLDERID_ProgramFiles, 0, std::ptr::null_mut(), &mut raw) };
    if hr != 0 || raw.is_null() {
        bail!("SHGetKnownFolderPath failed: {hr:#x}");
    }
    let mut len = 0;
    while unsafe { *raw.add(len) } != 0 {
        len += 1;
    }
    let path = OsString::from_wide(unsafe { std::slice::from_raw_parts(raw, len) });
    unsafe { CoTaskMemFree(raw.cast()) };
    Ok(PathBuf::from(path))
}

/// Where SYSTEM keeps its own files (ADR 0039). Only administrators can write under Program
/// Files, so no user can swap the binary the service runs as SYSTEM, nor pre-create or
/// redirect (junction) the folders it writes to.
fn protected_root() -> Result<PathBuf> {
    Ok(program_files()?.join("Kronos Agent"))
}

/// The service's private copy of the executable lives here.
fn service_home(name: &str) -> Result<PathBuf> {
    Ok(protected_root()?.join(name))
}

/// Log folder of the SYSTEM service and agent, created if missing.
pub fn log_dir() -> Option<PathBuf> {
    let dir = protected_root().ok()?.join("logs");
    std::fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
    s.as_ref().encode_wide().chain(Some(0)).collect()
}

fn session_of(pid: u32) -> Option<u32> {
    let mut session = 0u32;
    (unsafe { ProcessIdToSessionId(pid, &mut session) } != 0).then_some(session)
}

pub fn current_session() -> Option<u32> {
    session_of(unsafe { GetCurrentProcessId() })
}

/// True if this process runs with a full administrator token.
pub fn is_elevated() -> bool {
    use windows_sys::Win32::Security::{GetTokenInformation, TOKEN_ELEVATION, TokenElevation};
    unsafe {
        let mut token = std::ptr::null_mut();
        if OpenProcessToken(
            GetCurrentProcess(),
            windows_sys::Win32::Security::TOKEN_QUERY,
            &mut token,
        ) == 0
        {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION { TokenIsElevated: 0 };
        let mut size = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            (&raw mut elevation).cast(),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut size,
        );
        CloseHandle(token);
        ok != 0 && elevation.TokenIsElevated != 0
    }
}

/// True if a service with this name exists.
pub fn is_service_installed(name: &str) -> bool {
    let Ok(manager) = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)
    else {
        return false;
    };
    manager
        .open_service(name, ServiceAccess::QUERY_STATUS)
        .is_ok()
}

/// Relaunches this executable elevated (UAC) with `args`, waiting for it to exit.
/// `Ok(false)` if the user declined the UAC prompt.
pub fn run_self_elevated(args: &[&str]) -> Result<bool> {
    shell_execute_self_elevated(args, true)
}

/// Relaunches this executable elevated (UAC) with `args` without waiting (for a GUI
/// hand-off). `Ok(false)` if the user declined the UAC prompt.
pub fn spawn_self_elevated(args: &[&str]) -> Result<bool> {
    shell_execute_self_elevated(args, false)
}

fn shell_execute_self_elevated(args: &[&str], wait: bool) -> Result<bool> {
    use windows_sys::Win32::Foundation::{WAIT_FAILED, WAIT_OBJECT_0};
    use windows_sys::Win32::UI::Shell::{
        SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;
    let exe = std::env::current_exe()?;
    let params = args
        .iter()
        .map(|a| format!("\"{a}\""))
        .collect::<Vec<_>>()
        .join(" ");
    let verb = wide("runas");
    let file = wide(&exe);
    let params_w = wide(params);
    unsafe {
        let mut info: SHELLEXECUTEINFOW = std::mem::zeroed();
        info.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_NOCLOSEPROCESS;
        info.lpVerb = verb.as_ptr();
        info.lpFile = file.as_ptr();
        info.lpParameters = params_w.as_ptr();
        info.nShow = SW_HIDE as i32;
        if ShellExecuteExW(&mut info) == 0 {
            // 1223 = ERROR_CANCELLED (user said no to UAC).
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(1223) {
                return Ok(false);
            }
            return Err(err.into());
        }
        if info.hProcess.is_null() {
            return Ok(true);
        }
        if wait {
            if WaitForSingleObject(info.hProcess, 120_000) == WAIT_FAILED {
                CloseHandle(info.hProcess);
                bail!("waiting for the elevated process failed");
            }
            let _ = WAIT_OBJECT_0;
        }
        CloseHandle(info.hProcess);
    }
    Ok(true)
}

/// One pipe per product and session (several users, app and Quick Support side by side).
pub fn pipe_name(product: &str, session: u32) -> String {
    format!(r"\\.\pipe\kronos-agent-{product}-{session}")
}

fn image_path(pid: u32) -> Option<PathBuf> {
    let process = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if process.is_null() {
        return None;
    }
    let mut buf = [0u16; 1024];
    let mut len = buf.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(process, 0, buf.as_mut_ptr(), &mut len) };
    unsafe { CloseHandle(process) };
    (ok != 0).then(|| PathBuf::from(String::from_utf16_lossy(&buf[..len as usize])))
}

fn same_file(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| {
        std::fs::canonicalize(p)
            .unwrap_or_else(|_| p.to_path_buf())
            .to_string_lossy()
            .to_lowercase()
    };
    norm(a) == norm(b)
}

fn is_allowed(path: &Path, allowed: &[PathBuf]) -> bool {
    allowed.iter().any(|a| same_file(path, a))
}

/// The agent only serves the app it was installed for (or its own executable), in its own
/// session.
fn client_allowed(pipe: &NamedPipeServer, allowed: &[PathBuf], session: u32) -> bool {
    let mut pid = 0u32;
    if unsafe { GetNamedPipeClientProcessId(pipe.as_raw_handle() as HANDLE, &mut pid) } == 0 {
        return false;
    }
    let path_ok = image_path(pid).is_some_and(|p| is_allowed(&p, allowed));
    let session_ok = session_of(pid) == Some(session);
    if !(path_ok && session_ok) {
        tracing::warn!(pid, path_ok, session_ok, "agent: refused a pipe client");
    }
    path_ok && session_ok
}

/// The app only talks to a pipe created by a privileged process (owner SYSTEM or
/// Administrators): a normal user cannot, so no unprivileged program can pose as the agent.
fn owned_by_system(handle: HANDLE) -> bool {
    let mut owner: PSID = std::ptr::null_mut();
    let mut descriptor = std::ptr::null_mut();
    let rc = unsafe {
        GetSecurityInfo(
            handle,
            SE_KERNEL_OBJECT,
            OWNER_SECURITY_INFORMATION,
            &mut owner,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut descriptor,
        )
    };
    if rc != 0 {
        return false;
    }
    // A non-admin process cannot create an object owned by SYSTEM or Administrators, so
    // either proves the pipe was made by a privileged process (the agent/service).
    let system = unsafe {
        IsWellKnownSid(owner, WinLocalSystemSid) != 0
            || IsWellKnownSid(owner, WinBuiltinAdministratorsSid) != 0
    };
    unsafe { LocalFree(descriptor as HLOCAL) };
    system
}

fn create_pipe(name: &str, first: bool) -> std::io::Result<NamedPipeServer> {
    let sddl = wide(PIPE_SDDL);
    let mut descriptor = std::ptr::null_mut();
    if unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            1,
            &mut descriptor,
            std::ptr::null_mut(),
        )
    } == 0
    {
        return Err(std::io::Error::last_os_error());
    }
    let mut attributes = SECURITY_ATTRIBUTES {
        nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: descriptor,
        bInheritHandle: 0,
    };
    let result = unsafe {
        ServerOptions::new()
            .first_pipe_instance(first)
            .reject_remote_clients(true)
            .create_with_security_attributes_raw(name, (&raw mut attributes).cast::<c_void>())
    };
    unsafe { LocalFree(descriptor as HLOCAL) };
    result
}

/// True if this product's agent is listening in the current session.
pub fn agent_available(product: &str) -> bool {
    let Some(session) = current_session() else {
        return false;
    };
    let name = wide(pipe_name(product, session));
    // A short wait so a just-started agent is not missed (falls back to local otherwise).
    unsafe { WaitNamedPipeW(name.as_ptr(), 300) != 0 }
}

/// Connects the app to its agent (same session), refusing impostors.
pub async fn connect(product: &str) -> std::io::Result<NamedPipeClient> {
    let session = current_session().ok_or_else(|| std::io::Error::other("no session id"))?;
    let name = pipe_name(product, session);
    let client = loop {
        match ClientOptions::new().open(&name) {
            Ok(client) => break client,
            Err(e) if e.raw_os_error() == Some(ERROR_PIPE_BUSY as i32) => {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Err(e) => return Err(e),
        }
    };
    if !owned_by_system(client.as_raw_handle() as HANDLE) {
        return Err(std::io::Error::other(
            "the agent pipe was not created by a privileged process",
        ));
    }
    Ok(client)
}

/// Agent process (`--agent <product> [app]`): serve the connections of `app` (the app
/// executable the service was installed for) in this session until killed.
pub fn run_agent(product: &str, app: Option<&Path>) -> Result<()> {
    windesk::enable();
    // Pixel coordinates must match the real screen whatever the scaling.
    unsafe {
        windows_sys::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
            windows_sys::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        );
    }
    let session = current_session().context("no session id")?;
    let mut allowed = vec![std::env::current_exe()?];
    allowed.extend(app.map(Path::to_path_buf));
    let name = pipe_name(product, session);
    tracing::info!(session, %name, "agent starting");
    let backend: Arc<dyn Backend> = Arc::new(System);
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(async move {
        let mut server = create_pipe(&name, true).context("creating the agent pipe")?;
        loop {
            server.connect().await?;
            let connected = server;
            server = create_pipe(&name, false)?;
            if client_allowed(&connected, &allowed, session) {
                tokio::spawn(serve(connected, backend.clone()));
            }
        }
    })
}

/// Starts `exe --agent <product> <app>` as SYSTEM in `session` (what the service does).
pub fn launch_agent(session: u32, exe: &Path, product: &str, app: &Path) -> Result<HANDLE> {
    unsafe {
        let mut own = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_ALL_ACCESS, &mut own) == 0 {
            bail!("OpenProcessToken: {}", std::io::Error::last_os_error());
        }
        let mut token = std::ptr::null_mut();
        let ok = DuplicateTokenEx(
            own,
            TOKEN_ALL_ACCESS,
            std::ptr::null(),
            SecurityImpersonation,
            TokenPrimary,
            &mut token,
        );
        CloseHandle(own);
        if ok == 0 {
            bail!("DuplicateTokenEx: {}", std::io::Error::last_os_error());
        }
        let session_value = session;
        if SetTokenInformation(token, TokenSessionId, (&raw const session_value).cast(), 4) == 0 {
            CloseHandle(token);
            bail!("SetTokenInformation: {}", std::io::Error::last_os_error());
        }
        let application = wide(exe);
        let mut command = wide(format!(
            "\"{}\" --agent {product} \"{}\"",
            exe.display(),
            app.display()
        ));
        let mut desktop = wide(r"winsta0\default");
        let mut startup: STARTUPINFOW = std::mem::zeroed();
        startup.cb = size_of::<STARTUPINFOW>() as u32;
        startup.lpDesktop = desktop.as_mut_ptr();
        let mut info: PROCESS_INFORMATION = std::mem::zeroed();
        let ok = CreateProcessAsUserW(
            token,
            application.as_ptr(),
            command.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            CREATE_NO_WINDOW | CREATE_UNICODE_ENVIRONMENT,
            std::ptr::null(),
            std::ptr::null(),
            &startup,
            &mut info,
        );
        CloseHandle(token);
        if ok == 0 {
            bail!("CreateProcessAsUserW: {}", std::io::Error::last_os_error());
        }
        CloseHandle(info.hThread);
        Ok(info.hProcess)
    }
}

// ---- Service --------------------------------------------------------------------------

use windows_service::service::{
    ServiceAccess, ServiceControl, ServiceControlAccept, ServiceErrorControl, ServiceExitCode,
    ServiceInfo, ServiceStartType, ServiceState, ServiceStatus, ServiceType,
};
use windows_service::service_control_handler::{self, ServiceControlHandlerResult};
use windows_service::service_manager::{ServiceManager, ServiceManagerAccess};
use windows_service::{define_windows_service, service_dispatcher};

/// (service name, product, app executable) for the running service.
static SERVICE: OnceLock<(String, String, Option<PathBuf>)> = OnceLock::new();

define_windows_service!(ffi_service_main, service_main);

/// Service process (`--service <name> <product> [app]`), started by the Service Control
/// Manager. `app` is the app executable whose connections the agent accepts.
pub fn run_service(name: &str, product: &str, app: Option<&Path>) -> Result<()> {
    let _ = SERVICE.set((
        name.to_owned(),
        product.to_owned(),
        app.map(Path::to_path_buf),
    ));
    service_dispatcher::start(name, ffi_service_main).context("starting the service dispatcher")
}

fn service_main(_arguments: Vec<OsString>) {
    if let Err(e) = service_loop() {
        tracing::error!("service failed: {e:#}");
    }
}

fn path_key(path: &Path) -> String {
    path.to_string_lossy().to_lowercase()
}

/// True if `exe` already is the service's protected copy.
fn is_protected_copy(name: &str, exe: &Path) -> bool {
    let Ok(home) = service_home(name) else {
        return false;
    };
    path_key(exe).starts_with(&path_key(&home))
}

/// Copies `exe` into the service's protected home, retrying while an exiting service or
/// agent still holds the previous copy. Returns the copy's path.
fn install_copy(name: &str, exe: &Path) -> Result<PathBuf> {
    let home = service_home(name)?;
    std::fs::create_dir_all(&home).with_context(|| format!("creating {}", home.display()))?;
    let target = home.join(AGENT_EXE);
    let mut attempts = 0;
    loop {
        match std::fs::copy(exe, &target) {
            Ok(_) => return Ok(target),
            Err(e) if attempts < 40 => {
                attempts += 1;
                tracing::debug!("copy to {} failed, retrying: {e}", target.display());
                std::thread::sleep(Duration::from_millis(250));
            }
            Err(e) => {
                return Err(e).with_context(|| format!("copying to {}", target.display()));
            }
        }
    }
}

fn service_info(
    name: &str,
    display_name: OsString,
    start_type: ServiceStartType,
    exe: &Path,
    product: &str,
    app: &Path,
) -> ServiceInfo {
    ServiceInfo {
        name: name.into(),
        display_name,
        service_type: ServiceType::OWN_PROCESS,
        start_type,
        error_control: ServiceErrorControl::Normal,
        executable_path: exe.to_path_buf(),
        launch_arguments: vec![
            "--service".into(),
            name.into(),
            product.into(),
            app.as_os_str().to_owned(),
        ],
        dependencies: vec![],
        account_name: None,
        account_password: None,
    }
}

/// Services installed before 1.2.0 run the app executable itself, which its user can
/// replace. Move this one to a protected copy: from the next start SYSTEM runs that copy.
fn migrate_to_protected_copy(name: &str, product: &str, exe: &Path) -> Result<PathBuf> {
    let copy = install_copy(name, exe)?;
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;
    let service = manager.open_service(
        name,
        ServiceAccess::QUERY_CONFIG | ServiceAccess::CHANGE_CONFIG,
    )?;
    let config = service.query_config()?;
    let info = service_info(
        name,
        config.display_name,
        config.start_type,
        &copy,
        product,
        exe,
    );
    service.change_config(&info)?;
    tracing::info!(copy = %copy.display(), "service moved to its protected copy");
    Ok(copy)
}

fn service_loop() -> Result<()> {
    let (name, product, app) = SERVICE.get().cloned().context("service not configured")?;
    let (stop_tx, stop_rx) = std::sync::mpsc::channel::<()>();
    let handler = move |control| match control {
        ServiceControl::Stop | ServiceControl::Shutdown => {
            let _ = stop_tx.send(());
            ServiceControlHandlerResult::NoError
        }
        ServiceControl::Interrogate | ServiceControl::SessionChange(_) => {
            ServiceControlHandlerResult::NoError
        }
        _ => ServiceControlHandlerResult::NotImplemented,
    };
    let status = service_control_handler::register(&name, handler)?;
    let set = |state, accept| {
        status.set_service_status(ServiceStatus {
            service_type: ServiceType::OWN_PROCESS,
            current_state: state,
            controls_accepted: accept,
            exit_code: ServiceExitCode::Win32(0),
            checkpoint: 0,
            wait_hint: Duration::from_secs(5),
            process_id: None,
        })
    };
    set(
        ServiceState::Running,
        ServiceControlAccept::STOP
            | ServiceControlAccept::SHUTDOWN
            | ServiceControlAccept::SESSION_CHANGE,
    )?;

    let mut exe = std::env::current_exe()?;
    // Without an explicit app (pre-1.2.0 install) the service runs the app itself.
    let app = app.unwrap_or_else(|| exe.clone());
    if !is_protected_copy(&name, &exe) {
        match migrate_to_protected_copy(&name, &product, &exe) {
            Ok(copy) => exe = copy,
            Err(e) => tracing::warn!("cannot move the service to a protected copy: {e:#}"),
        }
    }
    // The agent runs in the session at the physical console; restarted if it exits or the
    // console moves to another session (fast user switching).
    let mut agent: Option<(u32, HANDLE)> = None;
    loop {
        let session = unsafe { WTSGetActiveConsoleSessionId() };
        let exited = agent
            .as_ref()
            .is_some_and(|(_, h)| unsafe { WaitForSingleObject(*h, 0) } == WAIT_OBJECT_0);
        let moved = agent.as_ref().is_some_and(|(s, _)| *s != session);
        if exited || moved {
            if let Some((_, h)) = agent.take() {
                unsafe {
                    TerminateProcess(h, 0);
                    CloseHandle(h);
                }
            }
        }
        if agent.is_none() && session != u32::MAX {
            match launch_agent(session, &exe, &product, &app) {
                Ok(h) => agent = Some((session, h)),
                Err(e) => tracing::warn!(session, "cannot start the agent: {e:#}"),
            }
        }
        if stop_rx.recv_timeout(Duration::from_secs(2)).is_ok() {
            break;
        }
    }
    if let Some((_, h)) = agent.take() {
        unsafe {
            TerminateProcess(h, 0);
            CloseHandle(h);
        }
    }
    set(ServiceState::Stopped, ServiceControlAccept::empty())?;
    Ok(())
}

/// Installs (replacing any previous one) and starts the service as LocalSystem. SYSTEM runs
/// a copy of `exe` under Program Files (ADR 0039); its agent serves `exe` (the app).
/// `auto_start` keeps it across reboots. Needs administrator rights.
pub fn install_service(
    name: &str,
    display_name: &str,
    exe: &Path,
    product: &str,
    auto_start: bool,
) -> Result<()> {
    let _ = uninstall_service(name);
    let copy = install_copy(name, exe)?;
    let manager = ServiceManager::local_computer(
        None::<&str>,
        ServiceManagerAccess::CONNECT | ServiceManagerAccess::CREATE_SERVICE,
    )?;
    let start_type = if auto_start {
        ServiceStartType::AutoStart
    } else {
        ServiceStartType::OnDemand
    };
    let info = service_info(name, display_name.into(), start_type, &copy, product, exe);
    let service =
        manager.create_service(&info, ServiceAccess::START | ServiceAccess::CHANGE_CONFIG)?;
    let _ = service.set_description(
        "Kronos Remote Support: permette all'assistenza remota di vedere e usare anche le finestre di amministratore (UAC) e la schermata di blocco.",
    );
    service.start(&[] as &[&OsStr])?;
    Ok(())
}

/// Stops and deletes the service, waiting until it is gone.
pub fn uninstall_service(name: &str) -> Result<()> {
    let manager = ServiceManager::local_computer(None::<&str>, ServiceManagerAccess::CONNECT)?;
    let service = manager.open_service(
        name,
        ServiceAccess::STOP | ServiceAccess::DELETE | ServiceAccess::QUERY_STATUS,
    )?;
    let _ = service.stop();
    for _ in 0..50 {
        match service.query_status() {
            Ok(s) if s.current_state != ServiceState::Stopped => {
                std::thread::sleep(Duration::from_millis(100))
            }
            _ => break,
        }
    }
    service.delete()?;
    drop(service);
    for _ in 0..50 {
        if manager
            .open_service(name, ServiceAccess::QUERY_STATUS)
            .is_err()
        {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    // The copy may still be in use for a moment by the exiting service or agent.
    if let Ok(home) = service_home(name) {
        for _ in 0..40 {
            if !home.exists() || std::fs::remove_dir_all(&home).is_ok() {
                break;
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }
    Ok(())
}
