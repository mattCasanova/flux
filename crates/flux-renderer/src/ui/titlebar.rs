//! Custom titlebar strip (macOS unified chrome): the native titlebar
//! is transparent and full-size-content, so this strip paints the
//! chrome color under the traffic lights and hosts the persistent
//! buttons — today, the sidebar panel toggle.

use crate::core::CellInstance;
use crate::renderer::Renderer;
use flux_types::Color;

impl Renderer {
    /// The panel-toggle button's current clickable rect (physical px),
    /// set by `set_titlebar`.
    pub fn titlebar_toggle_rect(&self) -> (f32, f32, f32, f32) {
        self.titlebar_toggle
    }

    /// Rebuild the titlebar strip. `height` is the strip height,
    /// `toggle_x` the button's left edge (both physical px — the app
    /// clears the traffic lights with it).
    pub fn set_titlebar(&mut self, height: f32, toggle_x: f32) {
        let window_w = self.gpu.surface_config.width as f32;
        let mut instances = std::mem::take(&mut self.titlebar_instances);
        instances.clear();

        if height > 0.0 {
            let bg = self.ui.titlebar_bg;
            let rect = |x: f32, y: f32, w: f32, h: f32, c: Color| CellInstance {
                position: [x, y],
                size: [w, h],
                glyph_uv: [0.0, 0.0, 0.0, 0.0],
                fg_color: [c.r, c.g, c.b, c.a],
                bg_color: [c.r, c.g, c.b, c.a],
            };
            instances.push(rect(0.0, 0.0, window_w, height, bg));
            // Hairline under the strip.
            instances.push(rect(0.0, height - 1.0, window_w, 1.0, self.ui.divider));

            // Panel-toggle icon, vertically centered.
            let (gw, gh) = (18.0, 13.0);
            let pad = 6.0;
            let gx = toggle_x + pad;
            let gy = ((height - gh) * 0.5).round();
            instances.push(rect(gx, gy, gw, gh, self.ui.tab_text));
            instances.push(rect(gx + 1.5, gy + 1.5, gw - 3.0, gh - 3.0, bg));
            instances.push(rect(gx + 1.5, gy + 1.5, 5.0, gh - 3.0, self.ui.tab_text));
            self.titlebar_toggle = (gx - pad, 0.0, gw + pad * 2.0, height);
        } else {
            self.titlebar_toggle = (0.0, 0.0, 0.0, 0.0);
        }

        self.titlebar_instances = instances;
        self.rebuild_combined_buffer();
    }
}
