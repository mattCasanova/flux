//! Input bar — the fixed prompt area at the bottom of the window.
//!
//! Renders the divider row, `❯ ` prompt prefix (with `  ` continuation
//! indent for multi-line), editor buffer text, and block cursor.
//! Grows vertically as the buffer gains lines.

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

impl Renderer {
    /// Clear the input bar from the next frame. Used when handing the
    /// screen to a full-screen program (vim, less, etc).
    pub fn hide_input_bar(&mut self) {
        if self.input_instances.is_empty() {
            return;
        }
        self.input_instances.clear();
        self.rebuild_combined_buffer();
    }

    /// Render the input bar at the bottom of the window.
    ///
    /// `text` may contain `\n` for multi-line commands. The bar grows
    /// upward from the bottom: line 0 (with `❯ ` prompt) is at the
    /// bottom, continuation lines stack above it. The divider sits one
    /// row above the topmost input line.
    ///
    /// `cursor` is `(row, col)` in character coordinates within the
    /// text block.
    pub fn set_input_block(&mut self, text: &str, cursor: (usize, usize)) {
        let cell_w = self.atlas.cell_width;
        let cell_h = self.atlas.cell_height;
        let baseline = self.atlas.baseline_offset;
        let pad_x = self.padding_x;
        let pad_y = self.padding_y;
        let window_w = self.gpu.surface_config.width as f32;
        let window_h = self.gpu.surface_config.height as f32;

        let lines: Vec<&str> = text.split('\n').collect();
        let line_count = lines.len().max(1);

        // The input block's bottom row sits one cell above the bottom padding.
        // Earlier lines stack upward from there.
        let block_bottom_y = window_h - pad_y - cell_h;
        let block_top_y = block_bottom_y - (line_count as f32 - 1.0) * cell_h;
        let divider_y = block_top_y - cell_h;

        let mut instances = std::mem::take(&mut self.input_instances);
        instances.clear();

        // Divider — dim thin horizontal rule.
        let divider_color = Color::new(0.30, 0.33, 0.45, 1.0);
        let divider_thickness = 1.0;
        instances.push(CellInstance {
            position: [pad_x, divider_y + cell_h * 0.5 - divider_thickness * 0.5],
            size: [(window_w - pad_x * 2.0).max(0.0), divider_thickness],
            glyph_uv: [0.0, 0.0, 0.0, 0.0],
            fg_color: [divider_color.r, divider_color.g, divider_color.b, divider_color.a],
            bg_color: [divider_color.r, divider_color.g, divider_color.b, divider_color.a],
        });

        let prompt_color = Color::from_hex("#7aa2f7").unwrap_or_default();
        let fg_color = Color::from_hex("#c0caf5").unwrap_or_default();
        let bg_color = Color::from_hex("#24283b").unwrap_or(Color::new(0.0, 0.0, 0.0, 1.0));
        let cursor_color = Color::from_hex("#c0caf5").unwrap_or_default();
        let (cursor_row, cursor_col) = cursor;
        let style = self.default_style;

        for (line_idx, line_text) in lines.iter().enumerate() {
            let line_y = block_top_y + (line_idx as f32) * cell_h;
            let prefix = if line_idx == 0 { PROMPT } else { CONTINUATION };

            // Draw the prefix.
            for (i, ch) in prefix.chars().enumerate() {
                let x = pad_x + (i as f32) * cell_w;
                if ch != ' ' {
                    self.render_glyph(
                        ch, style, x, line_y, baseline, prompt_color, bg_color, &mut instances,
                    );
                }
            }

            // Cursor block — pushed before the glyph so the glyph paints
            // on top with inverted colors.
            if cursor_row == line_idx {
                let cx = pad_x + (PREFIX_CHARS + cursor_col) as f32 * cell_w;
                instances.push(CellInstance {
                    position: [cx, line_y],
                    size: [cell_w, cell_h],
                    glyph_uv: [0.0, 0.0, 0.0, 0.0],
                    fg_color: [cursor_color.r, cursor_color.g, cursor_color.b, cursor_color.a],
                    bg_color: [cursor_color.r, cursor_color.g, cursor_color.b, cursor_color.a],
                });
            }

            // Draw the line text.
            for (i, ch) in line_text.chars().enumerate() {
                let x = pad_x + (PREFIX_CHARS + i) as f32 * cell_w;
                if ch == ' ' {
                    continue;
                }
                let is_under_cursor = cursor_row == line_idx && cursor_col == i;
                let (fg, bg) = if is_under_cursor {
                    (bg_color, cursor_color)
                } else {
                    (fg_color, bg_color)
                };
                self.render_glyph(ch, style, x, line_y, baseline, fg, bg, &mut instances);
            }
        }

        self.input_instances = instances;
        self.rebuild_combined_buffer();
    }
}
