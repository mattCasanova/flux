//! System clipboard integration — copy and paste.
//!
//! Paste routes into the editor (cooked) or the PTY with
//! bracketed-paste markers (raw). Copy pulls the active mouse
//! selection's text out of the grid snapshot.

use arboard::Clipboard;

use super::App;

impl App {
    /// Copy the active selection to the system clipboard. Returns true
    /// if a selection consumed the chord; false lets the caller fall
    /// through to whatever the key would otherwise do. The text comes
    /// from the terminal's content-anchored selection, so it can span
    /// scrollback well beyond the visible screen.
    pub(super) fn handle_copy(&mut self) -> bool {
        let Some(text) = self.terminal().and_then(|t| t.selection_text()) else {
            return false;
        };
        self.set_clipboard_text(text);
        true
    }

    pub(super) fn set_clipboard_text(&mut self, text: String) {
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

        let pty_owns = self.raw_mode || self.terminal().map(|t| t.is_executing()).unwrap_or(false);
        if pty_owns {
            let bracketed = self
                .terminal()
                .map(|t| t.is_bracketed_paste())
                .unwrap_or(false);
            if let Some(pane) = self.pane_mut() {
                let pty = &mut pane.pty;
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
            if let Some(editor) = self.input_mut() {
                editor.insert_str(&normalized);
            }
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

    /// Copy the most recent finished block's output — no selection
    /// needed (Cmd+Shift+C). Returns false when there is none.
    pub(super) fn copy_last_block_output(&mut self) -> bool {
        let Some(text) = self.terminal().and_then(|t| t.last_block_output()) else {
            return false;
        };
        self.set_clipboard_text(text);
        true
    }
}
