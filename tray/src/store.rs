//! Settings, cached state, and the password.
//!
//! The password goes to the operating system's credential store (Keychain on
//! macOS, Credential Manager on Windows, Secret Service on Linux) and nowhere
//! else. Settings and the session cache are small JSON files in the per-user
//! config directory, written atomically and private to the user.

use crate::model::{Thresholds, Unit};
use crate::share::{Reading, Region};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const KEYRING_SERVICE: &str = "G7 Companion";
const KEYRING_USER: &str = "dexcom-share";
const FILE_MAX_BYTES: u64 = 256 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub region: Region,
    pub account_name: Option<String>,
    pub account_id: Option<String>,
    pub unit: Unit,
    pub low: i64,
    pub high: i64,
    pub urgent_low: i64,
    pub refresh_sec: u64,
    pub stale_minutes: i64,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            region: Region::Us,
            account_name: None,
            account_id: None,
            unit: Unit::Mgdl,
            low: 70,
            high: 180,
            urgent_low: 55,
            refresh_sec: 60,
            stale_minutes: 10,
        }
    }
}

impl Settings {
    pub fn thresholds(&self) -> Thresholds {
        Thresholds::sane(self.urgent_low, self.low, self.high)
    }

    pub fn refresh_sec(&self) -> u64 {
        self.refresh_sec.clamp(30, 600)
    }

    pub fn stale_minutes(&self) -> i64 {
        self.stale_minutes.clamp(6, 60)
    }

    pub fn has_account(&self) -> bool {
        self.account_id.as_deref().is_some_and(crate::share::is_uuid)
            || self.account_name.as_deref().is_some_and(|n| !n.is_empty())
    }

    /// Enough to recognise the account without printing it in full.
    pub fn account_label(&self) -> String {
        match (&self.account_name, &self.account_id) {
            (Some(name), _) if name.contains('@') => {
                let (local, domain) = name.split_once('@').unwrap_or((name, ""));
                format!("{}***@{}", local.chars().next().unwrap_or('*'), domain)
            }
            (Some(name), _) if name.chars().count() > 4 => {
                let tail: String = name.chars().rev().take(4).collect::<Vec<_>>().into_iter().rev().collect();
                format!("***{tail}")
            }
            (Some(name), _) if !name.is_empty() => "***".into(),
            (_, Some(_)) => "account ID".into(),
            _ => String::new(),
        }
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    pub session_id: Option<String>,
    pub session_account_id: Option<String>,
    pub readings: Vec<Reading>,
    pub fetched_at: i64,
    pub backoff_until: i64,
    pub backoff_message: String,
    pub backoff_lockout: bool,
    pub auth_failed: bool,
    pub auth_message: String,
    pub last_error: String,
}

pub struct Store {
    dir: PathBuf,
}

impl Store {
    pub fn open() -> std::io::Result<Store> {
        let base = dirs::config_dir().ok_or_else(|| std::io::Error::other("no per-user config directory"))?;
        Store::at(base.join("G7 Companion"))
    }

    pub fn at(dir: PathBuf) -> std::io::Result<Store> {
        create_private_dir(&dir)?;
        Ok(Store { dir })
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    pub fn settings_path(&self) -> PathBuf {
        self.dir.join("settings.json")
    }

    pub fn load_settings(&self) -> Settings {
        read_json(&self.settings_path()).unwrap_or_default()
    }

    pub fn save_settings(&self, s: &Settings) -> std::io::Result<()> {
        write_json(&self.settings_path(), s)
    }

    pub fn load_state(&self) -> State {
        let mut st: State = read_json(&self.dir.join("state.json")).unwrap_or_default();
        st.readings.retain(|r| (20..=600).contains(&r.v));
        st.readings.truncate(288);
        st
    }

    pub fn save_state(&self, s: &State) -> std::io::Result<()> {
        write_json(&self.dir.join("state.json"), s)
    }

    pub fn clear(&self) -> std::io::Result<()> {
        for name in ["state.json"] {
            match fs::remove_file(self.dir.join(name)) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
        }
        let mut s = self.load_settings();
        s.account_name = None;
        s.account_id = None;
        self.save_settings(&s)?;
        delete_password()
    }
}

fn entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER).map_err(|e| format!("The system credential store is not available: {e}"))
}

pub fn load_password() -> Result<Option<String>, String> {
    match entry()?.get_password() {
        Ok(p) => Ok(Some(p)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("Could not read the password from the credential store: {e}")),
    }
}

pub fn save_password(p: &str) -> Result<(), String> {
    entry()?.set_password(p).map_err(|e| format!("Could not save the password to the credential store: {e}"))
}

pub fn delete_password() -> std::io::Result<()> {
    match entry().map_err(std::io::Error::other)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(std::io::Error::other(e.to_string())),
    }
}

fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        if let Ok(meta) = fs::symlink_metadata(dir) {
            if meta.file_type().is_symlink() || !meta.is_dir() {
                return Err(std::io::Error::other(format!("{} is not a directory", dir.display())));
            }
            if meta.permissions().mode() & 0o077 != 0 {
                fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
            }
            return Ok(());
        }
        fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)
    }
    #[cfg(not(unix))]
    {
        fs::create_dir_all(dir)
    }
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Option<T> {
    let meta = fs::symlink_metadata(path).ok()?;
    if !meta.is_file() || meta.len() > FILE_MAX_BYTES {
        return None;
    }
    let raw = fs::read(path).ok()?;
    if raw.len() as u64 > FILE_MAX_BYTES {
        return None;
    }
    serde_json::from_slice(&raw).ok()
}

fn write_json<T: Serialize>(path: &Path, value: &T) -> std::io::Result<()> {
    let data = serde_json::to_vec_pretty(value).map_err(std::io::Error::other)?;
    let dir = path.parent().ok_or_else(|| std::io::Error::other("no parent directory"))?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let tmp = dir.join(format!(".{name}.{}.tmp", random_suffix()));
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // create_new is O_CREAT|O_EXCL, which refuses an existing name,
        // a planted symlink included, so the temporary is always ours.
        opts.mode(0o600);
    }
    let result = (|| {
        let mut f = opts.open(&tmp)?;
        f.write_all(&data)?;
        f.sync_all()?;
        drop(f);
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

fn random_suffix() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0));
    h.write_u32(std::process::id());
    format!("{:016x}", h.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_private_files() {
        let dir = std::env::temp_dir().join(format!("g7-store-test-{}", random_suffix()));
        let store = Store::at(dir.clone()).unwrap();
        let mut s = Settings::default();
        s.account_name = Some("kid@example.com".into());
        store.save_settings(&s).unwrap();
        assert_eq!(store.load_settings().account_name.as_deref(), Some("kid@example.com"));
        assert_eq!(store.load_settings().account_label(), "k***@example.com");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(fs::metadata(store.settings_path()).unwrap().permissions().mode() & 0o777, 0o600);
            assert_eq!(fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
        }
        fs::write(dir.join("state.json"), b"{not json").unwrap();
        assert!(store.load_state().readings.is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn labels() {
        let mut s = Settings { account_name: Some("+15551234567".into()), ..Default::default() };
        assert_eq!(s.account_label(), "***4567");
        s.account_name = None;
        s.account_id = Some("11111111-2222-3333-4444-555555555555".into());
        assert_eq!(s.account_label(), "account ID");
        assert!(s.has_account());
    }
}
