//! System clipboard integration — paste detection and handling.
//!
//! F12 will add copy in this same module. For v0.2 the clipboard
//! surface is just "paste the system clipboard into the editor
//! (cooked) or into the PTY with bracketed-paste markers (raw)".

use arboard::Clipboard;

use super::App;

impl App {
    /// Detect the system paste chord — Cmd+V on macOS, Ctrl+Shift+V elsewhere.
    pub(super) fn is_paste_shortcut(&self, event: &winit::event::KeyEvent) -> bool {
        use winit::keyboard::{Key, NamedKey};
        let is_v = matches!(&event.logical_key, Key::Character(c) if c.eq_ignore_ascii_case("v"))
            || matches!(&event.logical_key, Key::Named(NamedKey::Paste));
        if !is_v {
            return false;
        }
        let m = self.modifiers;
        if cfg!(target_os = "macos") {
            m.super_key() && !m.control_key() && !m.alt_key()
        } else {
            m.control_key() && m.shift_key() && !m.alt_key() && !m.super_key()
        }
    }

    /// Read the system clipboard and route the text into the editor (cooked
    /// mode) or the PTY (raw mode). In raw mode we wrap the payload in the
    /// bracketed-paste markers when the child program has enabled that mode,
    /// so vim et al can distinguish a paste from typed input.
    pub(super) fn handle_paste(&mut self) {
        let text = match self.clipboard_text() {
            Some(t) if !t.is_empty() => t,
            _ => return,
        };

        if self.raw_mode {
            let bracketed = self
                .terminal
                .as_ref()
                .map(|t| t.is_bracketed_paste())
                .unwrap_or(false);
            if let Some(pty) = &mut self.pty {
                if bracketed {
                    let _ = pty.write(b"\x1b[200~");
                }
                let _ = pty.write(text.as_bytes());
                if bracketed {
                    let _ = pty.write(b"\x1b[201~");
                }
            }
        } else {
            // In cooked mode we collapse newlines so multi-line pastes don't
            // fire submissions through Enter handling. Proper multi-line
            // editing lands with #22.
            let sanitized: String = text
                .chars()
                .map(|c| if c == '\n' || c == '\r' { ' ' } else { c })
                .collect();
            self.input.insert_str(&sanitized);
            self.update_input_display();
        }

        self.request_redraw();
    }

    fn clipboard_text(&mut self) -> Option<String> {
        if self.clipboard.is_none() {
            match Clipboard::new() {
                Ok(cb) => self.clipboard = Some(cb),
                Err(e) => {
                    log::error!("Clipboard init failed: {}", e);
                    return None;
                }
            }
        }
        match self.clipboard.as_mut()?.get_text() {
            Ok(text) => Some(text),
            Err(e) => {
                log::warn!("Clipboard read failed: {}", e);
                None
            }
        }
    }
}
