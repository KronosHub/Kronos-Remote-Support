//! In-app updates (Tauri updater): signed packages published on the public
//! `Remote-Desktop-releases` repository (ADR 0023).

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::UpdaterExt;

#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    pub version: String,
    pub current: String,
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
struct Progress {
    downloaded: u64,
    total: Option<u64>,
}

fn err(e: impl std::fmt::Display) -> String {
    format!("aggiornamento non riuscito: {e}")
}

#[tauri::command]
pub fn app_version(app: AppHandle) -> String {
    app.package_info().version.to_string()
}

/// Asks the update server whether a newer version exists.
#[tauri::command]
pub async fn check_update(app: AppHandle) -> Result<Option<UpdateInfo>, String> {
    if crate::QUICK {
        // Quick access is downloaded fresh each time: nothing to update.
        return Ok(None);
    }
    let update = app.updater().map_err(err)?.check().await.map_err(err)?;
    Ok(update.map(|u| UpdateInfo {
        version: u.version.clone(),
        current: u.current_version.clone(),
        notes: u.body.clone(),
    }))
}

/// Downloads, verifies (signature) and installs the update, then restarts the app.
/// Progress is reported through the `update_progress` event.
#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    if crate::QUICK {
        return Err(err("non disponibile in Quick access"));
    }
    let update = app
        .updater()
        .map_err(err)?
        .check()
        .await
        .map_err(err)?
        .ok_or("nessun aggiornamento disponibile")?;
    let mut downloaded = 0u64;
    let progress_app = app.clone();
    let progress = move |chunk: usize, total: Option<u64>| {
        downloaded += chunk as u64;
        let _ = progress_app.emit("update_progress", Progress { downloaded, total });
    };
    // Signature checked by `download`.
    let bytes = update.download(progress, || {}).await.map_err(err)?;
    // Partners controlling this computer get told before the app quits: their viewer closes
    // instead of freezing on the last picture.
    let service = app.state::<crate::service::Service>();
    // Installed from the MSI (for all users): its installer needs an administrator. Asked
    // here, while the app still runs and the session is still open, so whoever controls
    // this PC remotely sees the UAC prompt and can approve it; the stock updater quits first
    // and the prompt waits for someone at the PC.
    #[cfg(windows)]
    if update
        .download_url
        .path()
        .to_ascii_lowercase()
        .ends_with(".msi")
    {
        let version = update.version.clone();
        let started = tauri::async_runtime::spawn_blocking(move || install_msi(&version, &bytes))
            .await
            .map_err(err)?
            .map_err(err)?;
        if !started {
            return Err(err("permesso di amministratore negato"));
        }
        tracing::info!(version = %update.version, "MSI update started, quitting");
        service.end_hosting_for_update().await;
        // The installer replaces the app and starts it again when done.
        app.exit(0);
        return Ok(());
    }
    service.end_hosting_for_update().await;
    update.install(bytes).map_err(err)?;
    tracing::info!(version = %update.version, "update installed, restarting");
    app.restart();
}

/// Writes the MSI and starts `msiexec` elevated (passive, app started again at the end with
/// the same arguments). `Ok(false)` if the UAC prompt was declined.
#[cfg(windows)]
fn install_msi(version: &str, bytes: &[u8]) -> anyhow::Result<bool> {
    let dir = std::env::temp_dir().join(format!("kronos-update-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let msi = dir.join(format!("Kronos Remote Support-{version}.msi"));
    std::fs::write(&msi, bytes)?;
    let args = std::env::args_os()
        .skip(1)
        .map(|a| msi_property_arg(&a.to_string_lossy()))
        .collect::<Vec<_>>()
        .join(" ");
    let params = format!(
        "/i \"{}\" /passive /promptrestart AUTOLAUNCHAPP=True LAUNCHAPPARGS=\"{args}\"",
        msi.display()
    );
    let msiexec = std::env::var_os("SYSTEMROOT")
        .map(|root| {
            std::path::Path::new(&root)
                .join("System32")
                .join("msiexec.exe")
        })
        .unwrap_or_else(|| "msiexec.exe".into());
    agent::windows::spawn_elevated(&msiexec, &params)
}

/// One argument inside the quoted `LAUNCHAPPARGS` property (as the Tauri updater does).
#[cfg(any(windows, test))]
fn msi_property_arg(arg: &str) -> String {
    if arg.is_empty() {
        return "\"\"\"\"".to_owned();
    }
    if !arg.contains(' ') && !arg.contains('"') {
        return arg.to_owned();
    }
    let arg = arg.replace('"', "\"\"\"\"");
    match arg.split_once('=') {
        Some((name, value)) if arg.starts_with('-') => format!("{name}=\"\"{value}\"\""),
        _ => format!("\"\"{arg}\"\""),
    }
}

#[cfg(test)]
mod tests {
    use super::msi_property_arg;

    #[test]
    fn msi_arguments_quoted_like_the_updater() {
        assert_eq!(msi_property_arg("--hidden"), "--hidden");
        assert_eq!(msi_property_arg(""), "\"\"\"\"");
        assert_eq!(msi_property_arg("a b"), "\"\"a b\"\"");
        assert_eq!(msi_property_arg("--dir=C:\\a b"), "--dir=\"\"C:\\a b\"\"");
    }
}
