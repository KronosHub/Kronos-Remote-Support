//! Export and import of "my devices" (ADR 0057). The file holds the devices and folders;
//! with the saved passwords (each device's derived personal secret, which lets anyone in
//! without approval) it is encrypted with a password chosen at export: PBKDF2-HMAC-SHA256
//! then AES-256-GCM. Without them it is plain JSON with names and folders only.

use std::num::NonZeroU32;

use ring::aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use serde::{Deserialize, Serialize};

use crate::store::{DeviceGroup, SavedDevice};

const FORMAT: &str = "kronos-remote-support-devices";
const VERSION: u32 = 1;
const ITERATIONS: u32 = 600_000;
const SALT_LEN: usize = 16;
/// Shortest password for an encrypted file.
pub const MIN_PASSWORD: usize = 8;
/// Larger files are refused (a list of devices is a few KB).
pub const MAX_FILE_BYTES: usize = 4 * 1024 * 1024;
const MAX_DEVICES: usize = 10_000;
const MAX_NAME: usize = 100;

/// What the file holds once decrypted.
#[derive(Debug, Serialize, Deserialize, PartialEq)]
pub struct DeviceList {
    pub devices: Vec<SavedDevice>,
    pub groups: Vec<DeviceGroup>,
}

#[derive(Serialize, Deserialize)]
struct File {
    format: String,
    version: u32,
    /// Plain list (no saved passwords).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    list: Option<DeviceList>,
    /// Encrypted list (with the saved passwords).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    encrypted: Option<Encrypted>,
}

#[derive(Serialize, Deserialize)]
struct Encrypted {
    iterations: u32,
    salt: String,
    nonce: String,
    data: String,
}

/// The file's text. With `password` the saved passwords are kept and everything is
/// encrypted; without, they are left out.
pub fn export(mut list: DeviceList, password: Option<&str>) -> Result<String, String> {
    let file = match password {
        Some(password) => {
            if password.chars().count() < MIN_PASSWORD {
                return Err(format!(
                    "la password del file deve avere almeno {MIN_PASSWORD} caratteri"
                ));
            }
            let salt: [u8; SALT_LEN] = rand::random();
            let nonce: [u8; NONCE_LEN] = rand::random();
            let mut data = serde_json::to_vec(&list).map_err(|e| e.to_string())?;
            key(password, &salt, ITERATIONS)
                .seal_in_place_append_tag(
                    Nonce::assume_unique_for_key(nonce),
                    Aad::from(FORMAT),
                    &mut data,
                )
                .map_err(|_| "cifratura non riuscita".to_owned())?;
            File {
                format: FORMAT.into(),
                version: VERSION,
                list: None,
                encrypted: Some(Encrypted {
                    iterations: ITERATIONS,
                    salt: hex(&salt),
                    nonce: hex(&nonce),
                    data: hex(&data),
                }),
            }
        }
        None => {
            for device in &mut list.devices {
                device.secret.clear();
            }
            File {
                format: FORMAT.into(),
                version: VERSION,
                list: Some(list),
                encrypted: None,
            }
        }
    };
    serde_json::to_string_pretty(&file).map_err(|e| e.to_string())
}

/// Whether the file needs a password to be read.
pub fn needs_password(text: &str) -> Result<bool, String> {
    Ok(parse(text)?.encrypted.is_some())
}

/// The list in the file, checked; `password` is needed for an encrypted one.
pub fn import(text: &str, password: Option<&str>) -> Result<DeviceList, String> {
    let file = parse(text)?;
    let list = match (file.list, file.encrypted) {
        (Some(list), None) => list,
        (None, Some(encrypted)) => {
            let password = password.ok_or("questo file è protetto da una password")?;
            decrypt(&encrypted, password)?
        }
        _ => return Err(not_a_list()),
    };
    check(list)
}

fn parse(text: &str) -> Result<File, String> {
    if text.len() > MAX_FILE_BYTES {
        return Err("file troppo grande".into());
    }
    let file: File = serde_json::from_str(text).map_err(|_| not_a_list())?;
    if file.format != FORMAT {
        return Err(not_a_list());
    }
    if file.version > VERSION {
        return Err("file creato da una versione più recente dell'app: aggiornala".into());
    }
    Ok(file)
}

fn not_a_list() -> String {
    "non è un file di dispositivi esportato da Kronos Remote Support".into()
}

fn decrypt(encrypted: &Encrypted, password: &str) -> Result<DeviceList, String> {
    let wrong = || "password errata o file danneggiato".to_owned();
    let salt = unhex(&encrypted.salt).ok_or_else(wrong)?;
    let nonce: [u8; NONCE_LEN] = unhex(&encrypted.nonce)
        .and_then(|n| n.try_into().ok())
        .ok_or_else(wrong)?;
    // Same work as at export, within reason (a damaged file must not hang the app).
    if !(1..=10 * ITERATIONS).contains(&encrypted.iterations) {
        return Err(wrong());
    }
    let mut data = unhex(&encrypted.data).ok_or_else(wrong)?;
    let plain = key(password, &salt, encrypted.iterations)
        .open_in_place(
            Nonce::assume_unique_for_key(nonce),
            Aad::from(FORMAT),
            &mut data,
        )
        .map_err(|_| wrong())?;
    serde_json::from_slice(plain).map_err(|_| wrong())
}

fn key(password: &str, salt: &[u8], iterations: u32) -> LessSafeKey {
    let mut bytes = [0u8; 32];
    ring::pbkdf2::derive(
        ring::pbkdf2::PBKDF2_HMAC_SHA256,
        NonZeroU32::new(iterations).expect("iterations > 0"),
        salt,
        password.as_bytes(),
        &mut bytes,
    );
    LessSafeKey::new(UnboundKey::new(&AES_256_GCM, &bytes).expect("32-byte key"))
}

/// Keeps what a hand-edited or damaged file may get wrong out of the saved list.
fn check(mut list: DeviceList) -> Result<DeviceList, String> {
    if list.devices.len() > MAX_DEVICES || list.groups.len() > MAX_DEVICES {
        return Err("troppi dispositivi nel file".into());
    }
    for device in &mut list.devices {
        device.id.retain(|c| c.is_ascii_digit());
        device.name = device.name.trim().chars().take(MAX_NAME).collect();
        if device.name.is_empty() {
            device.name = device.id.clone();
        }
    }
    list.devices.retain(|d| d.id.len() == 9);
    let mut seen = std::collections::HashSet::new();
    list.devices.retain(|d| seen.insert(d.id.clone()));
    list.groups
        .retain(|g| !g.id.is_empty() && !g.name.trim().is_empty());
    for group in &mut list.groups {
        group.name = group
            .name
            .trim()
            .chars()
            .take(crate::store::MAX_GROUP_NAME)
            .collect();
    }
    Ok(list)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn list() -> DeviceList {
        DeviceList {
            devices: vec![SavedDevice {
                id: "123456789".into(),
                name: "Ufficio".into(),
                secret: "segreto".into(),
                group: Some("g1".into()),
            }],
            groups: vec![DeviceGroup {
                id: "g1".into(),
                name: "Clienti".into(),
                parent: None,
                collapsed: false,
            }],
        }
    }

    #[test]
    fn encrypted_file_keeps_the_secrets_and_needs_the_password() {
        let text = export(list(), Some("password lunga")).unwrap();
        assert!(!text.contains("segreto") && !text.contains("Ufficio"));
        assert!(needs_password(&text).unwrap());
        assert_eq!(import(&text, Some("password lunga")).unwrap(), list());
        assert!(import(&text, Some("sbagliata!")).is_err());
        assert!(import(&text, None).is_err());
    }

    #[test]
    fn plain_file_leaves_the_secrets_out() {
        let text = export(list(), None).unwrap();
        assert!(!needs_password(&text).unwrap());
        let imported = import(&text, None).unwrap();
        assert_eq!(imported.devices[0].secret, "");
        assert_eq!(imported.devices[0].name, "Ufficio");
        assert_eq!(imported.groups, list().groups);
    }

    #[test]
    fn short_passwords_and_other_files_are_refused() {
        assert!(export(list(), Some("corta")).is_err());
        assert!(import("{}", None).is_err());
        assert!(import("non json", None).is_err());
    }

    #[test]
    fn bad_ids_are_dropped_and_names_trimmed() {
        let mut bad = list();
        bad.devices.push(SavedDevice {
            id: "12".into(),
            name: "x".into(),
            secret: String::new(),
            group: None,
        });
        bad.devices[0].id = "123 456 789".into();
        bad.devices[0].name = "  ".into();
        let checked = check(bad).unwrap();
        assert_eq!(checked.devices.len(), 1);
        assert_eq!(checked.devices[0].name, "123456789");
    }
}
