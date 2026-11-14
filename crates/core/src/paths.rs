//! Well-known locations. Uses environment variables only (no `dirs` crate).

use std::path::PathBuf;

const APP_DIR: &str = "quota";

fn env_dir(var: &str) -> Option<PathBuf> {
    std::env::var_os(var).filter(|v| !v.is_empty()).map(PathBuf::from)
}

/// `%USERPROFILE%` (falls back to `%HOME%`).
pub fn home() -> Option<PathBuf> {
    env_dir("USERPROFILE").or_else(|| env_dir("HOME"))
}

/// `%APPDATA%` (Roaming).
pub fn roaming() -> Option<PathBuf> {
    env_dir("APPDATA")
}

/// `%LOCALAPPDATA%`.
pub fn local() -> Option<PathBuf> {
    env_dir("LOCALAPPDATA")
}

/// `%LOCALAPPDATA%\quota` — snapshot and notification state.
/// `QUOTA_DATA_DIR` overrides it.
pub fn data_dir() -> Option<PathBuf> {
    env_dir("QUOTA_DATA_DIR").or_else(|| local().map(|p| p.join(APP_DIR)))
}

/// `%APPDATA%\quota` — config. `QUOTA_CONFIG_DIR` overrides it.
pub fn config_dir() -> Option<PathBuf> {
    env_dir("QUOTA_CONFIG_DIR").or_else(|| roaming().map(|p| p.join(APP_DIR)))
}

pub fn snapshot_file() -> Option<PathBuf> {
    data_dir().map(|d| d.join("snapshot.json"))
}

pub fn notified_file() -> Option<PathBuf> {
    data_dir().map(|d| d.join("notified.json"))
}

pub fn config_file() -> Option<PathBuf> {
    config_dir().map(|d| d.join("config.toml"))
}

