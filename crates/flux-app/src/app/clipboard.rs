//! System clipboard integration — copy and paste.
//!
//! Paste routes into the editor (cooked) or the PTY with
//! bracketed-paste markers (raw). Copy pulls the active mouse
//! selection's text out of the grid snapshot.

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

    /// Detect the system copy chord — Cmd+C on macOS, Ctrl+Shift+C elsewhere.
    /// Plain Ctrl+C stays SIGINT territory on every platform.
    pub(super) fn is_copy_shortcut(&self, event: &winit::event::KeyEvent) -> bool {
        use winit::keyboard::{Key, NamedKey};
        let is_c = matches!(&event.logical_key, Key::Character(c) if c.eq_ignore_ascii_case("c"))
            || matches!(&event.logical_key, Key::Named(NamedKey::Copy));
        if !is_c {
            return false;
        }
        let m = self.modifiers;
        if cfg!(target_os = "macos") {
            m.super_key() && !m.control_key() && !m.alt_key()
        } else {
            m.control_key() && m.shift_key() && !m.alt_key() && !m.super_key()
        }
    }

    /// Copy the active selection to the system clipboard. Returns true
    /// if a selection consumed the chord (even if it held only
    /// whitespace); false lets the caller fall through to whatever the
    /// key would otherwise do.
    pub(super) fn handle_copy(&mut self) -> bool {
        let Some(sel) = self.selection else {
            return false;
        };
        let Some(grid) = self.snapshot_for_selection() else {
            return false;
        };
        let text = sel.text(&grid);
        if !text.is_empty() {
            self.set_clipboard_text(text);
        }
        true
    }

    fn set_clipboard_text(&mut self, text: String) {
        if self.clipboard.is_none() {
            match Clipboard::new() {
                Ok(cb) => self.clipboard = Some(cb),
                Err(e) => {
                    log::error!("Clipboard init failed: {}", e);
                    return;
                }
            }
        }
        if let Some(cb) = self.clipboard.as_mut()
            && let Err(e) = cb.set_text(text)
        {
            log::warn!("Clipboard copy failed: {}", e);
        }
    }

    /// Read the system clipboard and route the text into the editor (when
    /// the editor owns the keyboard) or the PTY (alt-screen programs and
    /// executing commands). On the PTY path we wrap the payload in the
    /// bracketed-paste markers when the child program has enabled that
    /// mode, so vim et al can distinguish a paste from typed input.
    pub(super) fn handle_paste(&mut self) {
        let text = match self.clipboard_text() {
            Some(t) if !t.is_empty() => t,
            _ => return,
        };

        let pty_owns = self.raw_mode
            || self
                .terminal
                .as_ref()
                .map(|t| t.is_executing())
                .unwrap_or(false);
        if pty_owns {
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
            // Multi-line paste: normalize \r\n to \n, strip trailing \r.
            let normalized = text.replace("\r\n", "\n").replace('\r', "\n");
            self.input.insert_str(&normalized);
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
