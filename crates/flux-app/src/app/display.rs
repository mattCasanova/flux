//! Display refresh — pushing the current terminal + input state to the
//! renderer and requesting a redraw from winit. This is the narrow
//! "tell the GPU what to show" surface; the rendering code itself lives
//! entirely in `flux-renderer`.

use flux_types::Color;

use super::App;

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
