//! Windows "control admin (UAC) windows" support, exposed to the UI (ADR 0033).
//!
//! The full app installs a SYSTEM service (elevated, once) that runs the agent; the toggle
//! reflects and changes that. Quick Support handles elevation at startup instead, so it only
//! reports its state here.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct UacSupport {
    /// This platform/flavour can control admin windows at all.
    pub supported: bool,
    /// The agent service is installed (full app), or this Quick instance is elevated.
    pub active: bool,
    /// The UI should offer a toggle (full app on Windows). Quick shows only a note.
    pub toggleable: bool,
}

#[tauri::command]
pub fn uac_support() -> UacSupport {
    #[cfg(windows)]
    {
        if crate::QUICK {
            return UacSupport {
                supported: true,
                active: agent::windows::is_elevated(),
                toggleable: false,
            };
        }
        return UacSupport {
            supported: true,
            active: agent::windows::is_service_installed(crate::service_name()),
            toggleable: true,
        };
    }
    #[cfg(not(windows))]
    UacSupport {
        supported: false,
        active: false,
        toggleable: false,
    }
}

/// Installs (with a UAC prompt) or removes the agent service. Full app, Windows only.
#[tauri::command]
pub async fn set_uac_support(enable: bool) -> Result<UacSupport, String> {
    #[cfg(windows)]
    {
        if crate::QUICK {
            return Err("in Quick Support si attiva avviando come amministratore".into());
        }
        let arg = if enable {
            "--install-service"
        } else {
            "--uninstall-service"
        };
        let accepted =
            tauri::async_runtime::spawn_blocking(move || agent::windows::run_self_elevated(&[arg]))
                .await
                .map_err(|e| e.to_string())?
                .map_err(|e| format!("{e:#}"))?;
        if !accepted {
            return Err("permesso di amministratore negato".into());
        }
        Ok(uac_support())
    }
    #[cfg(not(windows))]
    {
        let _ = enable;
        Err("non disponibile su questo sistema".into())
    }
}
