//! Ctrl+Alt+Del on Windows: the secure attention sequence (SAS).
//!
//! Injected keys never produce it; `SendSAS` from `sas.dll` asks Winlogon for it. Windows only
//! honours that from a LocalSystem process (our agent, ADR 0033) and only while the
//! `SoftwareSASGeneration` policy allows services, so the policy is switched on for the call
//! and put back as it was right after.

use anyhow::{Result, bail};
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, FreeLibrary};
use windows::Win32::System::LibraryLoader::{
    GetProcAddress, LOAD_LIBRARY_SEARCH_SYSTEM32, LoadLibraryExW,
};
use windows::Win32::System::Registry::{
    HKEY, HKEY_LOCAL_MACHINE, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_DWORD, REG_VALUE_TYPE,
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
};
use windows::core::{BOOL, PCWSTR, s, w};

const POLICY_KEY: PCWSTR = w!("SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Policies\\System");
const POLICY_VALUE: PCWSTR = w!("SoftwareSASGeneration");
/// Policy bit allowing services (1 = services, 2 = Ease of Access apps, 3 = both).
const SERVICES: u32 = 1;

pub fn send() -> Result<()> {
    let policy = Policy::open()?;
    let before = policy.read();
    let allowed = before.is_some_and(|v| v & SERVICES != 0);
    if !allowed {
        policy.write(Some(before.unwrap_or(0) | SERVICES))?;
    }
    let result = call_send_sas();
    if !allowed && let Err(e) = policy.write(before) {
        tracing::warn!("cannot restore the SoftwareSASGeneration policy: {e:#}");
    }
    result
}

fn call_send_sas() -> Result<()> {
    // SAFETY: sas.dll comes from System32 only; `SendSAS(BOOL AsUser)` returns nothing.
    unsafe {
        let module = LoadLibraryExW(w!("sas.dll"), None, LOAD_LIBRARY_SEARCH_SYSTEM32)?;
        let Some(proc) = GetProcAddress(module, s!("SendSAS")) else {
            let _ = FreeLibrary(module);
            bail!("SendSAS non trovata in sas.dll");
        };
        let send_sas: extern "system" fn(BOOL) = std::mem::transmute(proc);
        // FALSE: the caller is a LocalSystem process, not the logged-on user.
        send_sas(BOOL(0));
        let _ = FreeLibrary(module);
    }
    tracing::info!("Ctrl+Alt+Del sent");
    Ok(())
}

/// The policy key under HKLM, open for reading and writing (needs SYSTEM or an administrator).
struct Policy(HKEY);

impl Policy {
    fn open() -> Result<Self> {
        let mut key = HKEY::default();
        // SAFETY: plain registry call with a valid out pointer.
        let status = unsafe {
            RegOpenKeyExW(
                HKEY_LOCAL_MACHINE,
                POLICY_KEY,
                None,
                KEY_QUERY_VALUE | KEY_SET_VALUE,
                &mut key,
            )
        };
        if status != ERROR_SUCCESS {
            bail!(
                "Ctrl+Alt+Canc richiede il servizio di Kronos (Controllo finestre di \
                 amministratore) sul computer controllato (registro: {status:?})"
            );
        }
        Ok(Self(key))
    }

    fn read(&self) -> Option<u32> {
        let mut kind = REG_VALUE_TYPE::default();
        let mut value = 0u32;
        let mut size = std::mem::size_of::<u32>() as u32;
        // SAFETY: the buffer is a u32 and `size` says so.
        let status = unsafe {
            RegQueryValueExW(
                self.0,
                POLICY_VALUE,
                None,
                Some(&mut kind),
                Some((&mut value as *mut u32).cast()),
                Some(&mut size),
            )
        };
        (status == ERROR_SUCCESS && kind == REG_DWORD).then_some(value)
    }

    /// Sets the value, or removes it for `None` (it was not there).
    fn write(&self, value: Option<u32>) -> Result<()> {
        // SAFETY: plain registry calls on an open key.
        let status = unsafe {
            match value {
                Some(v) => RegSetValueExW(
                    self.0,
                    POLICY_VALUE,
                    None,
                    REG_DWORD,
                    Some(&v.to_le_bytes()),
                ),
                None => RegDeleteValueW(self.0, POLICY_VALUE),
            }
        };
        if status != ERROR_SUCCESS && !(value.is_none() && status == ERROR_FILE_NOT_FOUND) {
            bail!("SoftwareSASGeneration non modificabile ({status:?})");
        }
        Ok(())
    }
}

impl Drop for Policy {
    fn drop(&mut self) {
        // SAFETY: the key was opened by `open`.
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}
