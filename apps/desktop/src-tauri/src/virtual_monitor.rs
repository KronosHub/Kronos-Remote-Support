//! Windows: the virtual monitor for PCs without one (ADR 0049), from the "Aggiornamenti"
//! section. The installer ships `KronosVirtualMonitor.exe` (tools/virtual-monitor), so an
//! app update also brings its newer version; the app runs it without a console and the
//! program asks for administrator rights itself.

use serde::Serialize;

#[derive(Debug, Clone, Default, Serialize)]
pub struct VirtualMonitor {
    /// The program is available (Windows, full app installed with it).
    pub supported: bool,
    pub installed: bool,
    /// Installed version, if known.
    pub version: Option<String>,
    /// Version the app brings: offered as an update when it differs from `version`.
    pub bundled: Option<String>,
    /// Outcome of the last install/remove, for the user.
    pub message: Option<String>,
}

#[tauri::command]
pub async fn virtual_monitor_status(app: tauri::AppHandle) -> VirtualMonitor {
    #[cfg(windows)]
    {
        tauri::async_runtime::spawn_blocking(move || imp::status(&app))
            .await
            .unwrap_or_default()
    }
    #[cfg(not(windows))]
    {
        let _ = app;
        VirtualMonitor::default()
    }
}

/// Installs (or updates) the virtual monitor, or removes it; Windows shows the UAC prompt.
#[tauri::command]
pub async fn set_virtual_monitor(
    app: tauri::AppHandle,
    install: bool,
) -> Result<VirtualMonitor, String> {
    #[cfg(windows)]
    {
        tauri::async_runtime::spawn_blocking(move || imp::set(&app, install))
            .await
            .map_err(|e| e.to_string())?
    }
    #[cfg(not(windows))]
    {
        let _ = (app, install);
        Err("disponibile solo su Windows".into())
    }
}

#[cfg(windows)]
mod imp {
    use std::os::windows::process::CommandExt;
    use std::path::PathBuf;
    use std::process::Command;

    use tauri::Manager;

    use super::VirtualMonitor;

    /// No console window for the program.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    fn program(app: &tauri::AppHandle) -> Option<PathBuf> {
        if crate::QUICK {
            return None;
        }
        let path = app
            .path()
            .resource_dir()
            .ok()?
            .join("virtual-monitor")
            .join("KronosVirtualMonitor.exe");
        path.is_file().then_some(path)
    }

    pub fn status(app: &tauri::AppHandle) -> VirtualMonitor {
        let Some(exe) = program(app) else {
            return VirtualMonitor::default();
        };
        let output = match Command::new(&exe)
            .arg("status")
            .creation_flags(CREATE_NO_WINDOW)
            .output()
        {
            Ok(output) => output,
            Err(e) => {
                tracing::warn!("virtual monitor status: {e}");
                return VirtualMonitor::default();
            }
        };
        let text = String::from_utf8_lossy(&output.stdout);
        let field = |key: &str| {
            text.lines()
                .find_map(|l| l.strip_prefix(key)?.strip_prefix('='))
                .map(|v| v.trim().to_owned())
                .filter(|v| !v.is_empty())
        };
        VirtualMonitor {
            supported: true,
            installed: field("installed").as_deref() == Some("true"),
            version: field("version"),
            bundled: field("bundled"),
            message: None,
        }
    }

    pub fn set(app: &tauri::AppHandle, install: bool) -> Result<VirtualMonitor, String> {
        let exe = program(app).ok_or("monitor virtuale non disponibile in questa installazione")?;
        let result =
            std::env::temp_dir().join(format!("kronos-virtual-monitor-{}.txt", std::process::id()));
        let _ = std::fs::remove_file(&result);
        let exit = Command::new(&exe)
            .arg(if install { "install" } else { "uninstall" })
            .arg("--result")
            .arg(&result)
            .creation_flags(CREATE_NO_WINDOW)
            .status()
            .map_err(|e| format!("avvio del programma del monitor virtuale: {e}"))?;
        let text = std::fs::read_to_string(&result)
            .map(|t| t.trim().to_owned())
            .unwrap_or_default();
        let _ = std::fs::remove_file(&result);
        tracing::info!(install, code = ?exit.code(), "virtual monitor: {text}");
        match exit.code() {
            Some(0 | 2) => Ok(VirtualMonitor {
                message: Some(text).filter(|t| !t.is_empty()),
                ..status(app)
            }),
            _ if text.is_empty() => Err("operazione non riuscita".into()),
            _ => Err(text.trim_start_matches("Errore: ").to_owned()),
        }
    }
}
