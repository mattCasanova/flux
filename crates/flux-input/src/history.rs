//! Command history: a rolling buffer of previously submitted commands.
//!
//! Persisted as plain newline-separated entries in
//! `~/.local/share/flux/history`. Multi-line commands are escaped
//! (`\n` → `\\n`) so each disk line is one entry. On startup, history
//! is seeded from the shell's own history file (via `flux-shell`) so
//! the user starts with their full existing history.
//!
//! Duplicate-consecutive commands are suppressed. Empty commands are
//! dropped. On load, entries past `max_size` are truncated from the
//! front (oldest dropped first).

use std::fs::OpenOptions;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

pub const DEFAULT_MAX_SIZE: usize = 10_000;

pub struct CommandHistory {
    entries: Vec<String>,
    path: Option<PathBuf>,
    max_size: usize,
}

impl CommandHistory {
    /// In-memory-only history with no persistence. Used in tests and
    /// when `platform::data_dir()` fails.
    pub fn in_memory(max_size: usize) -> Self {
        Self {
            entries: Vec::new(),
            path: None,
            max_size,
        }
    }

    /// Load Flux history from disk, seeded with shell history entries
    /// that aren't already present. If the Flux history file doesn't
    /// exist yet (first run), the shell history becomes the initial set.
    pub fn load(path: PathBuf, max_size: usize, shell_entries: Vec<String>) -> Self {
        let mut entries = Vec::new();

        // Load existing Flux history from disk.
        match std::fs::File::open(&path) {
            Ok(file) => {
                for line in BufReader::new(file).lines().map_while(Result::ok) {
                    let command = line.replace("\\n", "\n");
                    if !command.is_empty() {
                        entries.push(command);
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                // First run — seed from shell history.
                entries = shell_entries
                    .into_iter()
                    .filter(|s| !s.trim().is_empty())
                    .collect();
            }
            Err(e) => {
                log::warn!("Failed to load history from {}: {}", path.display(), e);
                // Fall back to shell history on read failure too.
                entries = shell_entries
                    .into_iter()
                    .filter(|s| !s.trim().is_empty())
                    .collect();
            }
        }

        // Truncate oldest if over the cap.
        if entries.len() > max_size {
            let excess = entries.len() - max_size;
            entries.drain(..excess);
        }

        Self {
            entries,
            path: Some(path),
            max_size,
        }
    }

    /// Append a command to history. Empty commands and duplicates of
    /// the most recent entry are dropped. Writes through to the file
    /// immediately (append mode — no full rewrite).
    pub fn append(&mut self, command: String) {
        if command.trim().is_empty() {
            return;
        }
        if self.entries.last() == Some(&command) {
            return;
        }

        self.entries.push(command.clone());

        if self.entries.len() > self.max_size {
            let excess = self.entries.len() - self.max_size;
            self.entries.drain(..excess);
        }

        if let Some(path) = &self.path
            && let Err(e) = Self::write_append(path, &command)
        {
            log::warn!("Failed to append to history {}: {}", path.display(), e);
        }
    }

    fn write_append(path: &Path, command: &str) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        let escaped = command.replace('\n', "\\n");
        writeln!(file, "{}", escaped)?;
        Ok(())
    }

    pub fn get(&self, index: usize) -> Option<&str> {
        self.entries.get(index).map(|s| s.as_str())
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

impl Default for CommandHistory {
    fn default() -> Self {
        Self::in_memory(DEFAULT_MAX_SIZE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_memory_append_and_get() {
        let mut h = CommandHistory::in_memory(100);
        h.append("ls".into());
        h.append("cd /tmp".into());
        assert_eq!(h.len(), 2);
        assert_eq!(h.get(0), Some("ls"));
        assert_eq!(h.get(1), Some("cd /tmp"));
    }

    #[test]
    fn suppresses_duplicate_consecutive() {
        let mut h = CommandHistory::in_memory(100);
        h.append("ls".into());
        h.append("ls".into());
        assert_eq!(h.len(), 1);
    }

    #[test]
    fn allows_non_consecutive_duplicates() {
        let mut h = CommandHistory::in_memory(100);
        h.append("ls".into());
        h.append("cd /tmp".into());
        h.append("ls".into());
        assert_eq!(h.len(), 3);
    }

    #[test]
    fn suppresses_empty() {
        let mut h = CommandHistory::in_memory(100);
        h.append("".into());
        h.append("   ".into());
        assert_eq!(h.len(), 0);
    }

    #[test]
    fn rolling_cap() {
        let mut h = CommandHistory::in_memory(3);
        h.append("a".into());
        h.append("b".into());
        h.append("c".into());
        h.append("d".into());
        assert_eq!(h.len(), 3);
        assert_eq!(h.get(0), Some("b"));
        assert_eq!(h.get(2), Some("d"));
    }

    #[test]
    fn persists_and_reloads() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history");

        {
            let mut h = CommandHistory::load(path.clone(), 100, vec![]);
            h.append("one".into());
            h.append("two".into());
        }

        let h2 = CommandHistory::load(path, 100, vec![]);
        assert_eq!(h2.len(), 2);
        assert_eq!(h2.get(0), Some("one"));
        assert_eq!(h2.get(1), Some("two"));
    }

    #[test]
    fn multiline_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history");
        let cmd = "for f in *; do\n  echo $f\ndone";
        {
            let mut h = CommandHistory::load(path.clone(), 100, vec![]);
            h.append(cmd.into());
        }
        let h2 = CommandHistory::load(path, 100, vec![]);
        assert_eq!(h2.len(), 1);
        assert_eq!(h2.get(0), Some(cmd));
    }

    #[test]
    fn seeds_from_shell_on_first_run() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history");
        let shell_entries = vec!["ls".into(), "cd /tmp".into(), "git status".into()];

        let h = CommandHistory::load(path, 100, shell_entries);
        assert_eq!(h.len(), 3);
        assert_eq!(h.get(0), Some("ls"));
        assert_eq!(h.get(2), Some("git status"));
    }

    #[test]
    fn does_not_seed_if_flux_history_exists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("history");

        // Write a Flux history file first.
        {
            let mut h = CommandHistory::load(path.clone(), 100, vec![]);
            h.append("flux-cmd".into());
        }

        // Load again with shell entries — they should NOT appear.
        let h2 = CommandHistory::load(path, 100, vec!["shell-cmd".into()]);
        assert_eq!(h2.len(), 1);
        assert_eq!(h2.get(0), Some("flux-cmd"));
    }
}
