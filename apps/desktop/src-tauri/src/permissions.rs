//! macOS privacy permissions needed to be watched and controlled, so the UI can show what
//! is missing and open the right System Settings pane. Other platforms need none.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct Permissions {
    /// False on platforms where these permissions do not exist.
    pub required: bool,
    /// Screen Recording: needed to share the screen.
    pub screen: bool,
    /// Accessibility: needed to be controlled (mouse/keyboard).
    pub accessibility: bool,
}

#[cfg(target_os = "macos")]
mod mac {
    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGPreflightScreenCaptureAccess() -> bool;
        fn CGRequestScreenCaptureAccess() -> bool;
    }

    #[link(name = "ApplicationServices", kind = "framework")]
    unsafe extern "C" {
        fn AXIsProcessTrusted() -> bool;
    }

    pub fn screen() -> bool {
        unsafe { CGPreflightScreenCaptureAccess() }
    }

    pub fn accessibility() -> bool {
        unsafe { AXIsProcessTrusted() }
    }

    /// Shows the system prompt (first time) and registers the app in the Screen Recording list.
    pub fn request_screen() {
        unsafe { CGRequestScreenCaptureAccess() };
    }
}

#[tauri::command]
pub fn get_permissions() -> Permissions {
    #[cfg(target_os = "macos")]
    return Permissions {
        required: true,
        screen: mac::screen(),
        accessibility: mac::accessibility(),
    };
    #[cfg(not(target_os = "macos"))]
    Permissions {
        required: false,
        screen: true,
        accessibility: true,
    }
}

/// Opens the System Settings pane for `kind` ("screen" or "accessibility").
#[tauri::command]
pub fn open_permission_settings(kind: String) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        let pane = match kind.as_str() {
            "screen" => {
                // Makes sure the app is listed in the pane (and prompts the first time).
                mac::request_screen();
                "Privacy_ScreenCapture"
            }
            "accessibility" => {
                input::prompt_accessibility();
                "Privacy_Accessibility"
            }
            _ => return Err("permesso sconosciuto".into()),
        };
        std::process::Command::new("open")
            .arg(format!(
                "x-apple.systempreferences:com.apple.preference.security?{pane}"
            ))
            .spawn()
            .map_err(|e| format!("impossibile aprire le Impostazioni: {e}"))?;
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = kind;
        Ok(())
    }
}
