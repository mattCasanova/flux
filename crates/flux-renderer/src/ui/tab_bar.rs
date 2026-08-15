//! Tab bar — one row across the top of the window, shown only when
//! more than one tab exists so a single-tab session keeps today's
//! chrome-free look. The N tabs always split the window width evenly
//! (2 tabs = half each, 4 = a quarter each), so every tab is always
//! visible and clickable — no overflow, no scrolling. Labels render
//! as `N:title`, centered in their slot, truncated with an ellipsis
//! when the slot is narrow. The focused tab gets the accent color and
//! a brighter background across its whole slot.

use crate::core::CellInstance;
use crate::renderer::Renderer;
use flux_types::Color;

/// Colors match the input bar's fixed palette until F16 themes land.
const BAR_BG: Color = Color::new(0.10, 0.11, 0.17, 1.0);
const TAB_FG: Color = Color::new(0.55, 0.60, 0.75, 1.0);
const FOCUSED_BG: Color = Color::new(0.16, 0.18, 0.28, 1.0);
const FOCUSED_FG: Color = Color::new(0.478, 0.635, 0.969, 1.0); // #7aa2f7
const SEPARATOR: Color = Color::new(0.30, 0.33, 0.45, 0.6);

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
            let n = titles.len();
            let slot_w = window_w / n as f32;

            // Full-width bar background.
            instances.push(rect(0.0, 0.0, window_w, cell_h, BAR_BG));

            for (idx, title) in titles.iter().enumerate() {
                let slot_x = idx as f32 * slot_w;
                let is_focused = idx == focused;
                let (fg, bg) = if is_focused {
                    (FOCUSED_FG, FOCUSED_BG)
                } else {
                    (TAB_FG, BAR_BG)
                };
                if is_focused {
                    instances.push(rect(slot_x, 0.0, slot_w, cell_h, bg));
                }

                // `N:title`, truncated to the slot with an ellipsis,
                // one cell of breathing room per side when possible.
                let slot_cols = (slot_w / cell_w) as usize;
                let max_chars = slot_cols.saturating_sub(2).max(1);
                let full: Vec<char> = format!("{}:{}", idx + 1, title).chars().collect();
                let label: Vec<char> = if full.len() > max_chars {
                    let keep = max_chars.saturating_sub(1);
                    full[..keep].iter().copied().chain(['\u{2026}']).collect()
                } else {
                    full
                };
                let label_w = label.len() as f32 * cell_w;
                let mut x = slot_x + ((slot_w - label_w) * 0.5).max(0.0);
                for &ch in &label {
                    if x + cell_w > slot_x + slot_w || x + cell_w > window_w {
                        break;
                    }
                    if ch != ' ' {
                        self.render_glyph(ch, style, x, 0.0, baseline, fg, bg, &mut instances);
                    }
                    x += cell_w;
                }

                // Thin separator on the slot's right edge.
                if idx + 1 < n {
                    instances.push(rect(
                        slot_x + slot_w - 0.5,
                        cell_h * 0.2,
                        1.0,
                        cell_h * 0.6,
                        SEPARATOR,
                    ));
                }
            }
        }

        self.tab_instances = instances;
        self.rebuild_combined_buffer();
    }
}

fn rect(x: f32, y: f32, w: f32, h: f32, c: Color) -> CellInstance {
    CellInstance {
        position: [x, y],
        size: [w, h],
        glyph_uv: [0.0, 0.0, 0.0, 0.0],
        fg_color: [c.r, c.g, c.b, c.a],
        bg_color: [c.r, c.g, c.b, c.a],
    }
}
