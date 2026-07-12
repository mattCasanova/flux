//! Platform abstraction for OS-specific paths and behavior.
//!
//! Each platform implements the Platform trait. The correct
//! implementation is selected at compile time via cfg.
//!
//! **Path ownership**: trait methods compute pure paths — no I/O, no
//! side effects. The free functions at the bottom of this file wrap the
//! trait methods with a `create_dir_all` call so the directory is
//! guaranteed to exist on first access. Named locations inside the base
//! directories (`themes_dir`, `history_file`, `crashes_dir`, …) also
//! live here so the on-disk layout is defined in exactly one place —
//! no other module may hardcode a path under a Flux directory.
//!
//! **XDG on macOS**: we deliberately use the Linux/XDG layout
//! (`~/.config`, `~/.local/share`, `~/.local/state`, `~/.cache`) on
//! macOS rather than `~/Library/Application Support` etc. This matches
//! the convention most CLI tools follow and makes dotfile sync across
//! macOS ↔ Linux machines simpler.

use std::path::{Path, PathBuf};

/// Platform-specific behavior.
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

/// Get the durable data directory (history, saved sessions).
pub fn data_dir() -> PathBuf {
    let dir = CurrentPlatform::data_dir();
    ensure_dir(&dir, "data");
    dir
}

/// Get the app state directory (logs, crash dumps).
pub fn state_dir() -> PathBuf {
    let dir = CurrentPlatform::state_dir();
    ensure_dir(&dir, "state");
    dir
}

/// Get the regenerable cache directory. F16 theme system is the
/// expected first consumer (compiled theme snapshots).
pub fn cache_dir() -> PathBuf {
    let dir = CurrentPlatform::cache_dir();
    ensure_dir(&dir, "cache");
    dir
}

// --- Named locations within the base directories (#57) ---
//
// All code that reads or writes a well-known file lives here, so the
// on-disk layout is defined in exactly one place:
//
//   config_dir/            config.toml, themes/, shell/
//   data_dir/              history, sessions/
//   state_dir/             crashes/ (rolling logs land here in F3)
//   cache_dir/             (reserved; compiled theme snapshots in F16)

/// Directory for user-supplied theme TOML files.
pub fn themes_dir() -> PathBuf {
    let dir = config_dir().join("themes");
    ensure_dir(&dir, "themes");
    dir
}

/// Directory the auto-injected shell integration scripts are written to.
pub fn shell_integration_dir() -> PathBuf {
    let dir = config_dir().join("shell");
    ensure_dir(&dir, "shell integration");
    dir
}

/// Path of the command history file (the file itself is created lazily
/// by `CommandHistory` on first append).
pub fn history_file() -> PathBuf {
    data_dir().join("history")
}

/// Directory for saved session state (v0.5 persistence).
pub fn sessions_dir() -> PathBuf {
    let dir = data_dir().join("sessions");
    ensure_dir(&dir, "sessions");
    dir
}

/// Directory for crash dumps (F3 writes here; rolling logs live next
/// to it in `state_dir`).
pub fn crashes_dir() -> PathBuf {
    let dir = state_dir().join("crashes");
    ensure_dir(&dir, "crashes");
    dir
}

/// Create the full on-disk directory tree. Called once at startup so a
/// first run leaves a complete, discoverable layout rather than dirs
/// appearing ad hoc as features first touch them.
pub fn ensure_layout() {
    themes_dir();
    shell_integration_dir();
    sessions_dir();
    crashes_dir();
    cache_dir();
}
