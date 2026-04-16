//! Inline path autocomplete — v1 of Flux's autocomplete system.
//!
//! Triggers after a known path-taking command (cd, ls, vim, etc.).
//! Data source: `std::fs::read_dir` of the shell's cwd (from OSC 7).
//! Filtering is prefix-match only — fuzzy matching via `nucleo` lands
//! in v0.3.

use std::fs;
use std::io;
use std::path::Path;

/// Commands that take path arguments — autocomplete triggers after
/// a space following one of these.
const PATH_COMMANDS: &[&str] = &[
    "cd", "ls", "cat", "less", "more", "head", "tail", "vim", "nvim", "nano", "emacs", "hx",
    "grep", "find", "rm", "cp", "mv", "touch", "mkdir", "rmdir", "open", "code", "subl",
];

/// Commands that only accept directories.
const DIR_ONLY_COMMANDS: &[&str] = &["cd"];

const MAX_VISIBLE: usize = 12;

#[derive(Debug, Clone)]
pub struct Candidate {
    pub name: String,
    pub kind: CandidateKind,
}

#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum CandidateKind {
    Directory,
    File,
    Symlink,
    Other,
}

#[derive(Default)]
pub struct Autocomplete {
    all_candidates: Vec<Candidate>,
    /// Indices into `all_candidates` that match the current prefix.
    visible: Vec<usize>,
    selected: usize,
    /// Byte offset in the editor buffer where the partial token starts.
    token_start: usize,
    prefix: String,
    active: bool,
}

impl Autocomplete {
    pub fn active(&self) -> bool {
        self.active
    }

    pub fn selected_index(&self) -> usize {
        self.selected
    }

    pub fn token_start(&self) -> usize {
        self.token_start
    }

    pub fn visible_candidates(&self) -> Vec<&Candidate> {
        self.visible
            .iter()
            .take(MAX_VISIBLE)
            .filter_map(|&i| self.all_candidates.get(i))
            .collect()
    }

    /// Check if autocomplete should trigger at the current cursor
    /// position. Returns `Some((token_start, command))` if yes.
    pub fn should_trigger(buffer: &str, cursor: usize) -> Option<(usize, String)> {
        let line_start = buffer[..cursor].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let line = &buffer[line_start..cursor];
        let first_token = line.split_whitespace().next()?;
        if !PATH_COMMANDS.contains(&first_token) {
            return None;
        }

        let partial_start = buffer[..cursor]
            .rfind(|c: char| c.is_whitespace())
            .map(|i| i + 1)
            .unwrap_or(line_start);

        // Still typing the command name — don't trigger.
        let first_token_end = line_start + first_token.len();
        if partial_start <= first_token_end {
            return None;
        }

        Some((partial_start, first_token.to_string()))
    }

    /// Populate candidates from `cwd` and filter by the current prefix.
    /// `command` is the trigger command (e.g., "cd") — used to decide
    /// whether to show only directories.
    ///
    /// If the partial token contains `/` (e.g., `src/lib`), the
    /// directory portion is resolved relative to `cwd` and we list
    /// that subdirectory instead. The prefix becomes just the filename
    /// part after the last `/`.
    pub fn trigger(
        &mut self,
        cwd: &Path,
        buffer: &str,
        cursor: usize,
        token_start: usize,
        command: &str,
    ) -> io::Result<()> {
        let dirs_only = DIR_ONLY_COMMANDS.contains(&command);
        let partial = &buffer[token_start..cursor];

        // Expand ~ to the user's home directory.
        let expanded;
        let partial = if partial.starts_with('~') {
            if let Some(home) = dirs::home_dir() {
                expanded = partial.replacen('~', &home.to_string_lossy(), 1);
                &expanded
            } else {
                partial
            }
        } else {
            partial
        };

        // Resolve subdirectory paths: "src/lib" → list "cwd/src", prefix "lib"
        let (list_dir, prefix) = if let Some(last_slash) = partial.rfind('/') {
            let dir_part = &partial[..=last_slash];
            let file_part = &partial[last_slash + 1..];
            let resolved = if std::path::Path::new(dir_part).is_absolute() {
                std::path::PathBuf::from(dir_part)
            } else {
                cwd.join(dir_part)
            };
            if resolved.is_dir() {
                (resolved, file_part.to_string())
            } else {
                // Path doesn't exist — no candidates.
                self.active = false;
                return Ok(());
            }
        } else {
            (cwd.to_path_buf(), partial.to_string())
        };

        self.all_candidates = list_directory(&list_dir, dirs_only)?;
        self.token_start = token_start;
        self.prefix = prefix;
        self.selected = 0;
        self.recompute_visible();
        self.active = !self.visible.is_empty();
        Ok(())
    }

    /// Update the filter after a keystroke. Returns `false` if the
    /// popup should dismiss (no matches or backspaced past token start).
    pub fn update_filter(&mut self, buffer: &str, cursor: usize) -> bool {
        if !self.active {
            return false;
        }
        if cursor < self.token_start {
            self.dismiss();
            return false;
        }
        let partial = &buffer[self.token_start..cursor];
        // Use only the part after the last `/` as the prefix filter,
        // since candidates are filenames within the resolved directory.
        self.prefix = partial.rfind('/').map_or(partial, |i| &partial[i + 1..]).to_string();
        self.recompute_visible();
        if self.visible.is_empty() {
            self.dismiss();
            return false;
        }
        if self.selected >= self.visible.len().min(MAX_VISIBLE) {
            self.selected = self.visible.len().min(MAX_VISIBLE) - 1;
        }
        true
    }

    fn recompute_visible(&mut self) {
        self.visible.clear();
        let prefix_lower = self.prefix.to_lowercase();
        for (i, cand) in self.all_candidates.iter().enumerate() {
            if cand.name.to_lowercase().starts_with(&prefix_lower) {
                self.visible.push(i);
            }
        }
    }

    pub fn select_next(&mut self) {
        if !self.active {
            return;
        }
        let max = self.visible.len().min(MAX_VISIBLE);
        if self.selected + 1 < max {
            self.selected += 1;
        }
    }

    pub fn select_prev(&mut self) {
        if !self.active {
            return;
        }
        self.selected = self.selected.saturating_sub(1);
    }

    /// Return `(replace_start, replacement)` for the selected candidate.
    /// `replace_start` is the byte offset where the replacement begins —
    /// after the last `/` in the partial token so the directory path is
    /// preserved. Directories get a trailing `/`.
    pub fn commit(&self, buffer: &str, cursor: usize) -> Option<(usize, String)> {
        if !self.active {
            return None;
        }
        let cand = self.visible.get(self.selected).and_then(|&i| self.all_candidates.get(i))?;
        let replacement = match cand.kind {
            CandidateKind::Directory => format!("{}/", cand.name),
            _ => cand.name.clone(),
        };

        // Only replace the filename portion — preserve the directory path.
        let partial = &buffer[self.token_start..cursor];
        let replace_start = partial
            .rfind('/')
            .map(|i| self.token_start + i + 1)
            .unwrap_or(self.token_start);

        Some((replace_start, replacement))
    }

    pub fn dismiss(&mut self) {
        self.active = false;
        self.all_candidates.clear();
        self.visible.clear();
        self.selected = 0;
        self.prefix.clear();
    }
}

/// List directory entries, sorted: directories first, then files,
/// then symlinks — alphabetical within each group.
/// Hidden files (dotfiles) are excluded unless the user's prefix
/// starts with `.`. `dirs_only` filters to directories when true.
fn list_directory(cwd: &Path, dirs_only: bool) -> io::Result<Vec<Candidate>> {
    let mut candidates = Vec::new();
    for entry in fs::read_dir(cwd)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        // Skip dotfiles — they'll be included when the prefix filter
        // starts with '.' (handled in recompute_visible).
        if name.starts_with('.') {
            continue;
        }
        let metadata = entry.metadata().ok();
        let kind = match metadata {
            Some(m) if m.is_dir() => CandidateKind::Directory,
            Some(m) if m.is_symlink() => CandidateKind::Symlink,
            Some(m) if m.is_file() => CandidateKind::File,
            _ => CandidateKind::Other,
        };
        if dirs_only && kind != CandidateKind::Directory {
            continue;
        }
        candidates.push(Candidate { name, kind });
    }
    candidates.sort_by(|a, b| {
        let rank = |k: CandidateKind| match k {
            CandidateKind::Directory => 0,
            CandidateKind::File => 1,
            CandidateKind::Symlink => 2,
            CandidateKind::Other => 3,
        };
        rank(a.kind).cmp(&rank(b.kind)).then_with(|| a.name.cmp(&b.name))
    });
    Ok(candidates)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trigger_on_cd_space() {
        let result = Autocomplete::should_trigger("cd ", 3);
        assert_eq!(result.as_ref().map(|(s, _)| *s), Some(3));
        assert_eq!(result.as_ref().map(|(_, c)| c.as_str()), Some("cd"));
    }

    #[test]
    fn trigger_on_cd_partial() {
        let result = Autocomplete::should_trigger("cd /tm", 6);
        assert_eq!(result.as_ref().map(|(s, _)| *s), Some(3));
    }

    #[test]
    fn no_trigger_while_typing_command() {
        assert_eq!(Autocomplete::should_trigger("cd", 2), None);
    }

    #[test]
    fn no_trigger_on_unknown_command() {
        assert_eq!(Autocomplete::should_trigger("foobar /t", 9), None);
    }

    #[test]
    fn trigger_on_ls_with_flags() {
        let result = Autocomplete::should_trigger("ls -la /tm", 10);
        assert_eq!(result.as_ref().map(|(s, _)| *s), Some(7));
    }

    #[test]
    fn cd_shows_only_directories() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("subdir")).unwrap();
        std::fs::write(dir.path().join("file.txt"), "").unwrap();

        let mut ac = Autocomplete::default();
        ac.trigger(dir.path(), "cd ", 3, 3, "cd").unwrap();

        assert!(ac.active());
        let visible = ac.visible_candidates();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].name, "subdir");
        assert_eq!(visible[0].kind, CandidateKind::Directory);
    }

    #[test]
    fn ls_shows_files_and_directories() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("subdir")).unwrap();
        std::fs::write(dir.path().join("file.txt"), "").unwrap();

        let mut ac = Autocomplete::default();
        ac.trigger(dir.path(), "ls ", 3, 3, "ls").unwrap();

        assert!(ac.active());
        let visible = ac.visible_candidates();
        assert_eq!(visible.len(), 2);
        assert_eq!(visible[0].name, "subdir");
        assert_eq!(visible[1].name, "file.txt");
    }

    #[test]
    fn hides_dotfiles() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".hidden")).unwrap();
        std::fs::write(dir.path().join("visible.txt"), "").unwrap();

        let mut ac = Autocomplete::default();
        ac.trigger(dir.path(), "ls ", 3, 3, "ls").unwrap();

        let visible = ac.visible_candidates();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].name, "visible.txt");
    }

    #[test]
    fn commit_appends_slash_for_dirs() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("mydir")).unwrap();

        let mut ac = Autocomplete::default();
        ac.trigger(dir.path(), "cd ", 3, 3, "cd").unwrap();
        let (start, result) = ac.commit("cd ", 3).unwrap();
        assert_eq!(start, 3);
        assert_eq!(result, "mydir/");
    }

    #[test]
    fn filter_narrows_candidates() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("alpha")).unwrap();
        std::fs::create_dir(dir.path().join("beta")).unwrap();
        std::fs::write(dir.path().join("gamma.txt"), "").unwrap();

        let mut ac = Autocomplete::default();
        ac.trigger(dir.path(), "ls ", 3, 3, "ls").unwrap();
        assert_eq!(ac.visible_candidates().len(), 3);

        // Simulate typing "al"
        ac.update_filter("ls al", 5);
        let visible = ac.visible_candidates();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].name, "alpha");
    }
}
