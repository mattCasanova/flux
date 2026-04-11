//! Platform abstraction for OS-specific paths and behavior.
//!
//! Each platform implements the Platform trait. The correct
//! implementation is selected at compile time via cfg.

use std::path::PathBuf;

/// Platform-specific behavior.
pub trait Platform {
    /// Directory for user config files (config.toml, themes, etc.)
    fn config_dir() -> PathBuf;

    /// Directory for user data files (history, state, etc.)
    fn data_dir() -> PathBuf;
}

/// macOS platform.
#[cfg(target_os = "macos")]
pub struct CurrentPlatform;

#[cfg(target_os = "macos")]
impl Platform for CurrentPlatform {
    fn config_dir() -> PathBuf {
        // CLI tools on macOS conventionally use ~/.config
        dirs::home_dir().unwrap().join(".config/flux")
    }

    fn data_dir() -> PathBuf {
        dirs::home_dir().unwrap().join(".local/share/flux")
    }
}

/// Linux platform.
#[cfg(target_os = "linux")]
pub struct CurrentPlatform;

#[cfg(target_os = "linux")]
impl Platform for CurrentPlatform {
    fn config_dir() -> PathBuf {
        // XDG standard: $XDG_CONFIG_HOME or ~/.config
        dirs::config_dir()
            .unwrap_or_else(|| dirs::home_dir().unwrap().join(".config"))
            .join("flux")
    }

    fn data_dir() -> PathBuf {
        dirs::data_dir()
            .unwrap_or_else(|| dirs::home_dir().unwrap().join(".local/share"))
            .join("flux")
    }
}

/// Windows platform (not yet implemented — placeholder).
#[cfg(target_os = "windows")]
pub struct CurrentPlatform;

#[cfg(target_os = "windows")]
impl Platform for CurrentPlatform {
    fn config_dir() -> PathBuf {
        // %APPDATA%\flux
        dirs::config_dir().unwrap().join("flux")
    }

    fn data_dir() -> PathBuf {
        dirs::data_local_dir().unwrap().join("flux")
    }
}

/// Get the config directory for the current platform.
pub fn config_dir() -> PathBuf {
    CurrentPlatform::config_dir()
}

/// Get the data directory for the current platform.
pub fn data_dir() -> PathBuf {
    CurrentPlatform::data_dir()
}
