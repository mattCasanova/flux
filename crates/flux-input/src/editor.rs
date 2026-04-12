//! Input editor — a single-line text buffer driven by the app.
//!
//! The app owns key dispatch; this type is just a buffer with fine-grained
//! edit operations. On Enter, the app calls [`InputEditor::take_line`] to
//! extract the composed command and forward it to the PTY.

/// A single-line editable text buffer.
///
/// The cursor is tracked as a byte offset into `buffer` and is always kept
/// on a character boundary. All mutating methods leave the buffer in a
/// valid UTF-8 state.
#[derive(Default)]
pub struct InputEditor {
    buffer: String,
    /// Cursor position as a byte offset into `buffer`.
    cursor: usize,
}

impl InputEditor {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn buffer(&self) -> &str {
        &self.buffer
    }

    /// Cursor position as a displayed column (character count before the cursor).
    /// Wide-character support (CJK) will land with text shaping (#20).
    pub fn cursor_col(&self) -> usize {
        self.buffer[..self.cursor].chars().count()
    }

    /// Insert text at the cursor position.
    pub fn insert_str(&mut self, text: &str) {
        self.buffer.insert_str(self.cursor, text);
        self.cursor += text.len();
    }

    /// Delete the character before the cursor.
    pub fn backspace(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let prev = self.prev_char_boundary();
        self.buffer.replace_range(prev..self.cursor, "");
        self.cursor = prev;
    }

    /// Delete the character at the cursor.
    pub fn delete_forward(&mut self) {
        if self.cursor >= self.buffer.len() {
            return;
        }
        let next = self.next_char_boundary();
        self.buffer.replace_range(self.cursor..next, "");
    }

    pub fn move_left(&mut self) {
        self.cursor = self.prev_char_boundary();
    }

    pub fn move_right(&mut self) {
        self.cursor = self.next_char_boundary();
    }

    pub fn home(&mut self) {
        self.cursor = 0;
    }

    pub fn end(&mut self) {
        self.cursor = self.buffer.len();
    }

    /// Clear the buffer without returning its contents. Used by Ctrl+C.
    pub fn clear(&mut self) {
        self.buffer.clear();
        self.cursor = 0;
    }

    /// Extract the buffer and reset the editor. The returned string is the
    /// composed command — the caller forwards it to the PTY with a trailing `\r`.
    pub fn take_line(&mut self) -> String {
        self.cursor = 0;
        std::mem::take(&mut self.buffer)
    }

    fn prev_char_boundary(&self) -> usize {
        self.buffer[..self.cursor]
            .char_indices()
            .last()
            .map(|(i, _)| i)
            .unwrap_or(0)
    }

    fn next_char_boundary(&self) -> usize {
        self.buffer[self.cursor..]
            .char_indices()
            .nth(1)
            .map(|(i, _)| self.cursor + i)
            .unwrap_or(self.buffer.len())
    }
}
