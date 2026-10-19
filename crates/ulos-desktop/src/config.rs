//! Versioned, bounded persistence for user-facing Kit settings.
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
};

const MAX_CONFIG_BYTES: u64 = 64 * 1024;
const MAX_RATE: u64 = 1024 * 1024 * 1024;
const MAX_PEERS: u32 = 10_000;
const MAX_CONNECTIONS: u8 = 16;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigSnapshot {
    pub version: u32,
    pub theme: ThemePreference,
    pub default_directory: Option<PathBuf>,
    pub http_connections: u8,
    pub http_rate_limit: Option<u64>,
    pub torrent_download_limit: Option<u64>,
    pub torrent_upload_limit: Option<u64>,
    pub torrent_peer_limit: u32,
    pub dht_enabled: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemePreference {
    #[default]
    System,
    Light,
    Dark,
}

impl Default for ConfigSnapshot {
    fn default() -> Self {
        Self {
            version: 1,
            theme: ThemePreference::System,
            default_directory: None,
            http_connections: 4,
            http_rate_limit: None,
            torrent_download_limit: None,
            torrent_upload_limit: None,
            torrent_peer_limit: 100,
            dht_enabled: false,
        }
    }
}
impl ConfigSnapshot {
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1 {
            return Err("Unsupported settings version.".into());
        }
        if !(1..=MAX_CONNECTIONS).contains(&self.http_connections) {
            return Err("HTTP connections must be between 1 and 16.".into());
        }
        if self.torrent_peer_limit == 0 || self.torrent_peer_limit > MAX_PEERS {
            return Err("Torrent peers must be between 1 and 10,000.".into());
        }
        for (name, value) in [
            ("HTTP rate", self.http_rate_limit),
            ("Torrent download rate", self.torrent_download_limit),
            ("Torrent upload rate", self.torrent_upload_limit),
        ] {
            if value.is_some_and(|v| v == 0 || v > MAX_RATE) {
                return Err(format!("{name} must be between 1 and 1 GiB/s."));
            }
        }
        if let Some(path) = &self.default_directory
            && !path.is_absolute()
        {
            return Err("Default directory must be an absolute path.".into());
        }
        Ok(())
    }
}

/// Baseline bytes are compared exactly before Save. This is conflict detection,
/// not protection against hostile concurrent filesystem mutation.
#[derive(Clone, Debug)]
pub struct LoadedSettings {
    pub snapshot: ConfigSnapshot,
    pub path: Option<PathBuf>,
    pub error: Option<String>,
    original: Option<Vec<u8>>,
    writable: bool,
}
impl LoadedSettings {
    pub fn unavailable() -> Self {
        Self {
            snapshot: ConfigSnapshot::default(),
            path: None,
            error: Some("Settings location is unavailable.".into()),
            original: None,
            writable: false,
        }
    }
    pub fn can_save(&self) -> bool {
        self.writable && self.path.is_some()
    }
    pub fn save(&self, snapshot: ConfigSnapshot) -> Result<Self, String> {
        snapshot.validate()?;
        if !self.can_save() {
            return Err("Reload settings after correcting the settings file or location.".into());
        }
        if let Some(directory) = &snapshot.default_directory
            && !fs::metadata(directory).is_ok_and(|meta| meta.is_dir())
        {
            return Err("Default directory must be an existing directory.".into());
        }
        let path = self
            .path
            .as_ref()
            .ok_or("Settings location is unavailable.")?;
        let parent = path.parent().ok_or("Settings directory is unavailable.")?;
        fs::create_dir_all(parent)
            .map_err(|e| format!("Could not create settings directory: {e}"))?;
        let current = read_original(path)?;
        if current != self.original {
            return Err(
                "Settings changed externally. Close and reopen Settings after reloading Ulos."
                    .into(),
            );
        }
        let mut bytes = serde_json::to_vec_pretty(&snapshot)
            .map_err(|e| format!("Could not encode settings: {e}"))?;
        bytes.push(b'\n');
        use std::io::Write;
        let mut stage = tempfile::NamedTempFile::new_in(parent)
            .map_err(|e| format!("Could not stage settings: {e}"))?;
        stage
            .write_all(&bytes)
            .and_then(|_| stage.as_file().sync_all())
            .map_err(|e| format!("Could not write settings: {e}"))?;
        // Repeat the baseline check after staging; existing replacement is only
        // authorized by the explicit Save action, never by startup.
        if read_original(path)? != self.original {
            return Err("Settings changed externally; Save was cancelled.".into());
        }
        if self.original.is_none() {
            stage
                .persist_noclobber(path)
                .map_err(|e| format!("Could not publish settings: {}", e.error))?;
        } else {
            stage
                .persist(path)
                .map_err(|e| format!("Could not replace settings: {}", e.error))?;
        }
        Ok(Self {
            snapshot,
            path: Some(path.clone()),
            original: Some(bytes),
            error: None,
            writable: true,
        })
    }
}
fn read_original(path: &Path) -> Result<Option<Vec<u8>>, String> {
    let meta = match fs::symlink_metadata(path) {
        Ok(meta) => meta,
        Err(e) if e.kind() == ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("Could not inspect settings: {e}")),
    };
    if meta.file_type().is_symlink() || !meta.is_file() {
        return Err("Settings must be a regular file, not a symlink or directory.".into());
    }
    if meta.len() > MAX_CONFIG_BYTES {
        return Err("Settings exceed the 64 KiB limit.".into());
    }
    use std::io::Read;
    let mut bytes = Vec::new();
    fs::File::open(path)
        .and_then(|f| f.take(MAX_CONFIG_BYTES + 1).read_to_end(&mut bytes))
        .map_err(|e| format!("Could not read settings: {e}"))?;
    if bytes.len() as u64 > MAX_CONFIG_BYTES {
        return Err("Settings exceed the 64 KiB limit.".into());
    }
    Ok(Some(bytes))
}
pub fn load(path: PathBuf) -> LoadedSettings {
    let result = read_original(&path).and_then(|original| {
        let snapshot = match original.as_ref() {
            Some(bytes) => {
                serde_json::from_slice(bytes).map_err(|e| format!("Invalid settings: {e}"))?
            }
            None => ConfigSnapshot::default(),
        };
        snapshot.validate()?;
        Ok((snapshot, original))
    });
    match result {
        Ok((snapshot, original)) => LoadedSettings {
            snapshot,
            path: Some(path),
            original,
            error: None,
            writable: true,
        },
        Err(error) => LoadedSettings {
            snapshot: ConfigSnapshot::default(),
            path: Some(path),
            original: None,
            error: Some(error),
            writable: false,
        },
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    #[test]
    fn defaults_are_safe() {
        let c = ConfigSnapshot::default();
        assert!(c.validate().is_ok());
        assert!(!c.dht_enabled);
        let json = serde_json::to_string(&c).unwrap();
        assert!(!json.contains("clipboard"));
        assert!(!json.contains("allow_upload"));
    }
    #[test]
    fn validation_bounds() {
        let mut c = ConfigSnapshot {
            http_connections: 17,
            ..Default::default()
        };
        assert!(c.validate().is_err());
        c.http_connections = 4;
        c.default_directory = Some(PathBuf::from("relative"));
        assert!(c.validate().is_err());
        c.default_directory = None;
        c.torrent_upload_limit = Some(MAX_RATE + 1);
        assert!(c.validate().is_err());
        c.torrent_upload_limit = Some(0);
        assert!(c.validate().is_err());
    }
    #[test]
    fn roundtrip_and_conflicts() {
        let d = tempdir().unwrap();
        let p = d.path().join("ulos.conf");
        let baseline = load(p.clone());
        let saved = baseline.save(ConfigSnapshot::default()).unwrap();
        assert_eq!(load(p.clone()).snapshot, saved.snapshot);
        assert!(baseline.save(ConfigSnapshot::default()).is_err());
        fs::write(&p, "external edit").unwrap();
        assert!(saved.save(ConfigSnapshot::default()).is_err());
        assert_eq!(fs::read_to_string(p).unwrap(), "external edit");
    }
    #[test]
    fn malformed_unknown_and_oversize_are_read_only() {
        let d = tempdir().unwrap();
        let p = d.path().join("ulos.conf");
        for bytes in [b"{bad".to_vec(), vec![b'x'; 65537], {
            let mut v = serde_json::to_value(ConfigSnapshot::default()).unwrap();
            v["clipboard_enabled"] = true.into();
            serde_json::to_vec(&v).unwrap()
        }] {
            fs::write(&p, &bytes).unwrap();
            let loaded = load(p.clone());
            assert!(loaded.error.is_some());
            assert!(loaded.save(ConfigSnapshot::default()).is_err());
            assert_eq!(fs::read(&p).unwrap(), bytes);
        }
    }
    #[test]
    fn directories_and_missing_download_folder_rejected() {
        let d = tempdir().unwrap();
        assert!(load(d.path().to_owned()).error.is_some());
        let p = d.path().join("ulos.conf");
        let c = ConfigSnapshot {
            default_directory: Some(d.path().join("absent")),
            ..Default::default()
        };
        assert!(load(p.clone()).save(c).is_err());
        assert!(!p.exists());
    }
    #[cfg(unix)]
    #[test]
    fn symlinks_rejected() {
        let d = tempdir().unwrap();
        let target = d.path().join("target");
        fs::write(&target, "untouched").unwrap();
        let p = d.path().join("ulos.conf");
        std::os::unix::fs::symlink(&target, &p).unwrap();
        let c = load(p);
        assert!(!c.can_save());
        assert!(c.save(ConfigSnapshot::default()).is_err());
        assert_eq!(fs::read_to_string(target).unwrap(), "untouched");
    }
}
