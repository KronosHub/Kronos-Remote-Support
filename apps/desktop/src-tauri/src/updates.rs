//! In-app updates (Tauri updater): signed packages published on the public
//! `Remote-Desktop-releases` repository (ADR 0023).

use serde::Serialize;
use tauri::{AppHandle, Emitter};
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
    update
        .download_and_install(
            move |chunk, total| {
                downloaded += chunk as u64;
                let _ = progress_app.emit("update_progress", Progress { downloaded, total });
            },
            || {},
        )
        .await
        .map_err(err)?;
    tracing::info!(version = %update.version, "update installed, restarting");
    app.restart();
}
