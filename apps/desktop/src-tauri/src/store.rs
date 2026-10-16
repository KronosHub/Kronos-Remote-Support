//! Small JSON files in the app data directory: the personal (unattended) secret of this
//! device and the list of "my devices" this machine can open without approval, in folders.
//!
//! Only secrets derived with `auth::derive_personal_secret` are stored, never passwords.
//! Files are written atomically and readable by the current user only.

use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const PERSONAL_FILE: &str = "personal.json";
const DEVICES_FILE: &str = "devices.json";
const GROUPS_FILE: &str = "groups.json";
/// Longest folder name, in characters.
pub const MAX_GROUP_NAME: usize = 40;
#[cfg_attr(not(windows), allow(dead_code))]
const PREFS_FILE: &str = "prefs.json";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SavedDevice {
    /// Peer id without spaces.
    pub id: String,
    pub name: String,
    /// Derived personal secret of that device.
    #[serde(skip_serializing_if = "String::is_empty", default)]
    pub secret: String,
    /// Folder it is in (`DeviceGroup::id`); `None`, or a folder that no longer exists: the
    /// top of the list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
}

/// A folder of "my devices"; folders nest through `parent`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DeviceGroup {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// Shown closed in the list.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub collapsed: bool,
}

#[derive(Serialize, Deserialize)]
struct Personal {
    secret: String,
}

/// Small per-device preferences (not secrets).
#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Serialize, Deserialize, Default)]
struct Prefs {
    /// Windows: the user declined installing the admin-windows (UAC) agent service, so we
    /// do not ask again at startup (they can still enable it from the button).
    #[serde(default)]
    uac_agent_declined: bool,
    /// This device's id, cached after the first server registration, so LAN discovery works
    /// even when the server is unreachable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cached_id: Option<String>,
    /// Names the user gave to devices seen on the local network, by device id.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    lan_names: std::collections::BTreeMap<String, String>,
    /// The user's name, shown to partners.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    display_name: Option<String>,
}

pub struct Store {
    /// `None`: nothing is read or written (Quick access leaves no files behind).
    dir: Option<PathBuf>,
}

impl Store {
    pub fn new(dir: Option<PathBuf>) -> Self {
        Self { dir }
    }

    fn dir(&self) -> std::io::Result<&Path> {
        self.dir.as_deref().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "non disponibile in Quick access",
            )
        })
    }

    fn read<T: for<'de> Deserialize<'de>>(&self, name: &str) -> Option<T> {
        let bytes = std::fs::read(self.dir().ok()?.join(name)).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    fn write<T: Serialize>(&self, name: &str, value: &T) -> std::io::Result<()> {
        let path = self.dir()?.join(name);
        let tmp = path.with_extension("tmp");
        let mut file = private_file(&tmp)?;
        file.write_all(&serde_json::to_vec_pretty(value).expect("serializable"))?;
        file.sync_all()?;
        std::fs::rename(&tmp, &path)
    }

    pub fn personal_secret(&self) -> Option<String> {
        self.read::<Personal>(PERSONAL_FILE).map(|p| p.secret)
    }

    pub fn set_personal_secret(&self, secret: Option<&str>) -> std::io::Result<()> {
        match secret {
            Some(secret) => self.write(
                PERSONAL_FILE,
                &Personal {
                    secret: secret.to_owned(),
                },
            ),
            None => match std::fs::remove_file(self.dir()?.join(PERSONAL_FILE)) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
                _ => Ok(()),
            },
        }
    }

    pub fn devices(&self) -> Vec<SavedDevice> {
        self.read(DEVICES_FILE).unwrap_or_default()
    }

    /// Adds or replaces the device with the same id (which stays in its folder).
    pub fn save_device(&self, mut device: SavedDevice) -> std::io::Result<()> {
        let mut devices = self.devices();
        if let Some(old) = devices.iter().find(|d| d.id == device.id) {
            device.group = device.group.or_else(|| old.group.clone());
        }
        devices.retain(|d| d.id != device.id);
        devices.push(device);
        devices.sort_by_key(|d| d.name.to_lowercase());
        self.write(DEVICES_FILE, &devices)
    }

    pub fn remove_device(&self, id: &str) -> std::io::Result<()> {
        let mut devices = self.devices();
        devices.retain(|d| d.id != id);
        self.write(DEVICES_FILE, &devices)
    }

    /// The folders, always a tree: one whose parent is missing, or that would be inside
    /// itself (a damaged file), is shown at the top.
    pub fn groups(&self) -> Vec<DeviceGroup> {
        let mut groups: Vec<DeviceGroup> = self.read(GROUPS_FILE).unwrap_or_default();
        for i in 0..groups.len() {
            let broken = groups[i].parent.as_deref().is_some_and(|parent| {
                !groups.iter().any(|g| g.id == parent) || is_within(&groups, parent, &groups[i].id)
            });
            if broken {
                groups[i].parent = None;
            }
        }
        groups
    }

    fn write_groups(&self, groups: &mut Vec<DeviceGroup>) -> std::io::Result<()> {
        groups.sort_by_key(|g| g.name.to_lowercase());
        self.write(GROUPS_FILE, groups)
    }

    /// New folder inside `parent` (`None`: at the top); returns its id.
    pub fn create_group(&self, name: &str, parent: Option<&str>) -> Result<String, String> {
        let name = group_name(name)?;
        let mut groups = self.groups();
        if let Some(parent) = parent
            && !groups.iter().any(|g| g.id == parent)
        {
            return Err("cartella non trovata".into());
        }
        let id = format!("{:016x}", rand::random::<u64>());
        groups.push(DeviceGroup {
            id: id.clone(),
            name,
            parent: parent.map(str::to_owned),
            collapsed: false,
        });
        self.write_groups(&mut groups).map_err(save_error)?;
        Ok(id)
    }

    pub fn rename_group(&self, id: &str, name: &str) -> Result<(), String> {
        let name = group_name(name)?;
        self.edit_group(id, |g| g.name = name)
    }

    pub fn set_group_collapsed(&self, id: &str, collapsed: bool) -> Result<(), String> {
        self.edit_group(id, |g| g.collapsed = collapsed)
    }

    /// Moves a folder into `parent` (`None`: to the top), never into itself or one of its
    /// subfolders.
    pub fn move_group(&self, id: &str, parent: Option<&str>) -> Result<(), String> {
        let groups = self.groups();
        if let Some(parent) = parent {
            if !groups.iter().any(|g| g.id == parent) {
                return Err("cartella non trovata".into());
            }
            if is_within(&groups, parent, id) {
                return Err("una cartella non può stare dentro sé stessa".into());
            }
        }
        self.edit_group(id, |g| g.parent = parent.map(str::to_owned))
    }

    fn edit_group(&self, id: &str, edit: impl FnOnce(&mut DeviceGroup)) -> Result<(), String> {
        let mut groups = self.groups();
        let group = groups
            .iter_mut()
            .find(|g| g.id == id)
            .ok_or("cartella non trovata")?;
        edit(group);
        self.write_groups(&mut groups).map_err(save_error)
    }

    /// Removes a folder; its devices and subfolders move to the folder that held it.
    pub fn delete_group(&self, id: &str) -> Result<(), String> {
        let mut groups = self.groups();
        let parent = groups
            .iter()
            .find(|g| g.id == id)
            .ok_or("cartella non trovata")?
            .parent
            .clone();
        let mut devices = self.devices();
        for device in devices
            .iter_mut()
            .filter(|d| d.group.as_deref() == Some(id))
        {
            device.group = parent.clone();
        }
        self.write(DEVICES_FILE, &devices).map_err(save_error)?;
        groups.retain(|g| g.id != id);
        for group in groups
            .iter_mut()
            .filter(|g| g.parent.as_deref() == Some(id))
        {
            group.parent = parent.clone();
        }
        self.write_groups(&mut groups).map_err(save_error)
    }

    /// Puts a saved device in a folder (`None`: at the top).
    pub fn move_device(&self, id: &str, group: Option<&str>) -> Result<(), String> {
        if let Some(group) = group
            && !self.groups().iter().any(|g| g.id == group)
        {
            return Err("cartella non trovata".into());
        }
        let mut devices = self.devices();
        let device = devices
            .iter_mut()
            .find(|d| d.id == id)
            .ok_or("dispositivo non trovato")?;
        device.group = group.map(str::to_owned);
        self.write(DEVICES_FILE, &devices).map_err(save_error)
    }

    /// Whether the user declined the admin-windows agent (Windows); default false.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn uac_agent_declined(&self) -> bool {
        self.read::<Prefs>(PREFS_FILE)
            .unwrap_or_default()
            .uac_agent_declined
    }

    #[cfg_attr(not(windows), allow(dead_code))]
    pub fn set_uac_agent_declined(&self, declined: bool) -> std::io::Result<()> {
        let mut prefs = self.read::<Prefs>(PREFS_FILE).unwrap_or_default();
        prefs.uac_agent_declined = declined;
        self.write(PREFS_FILE, &prefs)
    }

    pub fn lan_names(&self) -> std::collections::BTreeMap<String, String> {
        self.read::<Prefs>(PREFS_FILE).unwrap_or_default().lan_names
    }

    /// Sets (or with an empty name removes) the name of a LAN device.
    pub fn set_lan_name(&self, id: &str, name: &str) -> std::io::Result<()> {
        let mut prefs = self.read::<Prefs>(PREFS_FILE).unwrap_or_default();
        if name.is_empty() {
            prefs.lan_names.remove(id);
        } else {
            prefs.lan_names.insert(id.to_owned(), name.to_owned());
        }
        self.write(PREFS_FILE, &prefs)
    }

    pub fn display_name(&self) -> Option<String> {
        self.read::<Prefs>(PREFS_FILE).and_then(|p| p.display_name)
    }

    pub fn set_display_name(&self, name: Option<&str>) -> std::io::Result<()> {
        let mut prefs = self.read::<Prefs>(PREFS_FILE).unwrap_or_default();
        prefs.display_name = name.map(str::to_owned);
        self.write(PREFS_FILE, &prefs)
    }

    pub fn cached_id(&self) -> Option<String> {
        self.read::<Prefs>(PREFS_FILE).and_then(|p| p.cached_id)
    }

    pub fn set_cached_id(&self, id: &str) -> std::io::Result<()> {
        let mut prefs = self.read::<Prefs>(PREFS_FILE).unwrap_or_default();
        prefs.cached_id = Some(id.to_owned());
        self.write(PREFS_FILE, &prefs)
    }
}

fn group_name(name: &str) -> Result<String, String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("scrivi un nome per la cartella".into());
    }
    if name.chars().count() > MAX_GROUP_NAME {
        return Err(format!(
            "nome troppo lungo (massimo {MAX_GROUP_NAME} caratteri)"
        ));
    }
    Ok(name.to_owned())
}

fn save_error(e: std::io::Error) -> String {
    format!("impossibile salvare: {e}")
}

/// Whether folder `id` is `ancestor` or lies somewhere inside it.
fn is_within(groups: &[DeviceGroup], id: &str, ancestor: &str) -> bool {
    let mut current = Some(id);
    // Bounded: a damaged file with a loop must not hang the app.
    for _ in 0..=groups.len() {
        let Some(id) = current else {
            return false;
        };
        if id == ancestor {
            return true;
        }
        current = groups
            .iter()
            .find(|g| g.id == id)
            .and_then(|g| g.parent.as_deref());
    }
    true
}

/// Makes an existing file readable by its owner only.
#[cfg(unix)]
pub(crate) fn restrict_permissions(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
}

#[cfg(not(unix))]
pub(crate) fn restrict_permissions(_path: &Path) {}

#[cfg(unix)]
pub(crate) fn private_file(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
pub(crate) fn private_file(path: &Path) -> std::io::Result<std::fs::File> {
    // The per-user app data directory is already private on Windows.
    std::fs::File::create(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (Store, PathBuf) {
        let dir = std::env::temp_dir().join(format!(
            "desktop-remoto-store-{}-{}",
            std::process::id(),
            rand::random::<u32>()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        (Store::new(Some(dir.clone())), dir)
    }

    #[test]
    fn personal_secret_roundtrip() {
        let (store, dir) = store();
        assert_eq!(store.personal_secret(), None);
        store.set_personal_secret(Some("abc")).unwrap();
        assert_eq!(store.personal_secret().as_deref(), Some("abc"));
        store.set_personal_secret(None).unwrap();
        store.set_personal_secret(None).unwrap();
        assert_eq!(store.personal_secret(), None);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn devices_are_upserted_and_sorted() {
        let (store, dir) = store();
        let dev = |id: &str, name: &str| SavedDevice {
            id: id.into(),
            name: name.into(),
            secret: "s".into(),
            group: None,
        };
        store.save_device(dev("2", "ufficio")).unwrap();
        store.save_device(dev("1", "Casa")).unwrap();
        store.save_device(dev("2", "Ufficio PC")).unwrap();
        let names: Vec<_> = store.devices().into_iter().map(|d| d.name).collect();
        assert_eq!(names, ["Casa", "Ufficio PC"]);
        store.remove_device("1").unwrap();
        assert_eq!(store.devices().len(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn folders_nest_move_and_dissolve_into_their_parent() {
        let (store, dir) = store();
        let dev = |id: &str| SavedDevice {
            id: id.into(),
            name: id.into(),
            secret: "s".into(),
            group: None,
        };
        let clients = store.create_group(" Clienti ", None).unwrap();
        let rossi = store.create_group("Rossi", Some(&clients)).unwrap();
        assert!(store.create_group("  ", None).is_err());
        assert!(store.create_group("x", Some("missing")).is_err());
        assert_eq!(store.groups()[0].name, "Clienti");

        store.save_device(dev("1")).unwrap();
        store.move_device("1", Some(&rossi)).unwrap();
        // Saving it again (new password) keeps it in its folder.
        store.save_device(dev("1")).unwrap();
        assert_eq!(store.devices()[0].group.as_deref(), Some(rossi.as_str()));
        assert!(store.move_device("1", Some("missing")).is_err());

        // A folder cannot go inside itself or its subfolders.
        assert!(store.move_group(&clients, Some(&rossi)).is_err());
        assert!(store.move_group(&clients, Some(&clients)).is_err());
        store.move_group(&rossi, None).unwrap();
        store.move_group(&rossi, Some(&clients)).unwrap();

        store.set_group_collapsed(&clients, true).unwrap();
        store.rename_group(&clients, "Clienti 2026").unwrap();
        let group = |id: &str| store.groups().into_iter().find(|g| g.id == id).unwrap();
        assert!(group(&clients).collapsed);
        assert_eq!(group(&clients).name, "Clienti 2026");

        // Deleting "Rossi" moves its device up into "Clienti"; deleting "Clienti" moves
        // everything to the top. No device is ever lost.
        store.delete_group(&rossi).unwrap();
        assert_eq!(store.devices()[0].group.as_deref(), Some(clients.as_str()));
        store.delete_group(&clients).unwrap();
        assert_eq!(store.devices()[0].group, None);
        assert!(store.groups().is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_folder_loop_in_a_damaged_file_does_not_hang() {
        let g = |id: &str, parent: &str| DeviceGroup {
            id: id.into(),
            name: id.into(),
            parent: Some(parent.into()),
            collapsed: false,
        };
        let groups = [g("a", "b"), g("b", "a")];
        assert!(is_within(&groups, "a", "c"));
        let (store, dir) = store();
        std::fs::write(dir.join(GROUPS_FILE), serde_json::to_vec(&groups).unwrap()).unwrap();
        // Shown as a tree: the loop is broken where it is first seen.
        let shown = store.groups();
        assert_eq!(shown.iter().filter(|g| g.parent.is_none()).count(), 1);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn devices_saved_before_folders_still_load() {
        let (store, dir) = store();
        std::fs::write(
            dir.join(DEVICES_FILE),
            r#"[{"id":"123456789","name":"PC","secret":"s"}]"#,
        )
        .unwrap();
        assert_eq!(store.devices()[0].group, None);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn lan_names_set_and_cleared() {
        let (store, dir) = store();
        store.set_lan_name("123456789", "PC reception").unwrap();
        store.set_cached_id("987654321").unwrap();
        assert_eq!(
            store.lan_names().get("123456789").map(String::as_str),
            Some("PC reception")
        );
        // Other preferences survive.
        assert_eq!(store.cached_id().as_deref(), Some("987654321"));
        store.set_lan_name("123456789", "").unwrap();
        assert!(store.lan_names().is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn without_directory_nothing_is_stored() {
        let store = Store::new(None);
        assert!(store.set_personal_secret(Some("abc")).is_err());
        assert_eq!(store.personal_secret(), None);
        assert!(store.devices().is_empty());
    }

    #[cfg(unix)]
    #[test]
    fn files_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let (store, dir) = store();
        store.set_personal_secret(Some("abc")).unwrap();
        let mode = std::fs::metadata(dir.join(PERSONAL_FILE))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
