//! Sidebar — the left tab panel (replaces the top tab bar,
//! sidebar-direction.md). Two-line entries: title, then a dim
//! subtitle (branch · folder). A running-command dot sits before the
//! title; the focused entry gets a highlight and accent text. Drawn
//! with the same instanced rects + atlas glyphs as everything else.

use crate::core::CellInstance;
use crate::renderer::Renderer;
use flux_types::Color;

/// One tab's sidebar row, prepared by the app.
#[derive(Debug, Clone, PartialEq)]
pub struct SidebarEntry {
    pub title: String,
    /// Dim second line: "branch · folder" or the cwd.
    pub subtitle: String,
    /// A command is executing somewhere in this tab.
    pub running: bool,
}

/// Vertical padding above the first entry, in pixels.
pub const SIDEBAR_TOP_PAD: f32 = 8.0;
/// Left text inset, in pixels.
const TEXT_INSET: f32 = 10.0;

impl Renderer {
    /// Height of one sidebar entry in pixels.
    pub fn sidebar_entry_height(&self) -> f32 {
        (self.atlas.cell_height * 2.0 + 10.0).round()
    }

    pub fn hide_sidebar(&mut self) {
        if self.tab_instances.is_empty() {
            return;
        }
        self.tab_instances.clear();
        self.rebuild_combined_buffer();
    }

    /// Rebuild the sidebar. `width` is the panel's pixel width; the
    /// panel spans the window's full height.
    pub fn set_sidebar(&mut self, entries: &[SidebarEntry], focused: usize, width: f32) {
        let cell_w = self.atlas.cell_width;
        let cell_h = self.atlas.cell_height;
        let baseline = self.atlas.baseline_offset;
        let style = self.default_style;
        let window_h = self.gpu.surface_config.height as f32;
        let entry_h = self.sidebar_entry_height();

        let mut instances = std::mem::take(&mut self.tab_instances);
        instances.clear();

        if width > 0.0 && !entries.is_empty() {
            let rect = |x: f32, y: f32, w: f32, h: f32, c: Color| CellInstance {
                position: [x, y],
                size: [w, h],
                glyph_uv: [0.0, 0.0, 0.0, 0.0],
                fg_color: [c.r, c.g, c.b, c.a],
                bg_color: [c.r, c.g, c.b, c.a],
            };

            // Panel background, full height.
            instances.push(rect(0.0, 0.0, width, window_h, self.ui.tab_bg));

            let text_cols = (((width - TEXT_INSET * 2.0) / cell_w) as usize).max(4);
            for (idx, entry) in entries.iter().enumerate() {
                let top = SIDEBAR_TOP_PAD + idx as f32 * entry_h;
                if top + entry_h > window_h {
                    break;
                }
                let is_focused = idx == focused;
                if is_focused {
                    instances.push(rect(0.0, top, width, entry_h, self.ui.tab_focused_bg));
                    // Accent sliver on the left edge.
                    instances.push(rect(0.0, top, 2.0, entry_h, self.ui.accent));
                }

                let title_fg = if is_focused {
                    self.ui.tab_focused_text
                } else {
                    self.ui.tab_text
                };
                let bg = if is_focused {
                    self.ui.tab_focused_bg
                } else {
                    self.ui.tab_bg
                };

                // Line 1: running dot + title.
                let title_y = top + (entry_h * 0.5 - cell_h).max(0.0);
                let mut col = 0usize;
                if entry.running {
                    self.render_glyph(
                        '●',
                        style,
                        TEXT_INSET,
                        title_y,
                        baseline,
                        self.ui.sidebar_running,
                        bg,
                        &mut instances,
                    );
                }
                col += 2; // dot column + gap, reserved even when idle
                for ch in truncated(&entry.title, text_cols.saturating_sub(col)) {
                    let x = TEXT_INSET + col as f32 * cell_w;
                    if ch != ' ' {
                        self.render_glyph(
                            ch,
                            style,
                            x,
                            title_y,
                            baseline,
                            title_fg,
                            bg,
                            &mut instances,
                        );
                    }
                    col += 1;
                }

                // Line 2: dim subtitle.
                let sub_y = title_y + cell_h;
                let mut col = 2usize;
                for ch in truncated(&entry.subtitle, text_cols.saturating_sub(col)) {
                    let x = TEXT_INSET + col as f32 * cell_w;
                    if ch != ' ' {
                        self.render_glyph(
                            ch,
                            style,
                            x,
                            sub_y,
                            baseline,
                            self.ui.input_dim,
                            bg,
                            &mut instances,
                        );
                    }
                    col += 1;
                }
            }

            // Hairline between panel and content.
            instances.push(rect(width - 1.0, 0.0, 1.0, window_h, self.ui.divider));
        }

        self.tab_instances = instances;
        self.rebuild_combined_buffer();
    }
}

/// Chars of `text` fitted to `max` columns, ellipsized.
fn truncated(text: &str, max: usize) -> Vec<char> {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= max {
        chars
    } else if max == 0 {
        Vec::new()
    } else {
        let mut out: Vec<char> = chars[..max - 1].to_vec();
        out.push('…');
        out
    }
}
