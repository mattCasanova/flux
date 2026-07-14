//! Keyboard event routing.
//!
//! `handle_keyboard` is the top-level entry point. Order of operations:
//!
//! 1. Drop key releases (only Pressed events do anything).
//! 2. **Popup intercept** — if autocomplete (or future search) is
//!    active, give it first refusal on the key.
//! 3. Clipboard shortcut short-circuit — paste must work in both
//!    raw and cooked mode, so it runs before the mode split.
//! 4. Branch to `handle_keyboard_raw` (PTY-first) or
//!    `handle_keyboard_cooked` (editor-first) based on `raw_mode`.

use flux_input::Autocomplete;

use super::{App, PopupState};

impl App {
    pub(super) fn handle_keyboard(&mut self, event: winit::event::KeyEvent) {
        use winit::event::ElementState;

        if event.state != ElementState::Pressed {
            return;
        }

        // Popup intercept — exhaustive match, no wildcard.
        match &self.popup {
            PopupState::Hidden => {}
            PopupState::Autocomplete => {
                if self.handle_autocomplete_key(&event) {
                    return;
                }
            }
        }

        if self.is_paste_shortcut(&event) {
            self.handle_paste();
            return;
        }

        if self.is_copy_shortcut(&event) && self.handle_copy() {
            return;
        }

        if self.pty_owns_keyboard() {
            self.handle_keyboard_raw(event);
            return;
        }

        self.handle_keyboard_cooked(event);
    }

    /// True when keystrokes belong to the PTY rather than the Flux
    /// editor: alt-screen programs (vim, Claude Code), and any command
    /// still executing per OSC 133 phase (sudo password prompts, REPLs,
    /// and interactive programs that never touch the alt screen). At the
    /// shell prompt — integration phase Prompt/Input, or no integration
    /// at all — the Flux editor owns the keyboard.
    fn pty_owns_keyboard(&self) -> bool {
        self.raw_mode
            || self
                .terminal
                .as_ref()
                .map(|t| t.is_executing())
                .unwrap_or(false)
    }

    /// Handle a key while the autocomplete popup is active. Returns
    /// `true` if the key was consumed (caller should not process it
    /// further).
    fn handle_autocomplete_key(&mut self, event: &winit::event::KeyEvent) -> bool {
        use winit::keyboard::{Key, NamedKey};

        if !self.autocomplete.active() {
            return false;
        }

        match &event.logical_key {
            Key::Named(NamedKey::ArrowUp) => {
                self.autocomplete.select_prev();
                self.update_input_display();
                self.request_redraw();
                true
            }
            Key::Named(NamedKey::ArrowDown) => {
                self.autocomplete.select_next();
                self.update_input_display();
                self.request_redraw();
                true
            }
            Key::Named(NamedKey::Tab) | Key::Named(NamedKey::Enter) => {
                let cursor = self.input.cursor();
                let committed_dir = if let Some((replace_start, replacement)) =
                    self.autocomplete.commit(self.input.buffer(), cursor)
                {
                    let is_dir = replacement.ends_with('/');
                    self.input
                        .replace_range(replace_start, cursor, &replacement);
                    is_dir
                } else {
                    false
                };
                self.autocomplete.dismiss();
                self.popup = PopupState::Hidden;
                self.update_input_display();
                self.request_redraw();
                // Only re-trigger if we committed a directory — chaining
                // into subdirectories. Files are a leaf; the user hits
                // Enter again to run the command.
                if committed_dir {
                    self.maybe_update_autocomplete();
                }
                true
            }
            Key::Named(NamedKey::Escape) => {
                self.autocomplete.dismiss();
                self.popup = PopupState::Hidden;
                self.update_input_display();
                self.request_redraw();
                true
            }
            _ => {
                // Let the key fall through to normal text input.
                // After the text is inserted, handle_keyboard_cooked
                // calls maybe_update_autocomplete to re-filter.
                false
            }
        }
    }

    /// Cooked-mode key handling — keystrokes go through the Flux input editor,
    /// Enter submits the composed line, Ctrl+<letter> bypasses the editor.
    fn handle_keyboard_cooked(&mut self, event: winit::event::KeyEvent) {
        use winit::keyboard::{Key, NamedKey};
        use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;

        match &event.logical_key {
            // Shift+Enter inserts a newline; Enter submits the buffer.
            Key::Named(NamedKey::Enter) => {
                if self.modifiers.shift_key() {
                    self.input.insert_newline();
                } else {
                    // Submitting a command returns the viewport to the
                    // live tail — standard "typing brings you back".
                    self.snap_to_bottom();
                    self.clear_selection();
                    let line = self.input.take_line();
                    if let Some(pty) = &mut self.pty {
                        let _ = pty.write(line.as_bytes());
                        let _ = pty.write(b"\r");
                    }
                }
                self.update_input_display();
                self.request_redraw();
                return;
            }
            Key::Named(NamedKey::PageUp) => {
                self.scroll_page(true);
                return;
            }
            Key::Named(NamedKey::PageDown) => {
                self.scroll_page(false);
                return;
            }
            Key::Named(NamedKey::Backspace) => {
                self.input.backspace();
                self.update_input_display();
                self.maybe_update_autocomplete();
                self.request_redraw();
                return;
            }
            Key::Named(NamedKey::Delete) => {
                self.input.delete_forward();
                self.update_input_display();
                self.maybe_update_autocomplete();
                self.request_redraw();
                return;
            }
            Key::Named(NamedKey::ArrowLeft) => {
                self.input.move_left();
                self.update_input_display();
                self.request_redraw();
                return;
            }
            Key::Named(NamedKey::ArrowRight) => {
                self.input.move_right();
                self.update_input_display();
                self.request_redraw();
                return;
            }
            Key::Named(NamedKey::Home) => {
                self.input.home_line();
                self.update_input_display();
                self.request_redraw();
                return;
            }
            Key::Named(NamedKey::End) => {
                self.input.end_line();
                self.update_input_display();
                self.request_redraw();
                return;
            }
            Key::Named(NamedKey::ArrowUp) => {
                // Cmd+Up scrolls the output; plain Up is editor/history.
                if self.modifiers.super_key() {
                    self.scroll_terminal(1);
                    return;
                }
                let on_first_line = self.input.cursor_line() == 0;
                if self.input.line_count() == 1 || on_first_line {
                    self.input.history_prev();
                } else {
                    self.input.move_up();
                }
                self.update_input_display();
                self.request_redraw();
                return;
            }
            Key::Named(NamedKey::ArrowDown) => {
                if self.modifiers.super_key() {
                    self.scroll_terminal(-1);
                    return;
                }
                let on_last_line = self.input.cursor_line() == self.input.line_count() - 1;
                if self.input.line_count() == 1 || on_last_line {
                    self.input.history_next();
                } else {
                    self.input.move_down();
                }
                self.update_input_display();
                self.request_redraw();
                return;
            }
            Key::Named(NamedKey::Escape) => {
                if self.input.is_in_history_recall() {
                    self.input.cancel_history_recall();
                    self.update_input_display();
                    self.request_redraw();
                    return;
                }
                if self.selection.is_some() {
                    self.clear_selection();
                    return;
                }
            }
            _ => {}
        }

        let Some(text) = event.text_with_all_modifiers() else {
            return;
        };
        if text.is_empty() {
            return;
        }

        let is_control =
            text.len() == 1 && (text.as_bytes()[0] < 0x20 || text.as_bytes()[0] == 0x7f);
        if is_control {
            if text.as_bytes()[0] == 0x03 {
                self.input.clear();
                self.update_input_display();
            }
            if let Some(pty) = &mut self.pty {
                let _ = pty.write(text.as_bytes());
            }
        } else {
            // Typing means the user has moved on — stale selections go.
            self.clear_selection();
            self.input.insert_str(text);
            self.update_input_display();
            self.maybe_update_autocomplete();
        }

        self.request_redraw();
    }

    /// Check if autocomplete should trigger or update after a keystroke.
    pub(super) fn maybe_update_autocomplete(&mut self) {
        let buffer = self.input.buffer();
        let cursor = self.input.cursor();

        // Popup already open — update the filter.
        if matches!(self.popup, PopupState::Autocomplete) && self.autocomplete.active() {
            if !self.autocomplete.update_filter(buffer, cursor) {
                self.popup = PopupState::Hidden;
            }
            self.update_input_display();
            return;
        }

        // Check if we should trigger.
        let Some((token_start, command)) = Autocomplete::should_trigger(buffer, cursor) else {
            return;
        };
        let Some(cwd) = self
            .terminal
            .as_ref()
            .and_then(|t| t.cwd())
            .map(|p| p.to_path_buf())
        else {
            return;
        };
        match self
            .autocomplete
            .trigger(&cwd, buffer, cursor, token_start, &command)
        {
            Ok(()) if self.autocomplete.active() => {
                self.popup = PopupState::Autocomplete;
                self.update_input_display();
            }
            Ok(()) => {}
            Err(e) => {
                log::warn!("autocomplete trigger failed: {}", e);
            }
        }
    }

    /// Raw-mode key handling — the PTY owns the keyboard.
    fn handle_keyboard_raw(&mut self, event: winit::event::KeyEvent) {
        use winit::keyboard::{Key, NamedKey};
        use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;

        // Typing returns a scrolled viewport to the live tail, same as
        // submitting does in cooked mode. No-op on the alt screen where
        // the offset is always 0.
        self.snap_to_bottom();

        let bytes: Option<&[u8]> = match &event.logical_key {
            Key::Named(NamedKey::Enter) => Some(b"\r"),
            Key::Named(NamedKey::Backspace) => Some(b"\x7f"),
            Key::Named(NamedKey::Tab) => Some(b"\t"),
            Key::Named(NamedKey::Escape) => Some(b"\x1b"),
            Key::Named(NamedKey::ArrowUp) => Some(b"\x1b[A"),
            Key::Named(NamedKey::ArrowDown) => Some(b"\x1b[B"),
            Key::Named(NamedKey::ArrowRight) => Some(b"\x1b[C"),
            Key::Named(NamedKey::ArrowLeft) => Some(b"\x1b[D"),
            Key::Named(NamedKey::Home) => Some(b"\x1b[H"),
            Key::Named(NamedKey::End) => Some(b"\x1b[F"),
            Key::Named(NamedKey::PageUp) => Some(b"\x1b[5~"),
            Key::Named(NamedKey::PageDown) => Some(b"\x1b[6~"),
            Key::Named(NamedKey::Delete) => Some(b"\x1b[3~"),
            _ => None,
        };

        if let Some(bytes) = bytes {
            if let Some(pty) = &mut self.pty {
                let _ = pty.write(bytes);
            }
        } else if let Some(text) = event.text_with_all_modifiers()
            && let Some(pty) = &mut self.pty
        {
            let _ = pty.write(text.as_bytes());
        }

        self.request_redraw();
    }
}
