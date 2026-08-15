//! Tab bar — one row across the top of the window, shown only when
//! more than one tab exists so a single-tab session keeps today's
//! chrome-free look. Labels render as ` N:title `; the focused tab
//! gets the accent color and a brighter background.

use crate::core::CellInstance;
use crate::renderer::Renderer;
use flux_types::Color;

/// Colors match the input bar's fixed palette until F16 themes land.
const BAR_BG: Color = Color::new(0.10, 0.11, 0.17, 1.0);
const TAB_FG: Color = Color::new(0.55, 0.60, 0.75, 1.0);
const FOCUSED_BG: Color = Color::new(0.16, 0.18, 0.28, 1.0);
const FOCUSED_FG: Color = Color::new(0.478, 0.635, 0.969, 1.0); // #7aa2f7

impl Renderer {
    /// Height of the tab bar in pixels when `count` tabs exist.
    pub fn tab_bar_height(&self, count: usize) -> f32 {
        if count > 1 {
            self.atlas.cell_height
        } else {
            0.0
        }
    }

    pub fn hide_tab_bar(&mut self) {
        if self.tab_instances.is_empty() {
            return;
        }
        self.tab_instances.clear();
        self.rebuild_combined_buffer();
    }

    /// Rebuild the tab bar. `titles` are the labels in order; hidden
    /// entirely when fewer than two tabs exist.
    pub fn set_tab_bar(&mut self, titles: &[String], focused: usize) {
        let mut instances = std::mem::take(&mut self.tab_instances);
        instances.clear();

        if titles.len() > 1 {
            let cell_w = self.atlas.cell_width;
            let cell_h = self.atlas.cell_height;
            let baseline = self.atlas.baseline_offset;
            let window_w = self.gpu.surface_config.width as f32;
            let style = self.default_style;

            // Full-width bar background.
            instances.push(CellInstance {
                position: [0.0, 0.0],
                size: [window_w, cell_h],
                glyph_uv: [0.0, 0.0, 0.0, 0.0],
                fg_color: [BAR_BG.r, BAR_BG.g, BAR_BG.b, BAR_BG.a],
                bg_color: [BAR_BG.r, BAR_BG.g, BAR_BG.b, BAR_BG.a],
            });

            let mut col = 0usize;
            for (idx, title) in titles.iter().enumerate() {
                let label = format!(" {}:{} ", idx + 1, title);
                let is_focused = idx == focused;
                let (fg, bg) = if is_focused {
                    (FOCUSED_FG, FOCUSED_BG)
                } else {
                    (TAB_FG, BAR_BG)
                };
                if is_focused {
                    instances.push(CellInstance {
                        position: [col as f32 * cell_w, 0.0],
                        size: [label.chars().count() as f32 * cell_w, cell_h],
                        glyph_uv: [0.0, 0.0, 0.0, 0.0],
                        fg_color: [bg.r, bg.g, bg.b, bg.a],
                        bg_color: [bg.r, bg.g, bg.b, bg.a],
                    });
                }
                for ch in label.chars() {
                    let x = col as f32 * cell_w;
                    if x + cell_w > window_w {
                        break;
                    }
                    if ch != ' ' {
                        self.render_glyph(ch, style, x, 0.0, baseline, fg, bg, &mut instances);
                    }
                    col += 1;
                }
            }
        }

        self.tab_instances = instances;
        self.rebuild_combined_buffer();
    }
}
