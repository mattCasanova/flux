//! Input editor chrome — the fixed prompt line at the bottom of the
//! window. Owns `set_input_line` and `hide_input_line`.
//!
//! The input chrome is Flux-owned UI: divider row, prompt prefix,
//! buffer text, block cursor. It sits at a fixed Y (one cell above
//! the bottom padding) and is re-rendered on every keystroke. It is
//! NOT part of the shell grid — the shell's cursor row is drawn
//! separately by `set_grid`.

use crate::cell_renderer::CellInstance;
use crate::renderer::Renderer;
use flux_types::Color;

impl Renderer {
    /// Clear the fixed input editor chrome from the next frame. Use when
    /// handing the screen to a full-screen program (vim, less, etc).
    pub fn hide_input_line(&mut self) {
        if self.input_instances.is_empty() {
            return;
        }
        self.input_instances.clear();
        self.rebuild_combined_buffer();
    }

    /// Render the fixed input editor chrome at the bottom of the window.
    ///
    /// The input line is Flux chrome — it is not part of the shell grid. It
    /// lives at a fixed Y (one cell above the bottom padding), prefixed by
    /// `❯ `, with a block cursor at `cursor_col` and a dim divider row one
    /// cell above it.
    pub fn set_input_line(&mut self, text: &str, cursor_col: usize) {
        let cell_w = self.atlas.cell_width;
        let cell_h = self.atlas.cell_height;
        let baseline = self.atlas.baseline_offset;
        let pad_x = self.padding_x;
        let pad_y = self.padding_y;
        let window_w = self.gpu.surface_config.width as f32;
        let window_h = self.gpu.surface_config.height as f32;

        let mut instances = std::mem::take(&mut self.input_instances);
        instances.clear();

        // Input line sits on the bottom row (above bottom padding).
        let input_y = window_h - pad_y - cell_h;
        // Divider cell sits one row above the input.
        let divider_y = input_y - cell_h;

        // Dim horizontal rule centered in the divider row.
        let divider_color = Color::new(0.30, 0.33, 0.45, 1.0);
        let divider_thickness = 1.0;
        instances.push(CellInstance {
            position: [pad_x, divider_y + cell_h * 0.5 - divider_thickness * 0.5],
            size: [(window_w - pad_x * 2.0).max(0.0), divider_thickness],
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

        // Draw the prompt prefix in a slightly different color from the text.
        let prompt_color = Color::from_hex("#7aa2f7").unwrap_or_default();
        let fg_color = Color::from_hex("#c0caf5").unwrap_or_default();
        let bg_color = Color::from_hex("#24283b").unwrap_or(Color::new(0.0, 0.0, 0.0, 1.0));
        let cursor_color = Color::from_hex("#c0caf5").unwrap_or_default();

        let prefix = "❯ ";
        let prefix_len = prefix.chars().count();
        let style = self.default_style;
        for (col, ch) in prefix.chars().enumerate() {
            let x = pad_x + col as f32 * cell_w;
            if ch != ' ' {
                self.render_glyph(
                    ch,
                    style,
                    x,
                    input_y,
                    baseline,
                    prompt_color,
                    bg_color,
                    &mut instances,
                );
            }
        }

        // Cursor column in the full input row (after the prefix).
        let cursor_display_col = prefix_len + cursor_col;
        let cursor_x = pad_x + cursor_display_col as f32 * cell_w;

        // Cursor block — drawn before the glyph underneath so the glyph paints on top.
        // Full cell height so it matches the line grid uniformly.
        instances.push(CellInstance {
            position: [cursor_x, input_y],
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

        // Draw the buffer text, inverting the glyph that sits under the cursor.
        for (i, ch) in text.chars().enumerate() {
            let x = pad_x + (prefix_len + i) as f32 * cell_w;
            if ch == ' ' {
                continue;
            }
            let is_under_cursor = i == cursor_col;
            let (fg, bg) = if is_under_cursor {
                (bg_color, cursor_color)
            } else {
                (fg_color, bg_color)
            };
            self.render_glyph(ch, style, x, input_y, baseline, fg, bg, &mut instances);
        }

        self.input_instances = instances;
        self.rebuild_combined_buffer();
    }
}
