//! Window layout — padding, grid dimensions, resize, DPI change.
//!
//! All math that turns a window size into a grid size lives here, plus
//! the event handlers that react to the window changing. `handle_resize`
//! and `handle_scale_change` call into `apply_window_layout` after
//! reconfiguring the renderer's surface so the grid follows the window.

use std::sync::Arc;
use winit::window::Window;

use super::{App, INPUT_CHROME_ROWS};

impl App {
    /// Recompute the grid dimensions from the current window size, accounting
    /// for padding and whether Flux chrome is currently reserving rows. Called
    /// on startup, window resize, scale change, and raw-mode transitions.
    pub(super) fn apply_window_layout(&mut self) {
        let Some(window) = &self.window else { return };
        let Some(renderer) = &mut self.renderer else { return };

        let inner_size = window.inner_size();
        let metrics = renderer.cell_metrics();
        let pad_x = padding_x(&self.config, window);
        let pad_y = padding_y(&self.config, window);
        let usable_w = (inner_size.width as f32 - pad_x * 2.0).max(0.0);
        let usable_h = (inner_size.height as f32 - pad_y * 2.0).max(0.0);
        let cols = (usable_w / metrics.width) as usize;
        let total_rows = (usable_h / metrics.height) as usize;
        let chrome_rows = if self.raw_mode { 0 } else { INPUT_CHROME_ROWS };
        let rows = total_rows.saturating_sub(chrome_rows).max(1);

        if let Some(terminal) = &mut self.terminal {
            terminal.resize(cols.max(1), rows);
        }
        if let Some(pty) = &mut self.pty {
            let _ = pty.resize(cols.max(1) as u16, rows as u16);
        }
    }

    pub(super) fn handle_resize(&mut self, width: u32, height: u32) {
        // Reconfigure surface + resize grid + render — all in the same event.
        // Presenting a frame before returning from the resize handler prevents
        // the compositor from stretching a stale frame.
        if let Some(renderer) = &mut self.renderer {
            renderer.resize(width, height);
        }

        self.apply_window_layout();
        self.update_display();
        if !self.raw_mode {
            self.update_input_display();
        }

        let renderer = self.renderer.as_mut().expect("renderer not initialized");
        if let Err(e) = renderer.render() {
            log::error!("Resize render error: {}", e);
        }
    }

    pub(super) fn handle_scale_change(&mut self, scale_factor: f32) {
        log::info!("Scale factor changed to {}", scale_factor);

        let font_size_px = self.config.font.size * scale_factor;
        let font_family = self.config.font.family.clone();
        let line_height = self.config.font.line_height;

        let Some(renderer) = &mut self.renderer else { return };
        if let Err(e) = renderer.rebuild_font(&font_family, font_size_px, line_height) {
            log::error!("Failed to rebuild font: {}", e);
            return;
        }

        // Recalculate grid after font change
        if let Some(window) = &self.window {
            let size = window.inner_size();
            renderer.resize(size.width, size.height);
        }

        self.apply_window_layout();
        self.update_display();
        if !self.raw_mode {
            self.update_input_display();
        }
        self.request_redraw();
    }
}

/// Horizontal padding resolved against the window's current scale
/// factor. Pulled out as a free helper so `apply_window_layout` can
/// compute both dimensions without calling `self.padding_*()` (which
/// borrows `self.window` and `self.config` simultaneously).
fn padding_x(config: &crate::config::FluxConfig, window: &Arc<Window>) -> f32 {
    let scale_factor = window.scale_factor() as f32;
    config.window.padding_horizontal * scale_factor
}

fn padding_y(config: &crate::config::FluxConfig, window: &Arc<Window>) -> f32 {
    let scale_factor = window.scale_factor() as f32;
    config.window.padding_vertical * scale_factor
}
