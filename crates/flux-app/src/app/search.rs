//! Find in scrollback (F14) — Cmd+F opens a bar in the top-right,
//! typing searches as you go (literal, case-insensitive), Enter /
//! Down = next, Shift+Enter / Up = previous, Esc closes and leaves the
//! viewport where you were. Matches tint yellow, the focused one
//! orange. State is per-App (one bar), searching the focused pane.

use super::{App, PopupState};

#[derive(Default)]
pub(crate) struct SearchBar {
    pub query: String,
}

impl App {
    pub(super) fn open_search(&mut self) {
        self.popup = PopupState::Search;
        // Reopening keeps the previous query — like every editor.
        let query = self.search.query.clone();
        if let Some(pane) = self.pane_mut() {
            pane.terminal.search_set(&query);
        }
        self.refresh_search_ui();
    }

    pub(super) fn close_search(&mut self) {
        self.popup = PopupState::Hidden;
        if let Some(pane) = self.pane_mut() {
            pane.terminal.search_clear();
        }
        if let Some(renderer) = &mut self.renderer {
            renderer.hide_autocomplete_popup();
        }
        self.update_display();
        self.request_redraw();
    }

    /// Handle a key while the search bar is open. Always consumes the
    /// key — the bar owns the keyboard until Esc.
    pub(super) fn handle_search_key(&mut self, event: &winit::event::KeyEvent) -> bool {
        use winit::keyboard::{Key, NamedKey};
        use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;

        match &event.logical_key {
            Key::Named(NamedKey::Escape) => {
                self.close_search();
                return true;
            }
            Key::Named(NamedKey::Enter) => {
                if self.modifiers.shift_key() {
                    self.search_step(false);
                } else {
                    self.search_step(true);
                }
                return true;
            }
            Key::Named(NamedKey::ArrowDown) => {
                self.search_step(true);
                return true;
            }
            Key::Named(NamedKey::ArrowUp) => {
                self.search_step(false);
                return true;
            }
            Key::Named(NamedKey::Backspace) => {
                self.search.query.pop();
                self.apply_search_query();
                return true;
            }
            _ => {}
        }
        // Cmd-chords (Cmd+F again, Cmd+C…) are not query text.
        if self.modifiers.super_key() || self.modifiers.control_key() {
            return true;
        }
        if let Some(text) = event.text_with_all_modifiers() {
            let printable: String = text.chars().filter(|c| !c.is_control()).collect();
            if !printable.is_empty() {
                self.search.query.push_str(&printable);
                self.apply_search_query();
            }
        }
        true
    }

    fn apply_search_query(&mut self) {
        let query = self.search.query.clone();
        if let Some(pane) = self.pane_mut() {
            pane.terminal.search_set(&query);
        }
        self.refresh_search_ui();
    }

    fn search_step(&mut self, forward: bool) {
        if let Some(pane) = self.pane_mut() {
            if forward {
                pane.terminal.search_next();
            } else {
                pane.terminal.search_prev();
            }
        }
        self.refresh_search_ui();
    }

    /// Push bar text + highlights to the renderer.
    pub(super) fn refresh_search_ui(&mut self) {
        let status = self.terminal().and_then(|t| t.search_status());
        let (position, total) = status.unwrap_or((None, 0));
        let query = self.search.query.clone();
        if let Some(renderer) = &mut self.renderer {
            renderer.set_search_bar(&query, position, total);
        }
        self.update_display();
        self.request_redraw();
    }
}
