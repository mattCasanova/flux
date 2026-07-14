//! Output grid rendering — `set_grid`.
//!
//! Takes a `flux_types::TerminalGrid` and rebuilds `output_instances`
//! with per-cell backgrounds, glyphs, and the optional shell cursor
//! block. Handles bottom-anchor y-shift in cooked mode.

use crate::atlas::GlyphStyle;
use crate::core::{CellInstance, color_matches};
use crate::renderer::Renderer;
use flux_types::{CellFlags, Color, TerminalGrid};

/// The most common cell background along the grid perimeter — what an
/// alt-screen program "means" by its background color, robust against
/// individually tinted rows (statuslines, highlighted entries, Claude
/// Code's prompt rows). The full perimeter (not just top/bottom rows)
/// keeps a single full-width statusline from tying the vote.
fn dominant_edge_bg(grid: &TerminalGrid) -> Color {
    let mut counts: std::collections::HashMap<[u8; 4], (usize, Color)> = Default::default();
    let mut tally = |bg: Color| {
        let key = [
            (bg.r * 255.0) as u8,
            (bg.g * 255.0) as u8,
            (bg.b * 255.0) as u8,
            (bg.a * 255.0) as u8,
        ];
        counts.entry(key).or_insert((0, bg)).0 += 1;
    };

    let last_row = grid.rows - 1;
    let last_col = grid.cols - 1;
    for col in 0..grid.cols {
        tally(grid.get(0, col).bg);
        if last_row > 0 {
            tally(grid.get(last_row, col).bg);
        }
    }
    // Side columns, excluding the corners already counted above.
    for row in 1..last_row.max(1) {
        tally(grid.get(row, 0).bg);
        if last_col > 0 {
            tally(grid.get(row, last_col).bg);
        }
    }

    counts
        .into_values()
        .max_by_key(|(count, _)| *count)
        .map(|(_, color)| color)
        .unwrap_or(Color::new(0.0, 0.0, 0.0, 1.0))
}

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
    pub fn set_grid(&mut self, grid: &flux_types::TerminalGrid) {
        let cell_w = self.atlas.cell_width;
        let cell_h = self.atlas.cell_height;
        let baseline = self.atlas.baseline_offset;
        let pad_x = self.padding_x;
        let pad_y = self.padding_y;

        let y_shift_rows = if self.bottom_anchor {
            let anchor_row = grid
                .cursor
                .map(|(_, r)| r)
                .unwrap_or(grid.rows.saturating_sub(1));
            let last_row = grid.rows.saturating_sub(1);
            last_row.saturating_sub(anchor_row)
        } else {
            0
        };
        self.current_y_shift_rows = y_shift_rows;
        let y_shift = y_shift_rows as f32 * cell_h;

        // Padding / clear color policy:
        // - Alt screen: per `alt_bg_policy` — default syncs to the
        //   program's background (majority vote over the grid perimeter,
        //   NOT a single-cell sample: Claude Code tints individual rows
        //   and a corner sample flashed the padding) so vim et al fill
        //   the window edge-to-edge.
        // - Cooked + scrolled into history: optional `scrolled_bg` tint
        //   as a "not at the live tail" cue.
        // - Otherwise: the user's theme background.
        self.effective_clear_color = if !self.bottom_anchor && grid.rows > 0 && grid.cols > 0 {
            match self.alt_bg_policy {
                crate::renderer::AltBgPolicy::Sync => dominant_edge_bg(grid),
                crate::renderer::AltBgPolicy::Theme => self.clear_color,
                crate::renderer::AltBgPolicy::Fixed(color) => color,
            }
        } else if grid.display_offset > 0
            && let Some(scrolled) = self.scrolled_bg
        {
            scrolled
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
                self.render_glyph(
                    cell.character,
                    style,
                    cell_x,
                    cell_y,
                    baseline,
                    fg,
                    bg,
                    &mut instances,
                );
            }
        }

        self.output_instances = instances;
        self.rebuild_combined_buffer();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flux_types::CellData;

    fn grid_with_bgs(rows: usize, cols: usize, default: Color) -> TerminalGrid {
        let mut grid = TerminalGrid::new(cols, rows);
        for r in 0..rows {
            for c in 0..cols {
                grid.set(
                    r,
                    c,
                    CellData {
                        bg: default,
                        ..CellData::default()
                    },
                );
            }
        }
        grid
    }

    #[test]
    fn uniform_background_wins() {
        let bg = Color::new(0.1, 0.2, 0.3, 1.0);
        let grid = grid_with_bgs(24, 80, bg);
        assert_eq!(dominant_edge_bg(&grid), bg);
    }

    #[test]
    fn single_tinted_corner_does_not_flip_the_padding() {
        let bg = Color::new(0.1, 0.2, 0.3, 1.0);
        let tint = Color::new(0.4, 0.3, 0.2, 1.0);
        let mut grid = grid_with_bgs(24, 80, bg);
        // A tinted element scrolled into the top-left corner (the old
        // single-cell sample would have adopted it).
        for c in 0..10 {
            grid.set(
                0,
                c,
                CellData {
                    bg: tint,
                    ..CellData::default()
                },
            );
        }
        assert_eq!(dominant_edge_bg(&grid), bg);
    }

    #[test]
    fn statusline_bottom_row_does_not_outvote_normal_bg() {
        let bg = Color::new(0.1, 0.2, 0.3, 1.0);
        let status = Color::new(0.5, 0.5, 0.5, 1.0);
        let mut grid = grid_with_bgs(24, 80, bg);
        // vim-style full-width statusline on the bottom row: 80 status
        // cells vs 80 top-row cells + 44 side-column cells of Normal —
        // Normal must win decisively.
        for c in 0..80 {
            grid.set(
                23,
                c,
                CellData {
                    bg: status,
                    ..CellData::default()
                },
            );
        }
        assert_eq!(dominant_edge_bg(&grid), bg);
    }
}
