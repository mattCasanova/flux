//! Display refresh — pushing the current terminal + input state to the
//! renderer and requesting a redraw from winit. This is the narrow
//! "tell the GPU what to show" surface; the rendering code itself lives
//! entirely in `flux-renderer`.

use flux_types::Color;

use super::{App, PopupState};

impl App {
    /// Render the terminal grid.
    pub(super) fn update_display(&mut self) {
        let Some(terminal) = &self.terminal else { return };
        let Some(renderer) = &mut self.renderer else { return };

        let fg = Color::from_hex(&self.config.theme.foreground).unwrap_or_default();
        let bg = Color::from_hex(&self.config.theme.background)
            .unwrap_or(Color::new(0.0, 0.0, 0.0, 1.0));

        let grid = terminal.grid_snapshot(fg, bg);
        renderer.set_grid(&grid);
    }

    /// Push the current input editor state to the renderer. If the
    /// input line count changed, recompute the layout so the PTY gets
    /// the updated row count.
    pub(super) fn update_input_display(&mut self) {
        let current_lines = self.input.line_count();
        if current_lines != self.last_input_lines {
            self.last_input_lines = current_lines;
            self.apply_window_layout();
            self.update_display();
        }

        let Some(renderer) = &mut self.renderer else { return };
        let cursor = (self.input.cursor_line(), self.input.cursor_col_in_line());
        renderer.set_input_block(self.input.buffer(), cursor);

        // Autocomplete popup.
        if matches!(self.popup, PopupState::Autocomplete) && self.autocomplete.active() {
            let candidates: Vec<(String, flux_renderer::PopupKind)> = self
                .autocomplete
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
                .collect();

            let selected = self.autocomplete.selected_index();

            // Compute anchor position — cursor row Y in the input bar.
            let metrics = renderer.cell_metrics();
            let cell_h = metrics.height;
            let window_h = self.window.as_ref().map(|w| w.inner_size().height).unwrap_or(0) as f32;
            let scale = self.window.as_ref().map(|w| w.scale_factor() as f32).unwrap_or(1.0);
            let pad_y = self.config.window.padding_vertical * scale;
            let line_count = self.input.line_count();
            let block_bottom_y = window_h - pad_y - cell_h;
            let block_top_y = block_bottom_y - (line_count as f32 - 1.0) * cell_h;
            let cursor_row = self.input.cursor_line();
            let anchor_row_y = block_top_y + (cursor_row as f32) * cell_h;
            // +2 for the prompt prefix characters.
            let anchor_col = self.input.cursor_col_in_line() + 2;

            renderer.set_autocomplete_popup(&candidates, selected, anchor_col, anchor_row_y);
        } else {
            renderer.hide_autocomplete_popup();
        }
    }

    pub(super) fn handle_redraw(&mut self) {
        let Some(renderer) = &mut self.renderer else { return };
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
