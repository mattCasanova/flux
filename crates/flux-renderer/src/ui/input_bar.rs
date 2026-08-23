//! Per-pane input bars — the prompt area at the bottom of each cooked
//! pane (per-split input, 2026-08-23). Each bar renders a divider row,
//! `❯ ` prompt prefix (with `  ` continuation indent for multi-line),
//! the pane's editor buffer, and — on the focused pane only — the
//! block cursor. Unfocused bars draw dim. Alt-screen panes have no bar
//! (their pane's chrome_rows is 0).

use crate::core::CellInstance;
use crate::renderer::Renderer;
use flux_types::Color;

/// Prompt prefix for line 0.
const PROMPT: &str = "❯ ";
/// Continuation indent for lines 1+. Same width as the prompt so
/// columns stay aligned.
const CONTINUATION: &str = "  ";
/// Character width of both prefixes (must match).
const PREFIX_CHARS: usize = 2;

/// One pane's input bar, positioned by the app's layout.
#[derive(Debug, Clone)]
pub struct InputBar {
    /// Top-left pixel of the bar (divider row).
    pub origin: [f32; 2],
    /// Bar width in pixels (the pane's width).
    pub width: f32,
    /// Editor buffer (may hold `\n`).
    pub text: String,
    /// Cursor `(row, col)` in character coordinates — Some only on the
    /// focused pane's bar.
    pub cursor: Option<(usize, usize)>,
}

impl Renderer {
    /// Clear all input bars from the next frame (app shutdown paths).
    pub fn hide_input_bar(&mut self) {
        if self.input_instances.is_empty() {
            return;
        }
        self.input_instances.clear();
        self.rebuild_combined_buffer();
    }

    /// Rebuild every pane's input bar.
    pub fn set_input_bars(&mut self, bars: &[InputBar]) {
        let cell_w = self.atlas.cell_width;
        let cell_h = self.atlas.cell_height;
        let baseline = self.atlas.baseline_offset;
        let style = self.default_style;

        let mut instances = std::mem::take(&mut self.input_instances);
        instances.clear();

        let divider_color = Color::new(0.30, 0.33, 0.45, 1.0);
        let prompt_color = Color::from_hex("#7aa2f7").unwrap_or_default();
        let fg_color = Color::from_hex("#c0caf5").unwrap_or_default();
        let dim_color = Color::from_hex("#6a7099").unwrap_or_default();
        let bg_color = Color::from_hex("#24283b").unwrap_or(Color::new(0.0, 0.0, 0.0, 1.0));
        let cursor_color = Color::from_hex("#c0caf5").unwrap_or_default();

        for bar in bars {
            let [bar_x, bar_y] = bar.origin;
            let focused = bar.cursor.is_some();
            let lines: Vec<&str> = bar.text.split('\n').collect();
            let max_cols = (bar.width / cell_w) as usize;

            // Divider — dim thin horizontal rule across the pane.
            let divider_thickness = 1.0;
            instances.push(CellInstance {
                position: [bar_x, bar_y + cell_h * 0.5 - divider_thickness * 0.5],
                size: [bar.width.max(0.0), divider_thickness],
                glyph_uv: [0.0, 0.0, 0.0, 0.0],
                fg_color: [
                    divider_color.r,
                    divider_color.g,
                    divider_color.b,
                    divider_color.a,
                ],
                bg_color: [
                    divider_color.r,
                    divider_color.g,
                    divider_color.b,
                    divider_color.a,
                ],
            });

            let (prompt_fg, text_fg) = if focused {
                (prompt_color, fg_color)
            } else {
                (dim_color, dim_color)
            };

            for (line_idx, line_text) in lines.iter().enumerate() {
                let line_y = bar_y + (1 + line_idx) as f32 * cell_h;
                let prefix = if line_idx == 0 { PROMPT } else { CONTINUATION };

                for (i, ch) in prefix.chars().enumerate() {
                    if i >= max_cols {
                        break;
                    }
                    let x = bar_x + (i as f32) * cell_w;
                    if ch != ' ' {
                        self.render_glyph(
                            ch,
                            style,
                            x,
                            line_y,
                            baseline,
                            prompt_fg,
                            bg_color,
                            &mut instances,
                        );
                    }
                }

                // Cursor block — pushed before the glyph so the glyph
                // paints on top with inverted colors.
                let cursor_here = bar.cursor.filter(|&(row, _)| row == line_idx);
                if let Some((_, cursor_col)) = cursor_here {
                    let cx = bar_x + (PREFIX_CHARS + cursor_col) as f32 * cell_w;
                    instances.push(CellInstance {
                        position: [cx, line_y],
                        size: [cell_w, cell_h],
                        glyph_uv: [0.0, 0.0, 0.0, 0.0],
                        fg_color: [
                            cursor_color.r,
                            cursor_color.g,
                            cursor_color.b,
                            cursor_color.a,
                        ],
                        bg_color: [
                            cursor_color.r,
                            cursor_color.g,
                            cursor_color.b,
                            cursor_color.a,
                        ],
                    });
                }

                for (i, ch) in line_text.chars().enumerate() {
                    if PREFIX_CHARS + i >= max_cols {
                        break;
                    }
                    let x = bar_x + (PREFIX_CHARS + i) as f32 * cell_w;
                    if ch == ' ' {
                        continue;
                    }
                    let is_under_cursor = cursor_here.is_some_and(|(_, col)| col == i);
                    let (fg, bg) = if is_under_cursor {
                        (bg_color, cursor_color)
                    } else {
                        (text_fg, bg_color)
                    };
                    self.render_glyph(ch, style, x, line_y, baseline, fg, bg, &mut instances);
                }
            }
        }

        self.input_instances = instances;
        self.rebuild_combined_buffer();
    }
}
