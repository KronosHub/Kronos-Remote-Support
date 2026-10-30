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
    /// The installed service is older than this app's: "Aggiorna" refreshes it.
    pub outdated: bool,
}

/// Full app on Windows: the service is installed but from an older revision.
#[cfg(windows)]
pub fn service_outdated() -> bool {
    !crate::QUICK
        && agent::windows::is_service_installed(crate::service_name())
        && agent::windows::installed_revision(crate::service_name())
            < agent::windows::SERVICE_REVISION
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
                outdated: false,
            };
        }
        UacSupport {
            supported: true,
            active: agent::windows::is_service_installed(crate::service_name()),
            toggleable: true,
            outdated: service_outdated(),
        }
    }
    #[cfg(not(windows))]
    UacSupport {
        supported: false,
        active: false,
        toggleable: false,
        outdated: false,
    }
}

/// Arguments of the elevated service install: a remote-only computer stays one.
#[cfg(windows)]
pub fn install_args(remote_only: bool) -> Vec<&'static str> {
    let mut args = vec!["--install-service"];
    if remote_only {
        args.push("--remote-only");
    }
    args
}

/// Installs (with a UAC prompt) or removes the agent service. Full app, Windows only.
#[tauri::command]
pub async fn set_uac_support(
    service: tauri::State<'_, crate::service::Service>,
    enable: bool,
) -> Result<UacSupport, String> {
    #[cfg(windows)]
    {
        if crate::QUICK {
            return Err("in Quick Support si attiva avviando come amministratore".into());
        }
        let args = if enable {
            install_args(service.remote_only())
        } else {
            vec!["--uninstall-service"]
        };
        let accepted =
            tauri::async_runtime::spawn_blocking(move || agent::windows::run_self_elevated(&args))
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
        let _ = (service, enable);
        Err("non disponibile su questo sistema".into())
    }
}
