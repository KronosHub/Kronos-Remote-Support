//! "Scarica i log": the app's log files (and, on Windows, those of the service and of the
//! pre-login host) in one zip in the Downloads folder, to send along with a problem report.

use std::io::Write;
use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager};
use zip::write::SimpleFileOptions;

/// Log files larger than this are cut to their last part (a day of `trace` can be huge).
const MAX_FILE: u64 = 32 * 1024 * 1024;

/// The folders holding logs, each with the name it gets inside the zip.
fn log_dirs(app: &AppHandle) -> Vec<(&'static str, PathBuf)> {
    let mut dirs = Vec::new();
    if let Ok(dir) = app.path().app_log_dir() {
        dirs.push(("app", dir));
    }
    #[cfg(windows)]
    if let Some(dir) = agent::windows::log_dir() {
        dirs.push(("servizio", dir));
    }
    dirs
}

/// The last `MAX_FILE` bytes of a file (the newest lines), read without locking it: the
/// logger keeps writing to today's file.
fn tail(path: &Path) -> std::io::Result<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    if len > MAX_FILE {
        file.seek(SeekFrom::Start(len - MAX_FILE))?;
    }
    let mut data = Vec::with_capacity(len.min(MAX_FILE) as usize);
    file.read_to_end(&mut data)?;
    Ok(data)
}

fn about() -> String {
    format!(
        "Kronos Remote Support {}\nSistema: {} {}\nCompilazione: {}\n",
        env!("CARGO_PKG_VERSION"),
        std::env::consts::OS,
        std::env::consts::ARCH,
        if cfg!(feature = "quick-access") {
            "Quick Support"
        } else {
            "completa"
        },
    )
}

fn write_zip(path: &Path, dirs: &[(&str, PathBuf)]) -> Result<usize, String> {
    let file =
        std::fs::File::create(path).map_err(|e| format!("impossibile creare il file: {e}"))?;
    let mut zip = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
    let error = |e: &dyn std::fmt::Display| format!("impossibile scrivere il file: {e}");
    zip.start_file("info.txt", options).map_err(|e| error(&e))?;
    zip.write_all(about().as_bytes()).map_err(|e| error(&e))?;
    let mut count = 0;
    for (name, dir) in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let is_log = path.is_file()
                && path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.contains(".log"));
            if !is_log {
                continue;
            }
            // A file that cannot be read (e.g. the service's, without rights) is left out.
            let Ok(data) = tail(&path) else {
                tracing::warn!(path = %path.display(), "log not readable, left out");
                continue;
            };
            let file_name = entry.file_name().to_string_lossy().into_owned();
            zip.start_file(format!("{name}/{file_name}"), options)
                .map_err(|e| error(&e))?;
            zip.write_all(&data).map_err(|e| error(&e))?;
            count += 1;
        }
    }
    zip.finish().map_err(|e| error(&e))?;
    Ok(count)
}

/// Saves the logs as a zip in the Downloads folder and returns its path.
#[tauri::command]
pub async fn export_logs(app: AppHandle) -> Result<String, String> {
    let path = crate::download_path(&app, "log-kronos", "zip")?;
    let dirs = log_dirs(&app);
    let saved = path.clone();
    let count = tauri::async_runtime::spawn_blocking(move || write_zip(&saved, &dirs))
        .await
        .map_err(|e| e.to_string())?
        .inspect_err(|_| {
            let _ = std::fs::remove_file(&path);
        })?;
    tracing::info!(files = count, path = %path.display(), "logs exported");
    Ok(path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zips_the_log_files_only() {
        let root = std::env::temp_dir().join(format!("kronos-logs-test-{}", std::process::id()));
        let dir = root.join("logs");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("kronos.2026-11-11.log"), "first line\n").unwrap();
        std::fs::write(dir.join("other.txt"), "not a log").unwrap();
        let path = root.join("out.zip");

        let count = write_zip(
            &path,
            &[("app", dir.clone()), ("missing", root.join("none"))],
        )
        .unwrap();
        assert_eq!(count, 1);
        let mut zip = zip::ZipArchive::new(std::fs::File::open(&path).unwrap()).unwrap();
        let mut names: Vec<_> = zip.file_names().map(str::to_owned).collect();
        names.sort();
        assert_eq!(names, ["app/kronos.2026-11-11.log", "info.txt"]);
        let mut text = String::new();
        std::io::Read::read_to_string(
            &mut zip.by_name("app/kronos.2026-11-11.log").unwrap(),
            &mut text,
        )
        .unwrap();
        assert_eq!(text, "first line\n");
        std::fs::remove_dir_all(&root).unwrap();
    }
}
