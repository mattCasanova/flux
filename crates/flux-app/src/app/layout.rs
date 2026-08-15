//! Window layout — padding, grid dimensions, resize, DPI change.
//!
//! All math that turns a window size into a grid size lives here, plus
//! the event handlers that react to the window changing. `handle_resize`
//! and `handle_scale_change` call into `apply_window_layout` after
//! reconfiguring the renderer's surface so the grid follows the window.

use std::sync::Arc;
use winit::window::Window;

use super::App;

impl App {
    /// Recompute the grid dimensions from the current window size, accounting
    /// for padding and whether Flux chrome is currently reserving rows. Called
    /// on startup, window resize, scale change, and raw-mode transitions.
    pub(super) fn apply_window_layout(&mut self) {
        let Some(window) = &self.window else { return };
        let Some(renderer) = &mut self.renderer else {
            return;
        };

        let inner_size = window.inner_size();
        let metrics = renderer.cell_metrics();
        let pad_x = padding_x(&self.config, window);
        let pad_y = padding_y(&self.config, window);
        // The tab bar (2+ tabs) takes one row off the top, in raw mode
        // too — vim in one tab shouldn't hide the others.
        let bar_h = renderer.tab_bar_height(self.mux.tabs.len());
        renderer.set_content_top(bar_h);
        let usable_w = (inner_size.width as f32 - pad_x * 2.0).max(0.0);
        // 1 divider row + N input lines (dynamic based on editor content).
        let input_lines = self.input.line_count();
        let chrome_rows = if self.raw_mode { 0 } else { 1 + input_lines };
        let usable_h =
            (inner_size.height as f32 - pad_y * 2.0 - bar_h - chrome_rows as f32 * metrics.height)
                .max(0.0);
        let content = flux_types::Rect::new(pad_x, pad_y + bar_h, usable_w, usable_h);
        let cell_w = metrics.width;
        let cell_h = metrics.height;

        // Lay the pane tree out over the content rect and size every
        // pane's grid to its viewport (whole cells only).
        if let Some(tab) = self.mux.focused_tab_mut() {
            tab.root.layout(content);
            for pane in tab.root.panes_mut() {
                let cols = ((pane.viewport.width / cell_w) as usize).max(1);
                let rows = ((pane.viewport.height / cell_h) as usize).max(1);
                if pane.terminal.cols() != cols || pane.terminal.rows() != rows {
                    pane.terminal.resize(cols, rows);
                    let _ = pane.pty.resize(cols as u16, rows as u16);
                }
            }
        }
        self.update_pane_frames();

        // The bar's background rect spans the window — rebuild it so a
        // resize doesn't leave it at the old width.
        self.update_tab_bar();
    }

    /// Content rect (pixels) the pane tree is laid out in — same math
    /// as `apply_window_layout`, for callers that only need the rect.
    pub(super) fn content_rect(&self) -> Option<flux_types::Rect> {
        let window = self.window.as_ref()?;
        let renderer = self.renderer.as_ref()?;
        let inner_size = window.inner_size();
        let metrics = renderer.cell_metrics();
        let pad_x = padding_x(&self.config, window);
        let pad_y = padding_y(&self.config, window);
        let bar_h = renderer.tab_bar_height(self.mux.tabs.len());
        let chrome_rows = if self.raw_mode {
            0
        } else {
            1 + self.input.line_count()
        };
        let usable_w = (inner_size.width as f32 - pad_x * 2.0).max(0.0);
        let usable_h =
            (inner_size.height as f32 - pad_y * 2.0 - bar_h - chrome_rows as f32 * metrics.height)
                .max(0.0);
        Some(flux_types::Rect::new(
            pad_x,
            pad_y + bar_h,
            usable_w,
            usable_h,
        ))
    }

    /// Push split dividers + focused accent to the renderer.
    pub(super) fn update_pane_frames(&mut self) {
        let Some(content) = self.content_rect() else {
            return;
        };
        let (gutters, focused) = match self.mux.focused_tab() {
            Some(tab) => {
                let mut g = Vec::new();
                tab.root.dividers(content, &mut g);
                (g, tab.focused_pane().map(|p| p.viewport))
            }
            None => (Vec::new(), None),
        };
        if let Some(renderer) = &mut self.renderer {
            renderer.set_pane_frames(&gutters, focused);
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

        let Some(renderer) = &mut self.renderer else {
            return;
        };
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
