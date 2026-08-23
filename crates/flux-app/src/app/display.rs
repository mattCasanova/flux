//! Display refresh — pushing the current terminal + input state to the
//! renderer and requesting a redraw from winit. This is the narrow
//! "tell the GPU what to show" surface; the rendering code itself lives
//! entirely in `flux-renderer`.

use super::{App, PopupState};

impl App {
    /// Render every pane of the current tab. The focused pane decides
    /// the window's padding color and shows the shell cursor when the
    /// PTY owns the keyboard (alt screen OR a running command — sudo,
    /// ssh, REPLs); at the prompt the input bar owns cursor display.
    pub(super) fn update_display(&mut self) {
        let Some(tab) = self.mux.focused_tab_mut() else {
            return;
        };
        let focus = tab.focus;
        let mut frames: Vec<(u64, flux_types::TerminalGrid, flux_renderer::PaneView)> = Vec::new();
        for pane in tab.root.panes_mut() {
            let alt = pane.terminal.is_alt_screen();
            let is_focus = pane.id == focus;
            let show_cursor = is_focus && (alt || pane.terminal.is_executing());
            let grid = pane.terminal.grid_snapshot();
            frames.push((
                pane.id,
                grid,
                flux_renderer::PaneView {
                    origin: [pane.viewport.x, pane.viewport.y],
                    bottom_anchor: !alt,
                    show_cursor,
                    drives_clear_color: is_focus,
                },
            ));
        }
        let Some(renderer) = &mut self.renderer else {
            return;
        };
        for (id, grid, view) in &frames {
            renderer.set_pane_grid(*id, grid, *view);
        }
    }

    /// Push every cooked pane's input bar to the renderer (the focused
    /// pane's bar carries the block cursor; unfocused bars render dim
    /// with no cursor). Re-runs layout first when any pane's chrome
    /// (alt state, editor line count) changed — that resizes only the
    /// panes whose dimensions actually changed.
    pub(super) fn update_input_display(&mut self) {
        if self.chrome_dirty() {
            self.apply_window_layout();
            self.update_display();
        }

        let Some(tab) = self.mux.focused_tab() else {
            return;
        };
        let focus = tab.focus;
        let metrics = match &self.renderer {
            Some(renderer) => renderer.cell_metrics(),
            None => return,
        };
        let mut bars: Vec<flux_renderer::InputBar> = Vec::new();
        let mut anchor: Option<(f32, f32, usize)> = None; // popup anchor
        for pane in tab.root.panes() {
            if pane.terminal.is_alt_screen() {
                continue;
            }
            let focused = pane.id == focus;
            let lines = pane.input.line_count();
            let vp = pane.viewport;
            // The bar hugs the bottom of the pane's viewport.
            let bar_h = (1 + lines) as f32 * metrics.height;
            let top_y = vp.y + vp.height - bar_h;
            if focused {
                let cursor_row_y = top_y + metrics.height * (1 + pane.input.cursor_line()) as f32;
                anchor = Some((vp.x, cursor_row_y, pane.input.cursor_col_in_line()));
            }
            bars.push(flux_renderer::InputBar {
                origin: [vp.x, top_y],
                width: vp.width,
                text: pane.input.buffer().to_string(),
                cursor: focused
                    .then(|| (pane.input.cursor_line(), pane.input.cursor_col_in_line())),
            });
        }
        let popup_data = self.autocomplete_popup_data();
        let Some(renderer) = &mut self.renderer else {
            return;
        };
        renderer.set_input_bars(&bars);

        // Autocomplete popup, anchored at the focused pane's cursor.
        if let (Some(candidates), Some((bar_x, row_y, cursor_col))) = (popup_data, anchor) {
            let selected = self.autocomplete.selected_index();
            let anchor_px_x = bar_x;
            let anchor_col = cursor_col + 2; // prompt prefix width
            let _ = anchor_px_x;
            renderer.set_autocomplete_popup(&candidates, selected, anchor_col, row_y);
        } else if !matches!(self.popup, PopupState::Search) {
            renderer.hide_autocomplete_popup();
        }
    }

    /// Candidate list for the popup, if it should be visible.
    fn autocomplete_popup_data(&self) -> Option<Vec<(String, flux_renderer::PopupKind)>> {
        if !(matches!(self.popup, PopupState::Autocomplete) && self.autocomplete.active()) {
            return None;
        }
        Some(
            self.autocomplete
                .visible_candidates()
                .iter()
                .map(|c| {
                    let kind = match c.kind {
                        flux_input::CandidateKind::Directory => flux_renderer::PopupKind::Directory,
                        flux_input::CandidateKind::File => flux_renderer::PopupKind::File,
                        flux_input::CandidateKind::Symlink => flux_renderer::PopupKind::Symlink,
                        flux_input::CandidateKind::Other => flux_renderer::PopupKind::Other,
                    };
                    (c.name.clone(), kind)
                })
                .collect(),
        )
    }

    pub(super) fn handle_redraw(&mut self) {
        // Drag-autoscroll advances frame-paced: while a selection drag
        // rests past the output edge, each redraw scrolls another step
        // and requests the next frame.
        self.step_drag_autoscroll();

        let Some(renderer) = &mut self.renderer else {
            return;
        };
        if let Err(e) = renderer.render() {
            log::error!("Render error: {}", e);
        }
    }

    pub(super) fn request_redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}
