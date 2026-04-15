//! Platform abstraction for OS-specific paths and behavior.
//!
//! Each platform implements the Platform trait. The correct
//! implementation is selected at compile time via cfg.
//!
//! **Path ownership**: trait methods compute pure paths — no I/O, no
//! side effects. The free functions at the bottom of this file
//! (`config_dir`, `data_dir`, `state_dir`, `cache_dir`) wrap the trait
//! methods with a `create_dir_all` call so the directory is guaranteed
//! to exist on first access. Callers that write files into these
//! directories no longer need to pre-create parents themselves.
//!
//! **XDG on macOS**: we deliberately use the Linux/XDG layout
//! (`~/.config`, `~/.local/share`, `~/.local/state`, `~/.cache`) on
//! macOS rather than `~/Library/Application Support` etc. This matches
//! the convention most CLI tools follow and makes dotfile sync across
//! macOS ↔ Linux machines simpler.

use std::path::{Path, PathBuf};

/// Platform-specific behavior.
#[allow(dead_code)] // data_dir/state_dir/cache_dir are foundation for F1/F3/F5; F5 history is first caller
pub trait Platform {
    /// Directory for user config files (`config.toml`, themes, etc.)
    fn config_dir() -> PathBuf;

    /// Directory for durable user data (command history, saved
    /// sessions, persistent state that survives a crash).
    fn data_dir() -> PathBuf;

    /// Directory for transient app state (logs, crash dumps, PID
    /// files). Distinct from `data_dir` — state is recoverable
    /// metadata, data is user content.
    fn state_dir() -> PathBuf;

    /// Directory for regenerable caches (compiled themes, glyph
    /// snapshots, anything that can be rebuilt from scratch).
    fn cache_dir() -> PathBuf;
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

    fn state_dir() -> PathBuf {
        // No XDG_STATE_HOME equivalent on macOS; mirror the Linux
        // convention so logs live alongside the config tree.
        dirs::home_dir().unwrap().join(".local/state/flux")
    }

    fn cache_dir() -> PathBuf {
        dirs::home_dir().unwrap().join(".cache/flux")
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

    fn state_dir() -> PathBuf {
        // XDG_STATE_HOME or ~/.local/state
        dirs::state_dir()
            .unwrap_or_else(|| dirs::home_dir().unwrap().join(".local/state"))
            .join("flux")
    }

    fn cache_dir() -> PathBuf {
        dirs::cache_dir()
            .unwrap_or_else(|| dirs::home_dir().unwrap().join(".cache"))
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

    fn state_dir() -> PathBuf {
        // Windows has no XDG state equivalent; use %LOCALAPPDATA%\flux\state
        // so logs stay local to the machine rather than roaming profiles.
        dirs::data_local_dir().unwrap().join("flux").join("state")
    }

    fn cache_dir() -> PathBuf {
        // %LOCALAPPDATA%\flux\cache
        dirs::cache_dir().unwrap().join("flux")
    }
}

/// Ensure the directory exists on disk. Logs a warning (not an error)
/// if creation fails — the caller will hit a clearer error message
/// when it tries to read or write the first file, and that's a better
/// place to handle the problem than the path-resolution helper.
fn ensure_dir(dir: &Path, label: &'static str) {
    if let Err(e) = std::fs::create_dir_all(dir) {
        log::warn!(
            "failed to create {} directory at {}: {}",
            label,
            dir.display(),
            e
        );
    }
}

/// Get the config directory for the current platform, creating it if
/// necessary.
pub fn config_dir() -> PathBuf {
    let dir = CurrentPlatform::config_dir();
    ensure_dir(&dir, "config");
    dir
}

/// Get the durable data directory (history, saved sessions). F5
/// command history is the first consumer.
#[allow(dead_code)] // consumed by F5 history (#21)
pub fn data_dir() -> PathBuf {
    let dir = CurrentPlatform::data_dir();
    ensure_dir(&dir, "data");
    dir
}

/// Get the app state directory (logs, crash dumps). F3 crash dumps is
/// the first consumer.
#[allow(dead_code)] // consumed by F3 crash dumps + rolling logs (#48)
pub fn state_dir() -> PathBuf {
    let dir = CurrentPlatform::state_dir();
    ensure_dir(&dir, "state");
    dir
}

/// Get the regenerable cache directory. F16 theme system is the
/// expected first consumer (compiled theme snapshots).
#[allow(dead_code)] // consumed by F16 theme system
pub fn cache_dir() -> PathBuf {
    let dir = CurrentPlatform::cache_dir();
    ensure_dir(&dir, "cache");
    dir
}
