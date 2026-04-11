//! Shell abstraction layer for Flux.
//!
//! All shell-specific logic lives behind the [Shell] trait. The rest
//! of Flux interacts only with this interface. Adding a new shell
//! means implementing the trait — nothing else changes.
//!
//! ## Supported Shells
//!
//! - [zsh] — Zsh (default on macOS)
//! - [bash] — Bash
//! - [fish] — Fish

mod zsh;
mod bash;
mod fish;
mod history;

use std::path::{Path, PathBuf};

/// How to inject the shell integration script on startup.
pub enum InjectionMethod {
    /// Source a file in the shell's rc file
    RcFile {
        rc_path: PathBuf,
        source_line: String,
    },
    /// Set an environment variable
    EnvVar {
        key: String,
        value: String,
    },
}

/// Shell abstraction — all shell-specific behavior behind one interface.
pub trait Shell {
    /// Shell binary path (e.g., "/bin/zsh")
    fn binary(&self) -> &Path;

    /// Shell name for display
    fn name(&self) -> &str;

    /// Path to the shell's history file
    fn history_file(&self) -> PathBuf;

    /// Parse a raw history file line into a command string.
    fn parse_history_entry(&self, raw_line: &str) -> Option<String>;

    /// Load all history entries (parsed, most recent last).
    fn load_history(&self) -> Vec<String> {
        let path = self.history_file();
        if !path.exists() {
            return vec![];
        }
        std::fs::read_to_string(&path)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| self.parse_history_entry(line))
            .collect()
    }

    /// Shell integration script content (OSC 133 hooks).
    fn integration_script(&self) -> &str;

    /// How to inject the integration script.
    fn injection_method(&self) -> InjectionMethod;

    /// RC file paths sourced on startup.
    fn rc_files(&self) -> Vec<PathBuf>;

    /// Command-line arguments for spawning.
    fn spawn_args(&self) -> Vec<String>;
}

/// Detect the user's shell and return the appropriate implementation.
pub fn detect_shell() -> Box<dyn Shell> {
    if let Ok(shell_path) = std::env::var("SHELL") {
        let name = Path::new(&shell_path)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("");

        match name {
            "zsh" => return Box::new(zsh::Zsh::new(PathBuf::from(&shell_path))),
            "bash" => return Box::new(bash::Bash::new(PathBuf::from(&shell_path))),
            "fish" => return Box::new(fish::Fish::new(PathBuf::from(&shell_path))),
            _ => log::warn!("Unknown shell '{}', falling back to zsh", name),
        }
    }

    // Default to zsh on macOS
    Box::new(zsh::Zsh::new(PathBuf::from("/bin/zsh")))
}
