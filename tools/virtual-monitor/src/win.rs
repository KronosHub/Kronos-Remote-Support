//! Installs and removes the virtual monitor (Virtual Display Driver) with SetupAPI.

use std::io::Write;
use std::path::{Path, PathBuf};

use windows::Win32::Devices::DeviceAndDriverInstallation::{
    DICD_GENERATE_ID, DIF_REGISTERDEVICE, DIF_REMOVE, DIGCF_PROFILE, HDEVINFO, INSTALLFLAG_FORCE,
    SP_COPY_REPLACEONLY, SP_DEVINFO_DATA, SPDRP_HARDWAREID, SPOST_NONE, SUOI_FORCEDELETE,
    SetupCopyOEMInfW, SetupDiCallClassInstaller, SetupDiCreateDeviceInfoList,
    SetupDiCreateDeviceInfoW, SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInfo,
    SetupDiGetClassDevsW, SetupDiGetDeviceRegistryPropertyW, SetupDiSetDeviceRegistryPropertyW,
    SetupUninstallOEMInfW, UpdateDriverForPlugAndPlayDevicesW,
};
use windows::Win32::Foundation::ERROR_NO_MORE_ITEMS;
use windows::core::{BOOL, GUID, HSTRING, PCWSTR};

/// Hardware id the driver package installs on (see MttVDD.inf).
const HARDWARE_ID: &str = r"Root\MttVDD";
/// Display adapters (GUID_DEVCLASS_DISPLAY).
const DISPLAY_CLASS: GUID = GUID::from_u128(0x4d36e968_e325_11ce_bfc1_08002be10318);
/// Where the driver reads its settings (Driver.cpp, `confpath`).
const SETTINGS_DIR: &str = r"C:\VirtualDisplayDriver";

/// Driver package files (Virtual Display Driver, x64; version in driver/VERSION), fetched by
/// fetch-driver.sh.
const DRIVER_FILES: [(&str, &[u8]); 3] = [
    ("MttVDD.inf", include_bytes!("../driver/MttVDD.inf")),
    ("MttVDD.dll", include_bytes!("../driver/MttVDD.dll")),
    ("mttvdd.cat", include_bytes!("../driver/mttvdd.cat")),
];
/// The settings shipped with the driver, reduced below to one 1920×1080 monitor at 60 Hz.
const DEFAULT_SETTINGS: &str = include_str!("../driver/vdd_settings.xml");

/// Version of what this program installs: the program itself and the driver it bundles.
/// Written next to the settings on install, so the app can offer an update.
fn bundled_version() -> String {
    format!(
        "{}+vdd{}",
        env!("CARGO_PKG_VERSION"),
        include_str!("../driver/VERSION").trim()
    )
}

/// The installed version, from the marker written on install.
fn installed_version() -> Option<String> {
    std::fs::read_to_string(Path::new(SETTINGS_DIR).join(MARKER))
        .ok()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
}

const MARKER: &str = "kronos-virtual-monitor.txt";

/// Commands (the app uses them without a console):
/// - `status`: `installed=true|false`, `version=` (installed), `bundled=` (this program);
/// - `install` / `uninstall` [`--result <file>`]: asks for administrator rights itself
///   (UAC); the outcome goes to the exit code (0 ok, 1 error, 2 restart needed) and, with
///   `--result`, as text to that file;
/// - no argument: a menu, for a double-click.
pub fn run() -> i32 {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().map(|a| a.to_lowercase());
    let result_file = args
        .iter()
        .position(|a| a == "--result")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from);
    match command.as_deref() {
        Some("status") => {
            println!("installed={}", installed().unwrap_or(false));
            println!("version={}", installed_version().unwrap_or_default());
            println!("bundled={}", bundled_version());
            0
        }
        Some(command @ ("install" | "uninstall")) => {
            if !is_elevated() {
                return match run_elevated(&args, false) {
                    Ok(Some(code)) => {
                        if let Some(file) = &result_file
                            && let Ok(text) = std::fs::read_to_string(file)
                        {
                            println!("{text}");
                        }
                        code as i32
                    }
                    Ok(None) => {
                        report(&result_file, "permesso di amministratore negato");
                        eprintln!("Permesso di amministratore negato.");
                        1
                    }
                    Err(e) => {
                        report(&result_file, &e);
                        eprintln!("Errore: {e}");
                        1
                    }
                };
            }
            let outcome = if command == "install" {
                install()
            } else {
                uninstall().map(|m| (m, false))
            };
            match outcome {
                Ok((message, reboot)) => {
                    report(&result_file, &message);
                    println!("{message}");
                    if reboot { 2 } else { 0 }
                }
                Err(e) => {
                    let message = format!("Errore: {e}");
                    report(&result_file, &message);
                    eprintln!("{message}");
                    1
                }
            }
        }
        Some(other) => {
            eprintln!("comando sconosciuto: {other} (usa install, uninstall o status)");
            1
        }
        None if !is_elevated() => {
            // A double-click: the menu runs again as administrator, in its own console.
            match run_elevated(&[], true) {
                Ok(_) => 0,
                Err(e) => {
                    eprintln!("Errore: {e}");
                    1
                }
            }
        }
        None => menu(),
    }
}

fn report(file: &Option<PathBuf>, text: &str) {
    if let Some(file) = file {
        let _ = std::fs::write(file, text);
    }
}

/// True when this process runs with administrator rights.
fn is_elevated() -> bool {
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::Security::{
        GetTokenInformation, TOKEN_ELEVATION, TOKEN_QUERY, TokenElevation,
    };
    use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    // SAFETY: the token is opened, queried into a local struct and closed.
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token).is_err() {
            return false;
        }
        let mut elevation = TOKEN_ELEVATION::default();
        let mut size = 0u32;
        let ok = GetTokenInformation(
            token,
            TokenElevation,
            Some((&raw mut elevation).cast()),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut size,
        )
        .is_ok();
        let _ = CloseHandle(token);
        ok && elevation.TokenIsElevated != 0
    }
}

/// Runs this program again as administrator (UAC prompt) with `args`. With `visible`, in
/// its own console without waiting; otherwise hidden, waiting for its exit code.
/// `Ok(None)` when the user declines.
fn run_elevated(args: &[String], visible: bool) -> Result<Option<u32>, String> {
    use windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows::Win32::System::Threading::{GetExitCodeProcess, INFINITE, WaitForSingleObject};
    use windows::Win32::UI::Shell::{SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW};
    use windows::Win32::UI::WindowsAndMessaging::{SW_HIDE, SW_SHOWNORMAL};
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let params = args
        .iter()
        .map(|a| format!("\"{a}\""))
        .collect::<Vec<_>>()
        .join(" ");
    let (verb, file, params) = (
        HSTRING::from("runas"),
        HSTRING::from(exe.as_os_str()),
        HSTRING::from(params),
    );
    let mut info = SHELLEXECUTEINFOW {
        cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
        fMask: SEE_MASK_NOCLOSEPROCESS,
        lpVerb: PCWSTR(verb.as_ptr()),
        lpFile: PCWSTR(file.as_ptr()),
        lpParameters: PCWSTR(params.as_ptr()),
        nShow: if visible { SW_SHOWNORMAL.0 } else { SW_HIDE.0 },
        ..Default::default()
    };
    // SAFETY: the strings outlive the call; the process handle is closed below.
    unsafe {
        if let Err(e) = ShellExecuteExW(&mut info) {
            // ERROR_CANCELLED: the user said no to the UAC prompt.
            if e.code() == windows::Win32::Foundation::ERROR_CANCELLED.to_hresult() {
                return Ok(None);
            }
            return Err(format!("avvio come amministratore: {e}"));
        }
        if info.hProcess.is_invalid() {
            return Ok(Some(0));
        }
        if visible {
            let _ = CloseHandle(info.hProcess);
            return Ok(Some(0));
        }
        let mut code = 1u32;
        if WaitForSingleObject(info.hProcess, INFINITE) == WAIT_OBJECT_0 {
            let _ = GetExitCodeProcess(info.hProcess, &mut code);
        }
        let _ = CloseHandle(info.hProcess);
        Ok(Some(code))
    }
}

fn menu() -> i32 {
    println!("Kronos Virtual Monitor {}", bundled_version());
    println!("Monitor virtuale 1920×1080 per usare Kronos Remote Support su un PC senza monitor.");
    println!("Driver: Virtual Display Driver (licenza MIT, firmato da SignPath Foundation).");
    println!();
    let installed = installed().unwrap_or(false);
    println!(
        "Stato: {}",
        if installed {
            "monitor virtuale installato"
        } else {
            "monitor virtuale non installato"
        }
    );
    println!();
    println!(
        "  1  Installa il monitor virtuale{}",
        if installed { " (di nuovo)" } else { "" }
    );
    println!("  2  Rimuovi il monitor virtuale");
    println!("  0  Esci");
    print!("\nScelta: ");
    let _ = std::io::stdout().flush();
    let mut choice = String::new();
    let _ = std::io::stdin().read_line(&mut choice);
    let result = match choice.trim() {
        "1" => install().map(|(message, _)| message),
        "2" => uninstall(),
        _ => return 0,
    };
    let code = match result {
        Ok(message) => {
            println!("\n{message}");
            0
        }
        Err(e) => {
            println!("\nErrore: {e}");
            1
        }
    };
    print!("\nPremi Invio per chiudere.");
    let _ = std::io::stdout().flush();
    let _ = std::io::stdin().read_line(&mut String::new());
    code
}

/// Returns the message and whether Windows wants a restart.
fn install() -> Result<(String, bool), String> {
    write_settings()?;
    // A fresh device each time: the settings are read when it starts.
    remove_devices()?;
    let dir = extract_driver()?;
    let inf = dir.join("MttVDD.inf");
    let result = create_device().and_then(|()| install_driver(&inf));
    let _ = std::fs::remove_dir_all(&dir);
    let reboot = result?;
    let mut message = "Monitor virtuale 1920×1080 installato. Kronos Remote Support ora può condividere lo schermo di questo PC.".to_owned();
    if reboot {
        message
            .push_str("\nWindows chiede di riavviare il computer per completare l'installazione.");
    }
    let _ = std::fs::write(Path::new(SETTINGS_DIR).join(MARKER), bundled_version());
    Ok((message, reboot))
}

fn uninstall() -> Result<String, String> {
    let removed = remove_devices()?;
    let dir = extract_driver()?;
    let package = remove_driver_package(&dir.join("MttVDD.inf"));
    let _ = std::fs::remove_dir_all(&dir);
    package?;
    let _ = std::fs::remove_dir_all(SETTINGS_DIR);
    Ok(if removed > 0 {
        "Monitor virtuale rimosso.".to_owned()
    } else {
        "Nessun monitor virtuale da rimuovere: ripulito il driver.".to_owned()
    })
}

/// One monitor, 1920×1080 at 60 Hz only: Windows has nothing else to pick.
fn settings_xml() -> Result<String, String> {
    let replace = |xml: &str, open: &str, close: &str, with: &str| -> Result<String, String> {
        let start = xml
            .find(open)
            .ok_or_else(|| format!("impostazioni del driver senza {open}"))?;
        let end = xml[start..]
            .find(close)
            .map(|i| start + i + close.len())
            .ok_or_else(|| format!("impostazioni del driver senza {close}"))?;
        Ok(format!("{}{with}{}", &xml[..start], &xml[end..]))
    };
    let xml = replace(
        DEFAULT_SETTINGS,
        "<monitors>",
        "</monitors>",
        "<monitors>\n        <count>1</count>\n    </monitors>",
    )?;
    let xml = replace(
        &xml,
        "<global>",
        "</global>",
        "<global>\n\t\t<g_refresh_rate>60</g_refresh_rate>\n\t</global>",
    )?;
    replace(
        &xml,
        "<resolutions>",
        "</resolutions>",
        "<resolutions>\n        <resolution>\n            <width>1920</width>\n            <height>1080</height>\n            <refresh_rate>60</refresh_rate>\n        </resolution>\n    </resolutions>",
    )
}

fn write_settings() -> Result<(), String> {
    let xml = settings_xml()?;
    std::fs::create_dir_all(SETTINGS_DIR)
        .and_then(|()| std::fs::write(Path::new(SETTINGS_DIR).join("vdd_settings.xml"), xml))
        .map_err(|e| format!("impossibile scrivere le impostazioni in {SETTINGS_DIR}: {e}"))
}

fn extract_driver() -> Result<PathBuf, String> {
    let dir = std::env::temp_dir().join(format!("kronos-virtual-monitor-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    for (name, bytes) in DRIVER_FILES {
        std::fs::write(dir.join(name), bytes).map_err(|e| format!("{name}: {e}"))?;
    }
    Ok(dir)
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Registers a root-enumerated display device with our hardware id.
fn create_device() -> Result<(), String> {
    // SAFETY: SetupAPI calls on a device list owned here and destroyed before returning.
    unsafe {
        let set = SetupDiCreateDeviceInfoList(Some(&DISPLAY_CLASS), None)
            .map_err(|e| format!("elenco dispositivi: {e}"))?;
        let result = (|| {
            let mut info = SP_DEVINFO_DATA {
                cbSize: size_of::<SP_DEVINFO_DATA>() as u32,
                ..Default::default()
            };
            SetupDiCreateDeviceInfoW(
                set,
                &HSTRING::from("Display"),
                &DISPLAY_CLASS,
                PCWSTR::null(),
                None,
                DICD_GENERATE_ID,
                Some(&mut info),
            )
            .map_err(|e| format!("creazione del dispositivo: {e}"))?;
            // REG_MULTI_SZ: the id, then an empty string.
            let mut ids = wide(HARDWARE_ID);
            ids.push(0);
            let bytes = std::slice::from_raw_parts(ids.as_ptr().cast::<u8>(), ids.len() * 2);
            SetupDiSetDeviceRegistryPropertyW(set, &mut info, SPDRP_HARDWAREID, Some(bytes))
                .map_err(|e| format!("id del dispositivo: {e}"))?;
            SetupDiCallClassInstaller(DIF_REGISTERDEVICE, set, Some(&info))
                .map_err(|e| format!("registrazione del dispositivo: {e}"))
        })();
        let _ = SetupDiDestroyDeviceInfoList(set);
        result
    }
}

/// Installs the driver on the device; true if Windows wants a restart.
fn install_driver(inf: &Path) -> Result<bool, String> {
    let mut reboot = BOOL(0);
    // SAFETY: valid NUL-terminated strings and an out flag that outlives the call.
    unsafe {
        UpdateDriverForPlugAndPlayDevicesW(
            None,
            &HSTRING::from(HARDWARE_ID),
            &HSTRING::from(inf.as_os_str()),
            INSTALLFLAG_FORCE,
            Some(&mut reboot),
        )
    }
    .map_err(|e| format!("installazione del driver: {e}"))?;
    Ok(reboot.as_bool())
}

/// Devices (present or not) carrying our hardware id, removed; returns how many.
fn remove_devices() -> Result<usize, String> {
    let mut removed = 0;
    for_each_device(|set, info| {
        // SAFETY: `info` comes from the enumeration of `set`.
        if unsafe { SetupDiCallClassInstaller(DIF_REMOVE, set, Some(info)) }.is_ok() {
            removed += 1;
        }
    })?;
    Ok(removed)
}

fn installed() -> Result<bool, String> {
    let mut found = false;
    for_each_device(|_, _| found = true)?;
    Ok(found)
}

/// Calls `f` for each display device whose hardware ids include ours.
fn for_each_device(mut f: impl FnMut(HDEVINFO, &SP_DEVINFO_DATA)) -> Result<(), String> {
    // SAFETY: SetupAPI enumeration over a list owned here and destroyed before returning.
    unsafe {
        let set = SetupDiGetClassDevsW(Some(&DISPLAY_CLASS), PCWSTR::null(), None, DIGCF_PROFILE)
            .map_err(|e| format!("elenco schede video: {e}"))?;
        let mut index = 0;
        loop {
            let mut info = SP_DEVINFO_DATA {
                cbSize: size_of::<SP_DEVINFO_DATA>() as u32,
                ..Default::default()
            };
            if let Err(e) = SetupDiEnumDeviceInfo(set, index, &mut info) {
                if e.code() != ERROR_NO_MORE_ITEMS.to_hresult() {
                    let _ = SetupDiDestroyDeviceInfoList(set);
                    return Err(format!("elenco schede video: {e}"));
                }
                break;
            }
            index += 1;
            let mut buffer = [0u8; 1024];
            if SetupDiGetDeviceRegistryPropertyW(
                set,
                &info,
                SPDRP_HARDWAREID,
                None,
                Some(&mut buffer),
                None,
            )
            .is_err()
            {
                continue;
            }
            let units: Vec<u16> = buffer
                .as_chunks::<2>()
                .0
                .iter()
                .map(|&c| u16::from_le_bytes(c))
                .collect();
            let ours = units
                .split(|&u| u == 0)
                .map(String::from_utf16_lossy)
                .any(|id| id.eq_ignore_ascii_case(HARDWARE_ID));
            if ours {
                f(set, &info);
            }
        }
        let _ = SetupDiDestroyDeviceInfoList(set);
        Ok(())
    }
}

/// Removes the package from the driver store (it is found by its original .inf).
fn remove_driver_package(inf: &Path) -> Result<(), String> {
    let mut name = [0u16; 260];
    let mut file_part = windows::core::PWSTR::null();
    // SAFETY: REPLACEONLY copies nothing; it reports the oemNN.inf already in the store.
    let found = unsafe {
        SetupCopyOEMInfW(
            &HSTRING::from(inf.as_os_str()),
            PCWSTR::null(),
            SPOST_NONE,
            SP_COPY_REPLACEONLY,
            Some(&mut name),
            None,
            Some(&mut file_part),
        )
    };
    if found.is_err() || file_part.is_null() {
        // Not in the store: nothing to remove.
        return Ok(());
    }
    // SAFETY: `file_part` points into `name`, NUL-terminated.
    unsafe {
        SetupUninstallOEMInfW(PCWSTR(file_part.0), SUOI_FORCEDELETE, None)
            .ok()
            .map_err(|e| format!("rimozione del driver: {e}"))
    }
}
