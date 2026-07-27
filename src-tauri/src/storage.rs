//! Atomic JSON persistence for settings and profiles.

use std::fs;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::error::AppResult;
use crate::models::{ProfileStore, Settings};

const SETTINGS_FILE: &str = "settings.json";
const PROFILES_FILE: &str = "profiles.json";

/// Read a JSON file, falling back to `T::default()` if the file is missing
/// or corrupt (never fails; corruption is logged to stderr).
pub fn read_json<T: DeserializeOwned + Default>(path: &Path) -> T {
    match fs::read(path) {
        Ok(bytes) => match serde_json::from_slice(&bytes) {
            Ok(value) => value,
            Err(e) => {
                eprintln!(
                    "[umbra] corrupt json in {}: {e}; falling back to defaults",
                    path.display()
                );
                T::default()
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => T::default(),
        Err(e) => {
            eprintln!(
                "[umbra] failed to read {}: {e}; falling back to defaults",
                path.display()
            );
            T::default()
        }
    }
}

/// Write JSON atomically: serialize to `<path>.tmp`, then rename over the
/// destination (atomic on the same volume). Creates parent directories.
pub fn write_json<T: Serialize>(path: &Path, value: &T) -> AppResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = tmp_path(path);
    let bytes = serde_json::to_vec_pretty(value)?;
    fs::write(&tmp, &bytes)?;
    match fs::rename(&tmp, path) {
        Ok(()) => Ok(()),
        Err(_) => {
            // Windows can refuse to replace a file another handle just touched;
            // remove the destination and retry once.
            let _ = fs::remove_file(path);
            match fs::rename(&tmp, path) {
                Ok(()) => Ok(()),
                Err(e) => {
                    let _ = fs::remove_file(&tmp);
                    Err(e.into())
                }
            }
        }
    }
}

fn tmp_path(path: &Path) -> PathBuf {
    let mut os = path.as_os_str().to_owned();
    os.push(".tmp");
    PathBuf::from(os)
}

pub fn settings_path(data_dir: &Path) -> PathBuf {
    data_dir.join(SETTINGS_FILE)
}

pub fn profiles_path(data_dir: &Path) -> PathBuf {
    data_dir.join(PROFILES_FILE)
}

pub fn load_settings(data_dir: &Path) -> Settings {
    read_json(&settings_path(data_dir))
}

pub fn save_settings(data_dir: &Path, settings: &Settings) -> AppResult<()> {
    write_json(&settings_path(data_dir), settings)
}

pub fn load_profiles(data_dir: &Path) -> ProfileStore {
    read_json(&profiles_path(data_dir))
}

pub fn save_profiles(data_dir: &Path, profiles: &ProfileStore) -> AppResult<()> {
    write_json(&profiles_path(data_dir), profiles)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("umbra-storage-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn missing_file_yields_default() {
        let dir = scratch_dir();
        let settings = load_settings(&dir);
        assert_eq!(settings.mixed_port, Settings::default().mixed_port);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn corrupt_file_yields_default() {
        let dir = scratch_dir();
        fs::write(settings_path(&dir), b"{ not json !!").unwrap();
        let settings = load_settings(&dir);
        assert_eq!(settings.language, Settings::default().language);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn settings_roundtrip() {
        let dir = scratch_dir();
        let mut settings = Settings::default();
        settings.mixed_port = 7777;
        settings.language = "en".into();
        save_settings(&dir, &settings).unwrap();
        let loaded = load_settings(&dir);
        assert_eq!(loaded.mixed_port, 7777);
        assert_eq!(loaded.language, "en");
        // no leftover tmp file
        assert!(!tmp_path(&settings_path(&dir)).exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn profiles_roundtrip_creates_parent_dirs() {
        let dir = scratch_dir().join("nested").join("deeper");
        let profiles = ProfileStore::default();
        save_profiles(&dir, &profiles).unwrap();
        assert!(profiles_path(&dir).exists());
        let loaded = load_profiles(&dir);
        assert_eq!(loaded.version, 1);
        fs::remove_dir_all(dir.parent().unwrap().parent().unwrap()).unwrap();
    }

    #[test]
    fn write_replaces_existing_file() {
        let dir = scratch_dir();
        let path = dir.join("x.json");
        write_json(&path, &serde_json::json!({"a": 1})).unwrap();
        write_json(&path, &serde_json::json!({"a": 2})).unwrap();
        let v: serde_json::Value = read_json(&path);
        assert_eq!(v["a"], 2);
        fs::remove_dir_all(&dir).unwrap();
    }
}
