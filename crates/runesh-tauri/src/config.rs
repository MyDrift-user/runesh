//! TOML-based config file management for Tauri desktop apps.
//!
//! Stores config in the platform-appropriate config directory:
//! - Windows: `%APPDATA%/<app_name>/config.toml`
//! - Linux: `~/.config/<app_name>/config.toml`
//! - macOS: `~/Library/Application Support/<app_name>/config.toml`

use serde::{de::DeserializeOwned, Serialize};
use std::path::PathBuf;

/// Get the config directory for an app.
pub fn config_dir(app_name: &str) -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(app_name)
}

/// Get the full path to the config file.
pub fn config_path(app_name: &str) -> PathBuf {
    config_dir(app_name).join("config.toml")
}

/// Load a config from disk. Returns `None` if the file doesn't exist.
pub fn load_config<T: DeserializeOwned>(app_name: &str) -> Option<T> {
    let path = config_path(app_name);
    let content = std::fs::read_to_string(&path).ok()?;
    toml::from_str(&content).ok()
}

/// Save a config to disk. Creates the directory if needed.
pub fn save_config<T: Serialize>(app_name: &str, config: &T) -> Result<(), String> {
    let dir = config_dir(app_name);
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Failed to create config dir: {e}"))?;

    let content = toml::to_string_pretty(config)
        .map_err(|e| format!("Failed to serialize config: {e}"))?;

    std::fs::write(config_path(app_name), content)
        .map_err(|e| format!("Failed to write config: {e}"))?;

    Ok(())
}

/// Load a config, or create a default one if it doesn't exist.
pub fn load_or_create<T: DeserializeOwned + Serialize + Default>(app_name: &str) -> T {
    match load_config(app_name) {
        Some(config) => config,
        None => {
            let default = T::default();
            let _ = save_config(app_name, &default);
            default
        }
    }
}
