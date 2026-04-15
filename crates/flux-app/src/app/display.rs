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

        let grid = terminal.render_grid(fg, bg);
        renderer.set_grid(&grid);
    }

    /// Push the current input editor state to the renderer.
    pub(super) fn update_input_display(&mut self) {
        let Some(renderer) = &mut self.renderer else { return };
        renderer.set_input_line(self.input.buffer(), self.input.cursor_col());
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
