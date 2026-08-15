//! Scrollbar overlay — `set_scrollbar`.
//!
//! A thin thumb (plus a faint track) along the right edge of the output
//! area, shown only while the viewport is scrolled into history. Thumb
//! height is the visible fraction of history+screen; its position maps
//! `display_offset` onto the track (top = oldest, bottom = live tail).
//! Rebuilt from `set_grid`, so it always agrees with what's painted.

use crate::core::CellInstance;
use crate::renderer::Renderer;
use flux_types::{Color, TerminalGrid};

/// Thumb and track colors — cool gray at two alphas, matching the
/// divider's palette. Becomes a theme key when F16 lands.
const THUMB: Color = Color::new(0.60, 0.65, 0.80, 0.55);
const TRACK: Color = Color::new(0.60, 0.65, 0.80, 0.10);

impl Renderer {
    pub(crate) fn set_scrollbar(&mut self, grid: &TerminalGrid) {
        let mut instances = std::mem::take(&mut self.scrollbar_instances);
        instances.clear();

        // Only while scrolled up, and only if there is history to show
        // (alt-screen grids report none).
        if grid.display_offset > 0 && grid.history_size > 0 && grid.rows > 0 {
            let cell_h = self.atlas.cell_height;
            let window_w = self.gpu.surface_config.width as f32;
            let pad_x = self.padding_x;
            let pad_y = self.padding_y;

            let area_y = pad_y + self.content_top;
            let area_h = grid.rows as f32 * cell_h;

            // Sit centered in the right padding when there is room,
            // else hug the window edge over the last column.
            let width = (pad_x * 0.3).clamp(2.0, 5.0);
            let x = if pad_x >= width + 2.0 {
                window_w - pad_x + (pad_x - width) * 0.5
            } else {
                window_w - width - 1.0
            };

            let total = (grid.history_size + grid.rows) as f32;
            let min_thumb = cell_h.max(12.0);
            let thumb_h = (area_h * grid.rows as f32 / total)
                .max(min_thumb)
                .min(area_h);
            // 1.0 = scrolled to the oldest line, 0.0 = live tail.
            let up = grid.display_offset.min(grid.history_size) as f32 / grid.history_size as f32;
            let thumb_y = area_y + (area_h - thumb_h) * (1.0 - up);

            let rect = |x: f32, y: f32, w: f32, h: f32, c: Color| CellInstance {
                position: [x, y],
                size: [w, h],
                glyph_uv: [0.0, 0.0, 0.0, 0.0],
                fg_color: [c.r, c.g, c.b, c.a],
                bg_color: [c.r, c.g, c.b, c.a],
            };
            instances.push(rect(x, area_y, width, area_h, TRACK));
            instances.push(rect(x, thumb_y, width, thumb_h, THUMB));
        }

        self.scrollbar_instances = instances;
    }
}
