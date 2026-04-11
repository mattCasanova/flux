//! Input editor — the fixed prompt at the bottom of the screen.

/// Action produced by the editor in response to a key event.
pub enum InputAction {
    /// No action needed.
    None,
    /// Editor content changed — trigger redraw.
    Redraw,
    /// User hit Enter — send this command to the PTY.
    SendCommand(String),
    /// Forward these bytes directly to the PTY (raw mode, Ctrl+C, etc.)
    ForwardToPty(Vec<u8>),
    /// Navigate to previous history entry.
    HistoryPrev,
    /// Navigate to next history entry.
    HistoryNext,
}

/// The input editor buffer.
pub struct InputEditor {
    /// The text being typed.
    buffer: String,
    /// Cursor position (byte offset in buffer).
    cursor: usize,
    /// Prompt context — current working directory.
    cwd: String,
    /// Prompt context — git branch (if any).
    git_branch: Option<String>,
    /// Is raw mode active? (vim, ssh, fzf, etc.)
    raw_mode: bool,
}

impl InputEditor {
    pub fn new() -> Self {
        Self {
            buffer: String::new(),
            cursor: 0,
            cwd: String::from("~"),
            git_branch: None,
            raw_mode: false,
        }
    }

    /// Get the current buffer contents.
    pub fn buffer(&self) -> &str {
        &self.buffer
    }

    /// Get the cursor position.
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// Get the current working directory.
    pub fn cwd(&self) -> &str {
        &self.cwd
    }

    /// Get the git branch, if any.
    pub fn git_branch(&self) -> Option<&str> {
        self.git_branch.as_deref()
    }

    /// Set raw mode (bypass editor, forward keys to PTY).
    pub fn set_raw_mode(&mut self, raw: bool) {
        self.raw_mode = raw;
    }

    /// Is the editor in raw mode?
    pub fn is_raw_mode(&self) -> bool {
        self.raw_mode
    }

    /// Update the prompt context.
    pub fn set_cwd(&mut self, cwd: String) {
        self.cwd = cwd;
    }

    /// Update the git branch.
    pub fn set_git_branch(&mut self, branch: Option<String>) {
        self.git_branch = branch;
    }

    /// Set the buffer contents (for history navigation).
    pub fn set_buffer(&mut self, text: String) {
        self.cursor = text.len();
        self.buffer = text;
    }

    // TODO: Phase 1, Step 6
    // - handle_key() method that routes key events to actions
    // - Insert character at cursor
    // - Backspace, Delete
    // - Cursor movement (left, right, home, end)
    // - Enter → SendCommand
    // - Shift+Enter → insert newline
    // - Up/Down → HistoryPrev/HistoryNext
    // - Ctrl+C → ForwardToPty
    // - Raw mode → ForwardToPty for everything
}
