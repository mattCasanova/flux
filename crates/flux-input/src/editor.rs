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

// --- Multi-line support (R5) ---
//
// The buffer holds embedded `\n` characters. `cursor` remains a byte
// offset. Existing single-line methods (`insert_str`, `backspace`, etc.)
// already handle newlines correctly because they don't special-case
// line boundaries. These new methods are the line-aware helpers F6
// will need when it wires up Shift+Enter / arrow-up / arrow-down
// handling. Nothing in R5 calls them — they're pure additive data-
// model scaffolding.
//
// Column math uses `.chars().count()`, which is right for Latin text
// and wrong for wide-character CJK (a CJK glyph is one `char` but
// two cells). Text shaping in #20 will fix that; for now all of
// multi-line editing treats columns as character counts.

impl InputEditor {
    /// Number of lines in the buffer. A buffer without any `\n` is
    /// one line; each `\n` adds another. Always at least 1.
    pub fn line_count(&self) -> usize {
        self.buffer.matches('\n').count() + 1
    }

    /// Zero-based line index containing the cursor.
    pub fn cursor_line(&self) -> usize {
        self.buffer[..self.cursor].matches('\n').count()
    }

    /// Character-count column within the cursor's line, measured
    /// from the first character after the preceding `\n` (or the
    /// buffer start on line 0).
    pub fn cursor_col_in_line(&self) -> usize {
        let line_start = self.current_line_start();
        self.buffer[line_start..self.cursor].chars().count()
    }

    /// Move the cursor to the first character of the current line.
    /// On a single-line buffer this is identical to `home()`.
    pub fn home_line(&mut self) {
        self.cursor = self.current_line_start();
    }

    /// Move the cursor to the last character of the current line —
    /// just before the trailing `\n`, or the buffer end on the
    /// final line.
    pub fn end_line(&mut self) {
        let rel = self.buffer[self.cursor..]
            .find('\n')
            .unwrap_or(self.buffer.len() - self.cursor);
        self.cursor += rel;
    }

    /// Insert a newline at the cursor position. Kept as its own
    /// method so F6 can bind Shift+Enter (or similar) to it
    /// explicitly rather than reaching into `insert_str("\n")`.
    pub fn insert_newline(&mut self) {
        self.insert_str("\n");
    }

    /// Move the cursor up one line, preserving the column where
    /// possible (clamped to the length of the target line).
    /// Returns `false` if already on line 0.
    pub fn move_up(&mut self) -> bool {
        let current_start = self.current_line_start();
        if current_start == 0 {
            return false; // already on the first line
        }
        let col = self.buffer[current_start..self.cursor].chars().count();

        // Previous line starts right after the newline before
        // `current_start - 1` (or the buffer start).
        let prev_line_end = current_start - 1; // the '\n' byte
        let prev_line_start = self.buffer[..prev_line_end]
            .rfind('\n')
            .map(|i| i + 1)
            .unwrap_or(0);
        let prev_line = &self.buffer[prev_line_start..prev_line_end];

        // Walk `col` chars into the previous line, or stop at its end.
        let target_offset = prev_line
            .char_indices()
            .nth(col)
            .map(|(i, _)| i)
            .unwrap_or(prev_line.len());
        self.cursor = prev_line_start + target_offset;
        true
    }

    /// Move the cursor down one line, preserving the column where
    /// possible (clamped to the length of the target line).
    /// Returns `false` if already on the last line.
    pub fn move_down(&mut self) -> bool {
        let current_start = self.current_line_start();
        let col = self.buffer[current_start..self.cursor].chars().count();

        // Next line starts right after the next '\n' from the cursor.
        let next_nl = self.buffer[self.cursor..]
            .find('\n')
            .map(|rel| self.cursor + rel);
        let Some(next_nl) = next_nl else {
            return false; // already on the last line
        };
        let next_line_start = next_nl + 1;

        // Next line ends at the following '\n' or the buffer end.
        let next_line_end = self.buffer[next_line_start..]
            .find('\n')
            .map(|rel| next_line_start + rel)
            .unwrap_or(self.buffer.len());
        let next_line = &self.buffer[next_line_start..next_line_end];

        let target_offset = next_line
            .char_indices()
            .nth(col)
            .map(|(i, _)| i)
            .unwrap_or(next_line.len());
        self.cursor = next_line_start + target_offset;
        true
    }

    /// Byte offset of the first character on the cursor's current
    /// line.
    fn current_line_start(&self) -> usize {
        self.buffer[..self.cursor]
            .rfind('\n')
            .map(|i| i + 1)
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_count_single_line() {
        let e = InputEditor::new();
        assert_eq!(e.line_count(), 1);
    }

    #[test]
    fn line_count_three_lines() {
        let mut e = InputEditor::new();
        e.insert_str("a\nb\nc");
        assert_eq!(e.line_count(), 3);
    }

    #[test]
    fn cursor_line_tracks_newlines() {
        let mut e = InputEditor::new();
        e.insert_str("foo\nbar\nbaz");
        // Cursor at end of buffer, which is line 2
        assert_eq!(e.cursor_line(), 2);
        e.home();
        assert_eq!(e.cursor_line(), 0);
    }

    #[test]
    fn move_up_preserves_column() {
        let mut e = InputEditor::new();
        e.insert_str("hello\nworld");
        // Cursor at end of "world" — col 5, line 1
        assert_eq!(e.cursor_line(), 1);
        assert_eq!(e.cursor_col_in_line(), 5);
        assert!(e.move_up());
        assert_eq!(e.cursor_line(), 0);
        assert_eq!(e.cursor_col_in_line(), 5);
    }

    #[test]
    fn move_up_clamps_to_shorter_line() {
        let mut e = InputEditor::new();
        e.insert_str("hi\nworld");
        // Cursor at end of "world" — col 5, line 1
        assert!(e.move_up());
        // "hi" is only 2 chars, so cursor clamps to end of "hi"
        assert_eq!(e.cursor_line(), 0);
        assert_eq!(e.cursor_col_in_line(), 2);
    }

    #[test]
    fn move_up_returns_false_on_first_line() {
        let mut e = InputEditor::new();
        e.insert_str("hello");
        assert!(!e.move_up());
    }

    #[test]
    fn move_down_preserves_column_and_reports_false_at_end() {
        let mut e = InputEditor::new();
        e.insert_str("foo\nbar\nbaz");
        e.home(); // back to (0, 0)
        assert!(e.move_down());
        assert_eq!(e.cursor_line(), 1);
        assert_eq!(e.cursor_col_in_line(), 0);
        assert!(e.move_down());
        assert_eq!(e.cursor_line(), 2);
        // Last line — can't go further
        assert!(!e.move_down());
    }

    #[test]
    fn home_line_and_end_line_in_middle_of_multi_line() {
        let mut e = InputEditor::new();
        e.insert_str("first\nsecond\nthird");
        // Cursor at end of "third" (col 5). Move up preserves column,
        // so we land at col 5 inside "second" (which is 6 chars long
        // and easily accommodates col 5).
        e.move_up();
        assert_eq!(e.cursor_line(), 1);
        assert_eq!(e.cursor_col_in_line(), 5);
        // home_line jumps to the start of "second"
        e.home_line();
        assert_eq!(e.cursor_col_in_line(), 0);
        // end_line jumps to the end of "second" (col 6)
        e.end_line();
        assert_eq!(e.cursor_col_in_line(), 6);
    }

    #[test]
    fn insert_newline_splits_buffer() {
        let mut e = InputEditor::new();
        e.insert_str("hello");
        e.insert_newline();
        e.insert_str("world");
        assert_eq!(e.line_count(), 2);
        assert_eq!(e.buffer(), "hello\nworld");
    }
}
