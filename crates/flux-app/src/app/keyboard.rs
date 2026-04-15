//! Keyboard event routing.
//!
//! `handle_keyboard` is the top-level entry point. Order of operations:
//!
//! 1. Drop key releases (only Pressed events do anything).
//! 2. **Popup intercept** (R6 scaffold) — if an overlay like
//!    autocomplete or search is active, give it first refusal on
//!    the key. R6 only has `PopupState::Hidden` so this branch
//!    currently does nothing; F7 and F14 add real arms. The match
//!    is exhaustive with NO wildcard arm, so adding a new variant
//!    is a compile error until its intercept is wired up.
//! 3. Clipboard shortcut short-circuit — paste must work in both
//!    raw and cooked mode, so it runs before the mode split.
//! 4. Branch to `handle_keyboard_raw` (PTY-first) or
//!    `handle_keyboard_cooked` (editor-first) based on `raw_mode`.

use super::{App, PopupState};

impl App {
    pub(super) fn handle_keyboard(&mut self, event: winit::event::KeyEvent) {
        use winit::event::ElementState;

        if event.state != ElementState::Pressed {
            return;
        }

        // Popup intercept. Exhaustive match — NO wildcard arm. F7
        // adds `PopupState::Autocomplete => { ... }`, F14 adds
        // `PopupState::Search => { ... }`; each feature's arm is
        // free to return early if it handled the event.
        match &self.popup {
            PopupState::Hidden => {}
        }

        // Clipboard shortcuts run ahead of mode-specific handling so they
        // work identically in cooked and raw mode. Cmd on macOS maps to
        // super in winit; Ctrl+Shift is the cross-platform fallback.
        if self.is_paste_shortcut(&event) {
            self.handle_paste();
            return;
        }

        if self.raw_mode {
            self.handle_keyboard_raw(event);
            return;
        }

        self.handle_keyboard_cooked(event);
    }

    /// Cooked-mode key handling — keystrokes go through the Flux input editor,
    /// Enter submits the composed line, Ctrl+<letter> bypasses the editor.
    fn handle_keyboard_cooked(&mut self, event: winit::event::KeyEvent) {
        use winit::keyboard::{Key, NamedKey};
        use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;

        match &event.logical_key {
            // Enter submits the composed line to the PTY (plus \r).
            Key::Named(NamedKey::Enter) => {
                let line = self.input.take_line();
                if let Some(pty) = &mut self.pty {
                    let _ = pty.write(line.as_bytes());
                    let _ = pty.write(b"\r");
                }
                self.update_input_display();
                self.request_redraw();
                return;
            }
            Key::Named(NamedKey::Backspace) => {
                self.input.backspace();
                self.update_input_display();
                self.request_redraw();
                return;
            }
            Key::Named(NamedKey::Delete) => {
                self.input.delete_forward();
                self.update_input_display();
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
                self.input.home();
                self.update_input_display();
                self.request_redraw();
                return;
            }
            Key::Named(NamedKey::End) => {
                self.input.end();
                self.update_input_display();
                self.request_redraw();
                return;
            }
            // Arrow up/down are reserved for history (#21) — swallow for now so
            // they don't bleed into the PTY as cursor movements.
            Key::Named(NamedKey::ArrowUp) | Key::Named(NamedKey::ArrowDown) => return,
            _ => {}
        }

        // Everything else: text input or Ctrl+<letter>. `text_with_all_modifiers`
        // folds Ctrl effects into the string (Ctrl+C → \x03, Ctrl+D → \x04, etc.),
        // which is the terminal-correct interpretation. Single-byte control
        // characters bypass the editor and go straight to the PTY; anything else
        // is insertable text.
        let Some(text) = event.text_with_all_modifiers() else { return };
        if text.is_empty() {
            return;
        }

        let is_control =
            text.len() == 1 && (text.as_bytes()[0] < 0x20 || text.as_bytes()[0] == 0x7f);
        if is_control {
            // Ctrl+C clears the editor buffer so the user starts fresh after the interrupt.
            if text.as_bytes()[0] == 0x03 {
                self.input.clear();
                self.update_input_display();
            }
            if let Some(pty) = &mut self.pty {
                let _ = pty.write(text.as_bytes());
            }
        } else {
            self.input.insert_str(text);
            self.update_input_display();
        }

        self.request_redraw();
    }

    /// Raw-mode key handling — the PTY owns the keyboard. Forward named keys
    /// as the standard xterm escape sequences and everything else via
    /// `text_with_all_modifiers` so Ctrl combos land correctly.
    fn handle_keyboard_raw(&mut self, event: winit::event::KeyEvent) {
        use winit::keyboard::{Key, NamedKey};
        use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;

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
