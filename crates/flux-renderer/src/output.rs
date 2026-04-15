//! Output grid rendering — `set_grid`.
//!
//! Takes a `flux_types::RenderGrid` and rebuilds `output_instances`
//! with per-cell backgrounds, glyphs, and the optional shell cursor
//! block. Handles bottom-anchor y-shift in cooked mode.

use crate::atlas::GlyphStyle;
use crate::buffer::color_matches;
use crate::cell_renderer::CellInstance;
use crate::renderer::Renderer;
use flux_types::{CellFlags, Color};

impl Renderer {
    /// Render a terminal grid — each cell at its grid position.
    ///
    /// The grid is bottom-anchored on the shell's cursor row: whichever row
    /// the shell is currently writing to is placed at the bottom of the
    /// output area, and earlier rows stack above it. Before the shell has
    /// produced enough output to fill the grid, the top of the output area
    /// is just blank. Once the shell overflows the grid and alacritty_terminal
    /// starts scrolling, the anchor is at row `rows-1` and behavior matches a
    /// normal top-anchored terminal.
    ///
    /// The shell's own cursor block is intentionally not drawn — Flux owns
    /// input via the fixed editor below, so the shell cursor is redundant
    /// noise. (Raw-mode bypass will need to re-enable it — see the raw-mode
    /// item on Phase 1.)
    pub fn set_grid(&mut self, grid: &flux_types::RenderGrid) {
        let cell_w = self.atlas.cell_width;
        let cell_h = self.atlas.cell_height;
        let baseline = self.atlas.baseline_offset;
        let pad_x = self.padding_x;
        let pad_y = self.padding_y;

        let y_shift = if self.bottom_anchor {
            let anchor_row = grid
                .cursor
                .map(|(_, r)| r)
                .unwrap_or(grid.rows.saturating_sub(1));
            let last_row = grid.rows.saturating_sub(1);
            (last_row.saturating_sub(anchor_row)) as f32 * cell_h
        } else {
            0.0
        };

        // In raw mode, sync the effective clear to whatever bg the alt-screen
        // program is using — sampled from the top-left cell, which reliably
        // carries the Normal highlight bg for vim, nano, less, etc. In cooked
        // mode, reset to the user's configured theme bg.
        self.effective_clear_color = if !self.bottom_anchor && grid.rows > 0 && grid.cols > 0 {
            grid.get(0, 0).bg
        } else {
            self.clear_color
        };

        let mut instances = std::mem::take(&mut self.output_instances);
        instances.clear();

        // Draw the shell's cursor block first (so any underlying glyph paints on top).
        // Uses the full cell height so the cursor matches the line grid
        // uniformly regardless of which glyph sits under it.
        if self.show_shell_cursor
            && let Some((col, row)) = grid.cursor
        {
            let cursor_x = pad_x + col as f32 * cell_w;
            let cursor_y = pad_y + row as f32 * cell_h + y_shift;
            let cursor_color = Color::from_hex("#c0caf5").unwrap_or_default();
            instances.push(CellInstance {
                position: [cursor_x, cursor_y],
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

        let clear = self.effective_clear_color;

        for row in 0..grid.rows {
            for col in 0..grid.cols {
                let cell = grid.get(row, col);
                let cell_x = pad_x + col as f32 * cell_w;
                let cell_y = pad_y + row as f32 * cell_h + y_shift;
                let is_under_cursor = self.show_shell_cursor && grid.cursor == Some((col, row));

                // Paint the cell's background across the whole cell rect
                // when it differs from the effective clear color. Skip the
                // cursor cell — the cursor block drawn earlier is already
                // filling that cell, and overdrawing it here would erase the
                // cursor and leave only the small inverted-color glyph rect.
                if !is_under_cursor && !color_matches(cell.bg, clear) {
                    instances.push(CellInstance {
                        position: [cell_x, cell_y],
                        size: [cell_w, cell_h],
                        glyph_uv: [0.0, 0.0, 0.0, 0.0],
                        fg_color: [cell.bg.r, cell.bg.g, cell.bg.b, cell.bg.a],
                        bg_color: [cell.bg.r, cell.bg.g, cell.bg.b, cell.bg.a],
                    });
                }

                if cell.character == ' ' || cell.character == '\0' {
                    continue;
                }

                // When a cell has no style flags, fall back to the config-
                // level default — this is how `[font] weight` / `style` act
                // as a terminal-wide baseline (matching iTerm's profile
                // "default font" behavior). When the shell or an alt-screen
                // program sets BOLD or ITALIC explicitly, those win, which
                // lets vim/nano render a differentiated statusline against
                // an italic-baseline welcome screen.
                let has_flags = cell.flags.intersects(CellFlags::BOLD | CellFlags::ITALIC);
                let style = if has_flags {
                    GlyphStyle::from_flags(
                        cell.flags.contains(CellFlags::BOLD),
                        cell.flags.contains(CellFlags::ITALIC),
                    )
                } else {
                    self.default_style
                };

                // When the shell cursor sits on a glyph, invert its colors so it
                // reads against the cursor block (already pushed above).
                let (fg, bg) = if is_under_cursor {
                    let cursor_bg =
                        Color::from_hex("#24283b").unwrap_or(Color::new(0.0, 0.0, 0.0, 1.0));
                    let cursor_fg = Color::from_hex("#c0caf5").unwrap_or_default();
                    (cursor_bg, cursor_fg)
                } else {
                    (cell.fg, cell.bg)
                };
                self.render_glyph(cell.character, style, cell_x, cell_y, baseline, fg, bg, &mut instances);
            }
        }

        self.output_instances = instances;
        self.rebuild_combined_buffer();
    }
}
