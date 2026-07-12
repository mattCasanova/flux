//! Selection highlight rendering — `set_selection`.
//!
//! Pushes one translucent rect per selected cell into
//! `selection_instances`. The pipeline already runs standard alpha
//! blending (`BlendState::ALPHA_BLENDING`, pipeline.rs), so a 30%-alpha
//! rect composites over the output glyphs without shader changes.

use crate::core::CellInstance;
use crate::renderer::Renderer;
use flux_types::{Color, Selection};

/// Selection tint — theme blue at 30% alpha. Becomes
/// `theme.ui.selection_bg` when F16 lands.
const SELECTION_TINT: Color = Color::new(0.478, 0.635, 0.969, 0.30); // #7aa2f7

impl Renderer {
    /// Rebuild the selection overlay. Pass `None` to clear. Cell
    /// positions use the same padding + bottom-anchor shift as the
    /// output grid so the highlight sits exactly over the text.
    pub fn set_selection(&mut self, selection: Option<&Selection>, grid_cols: usize) {
        let mut instances = std::mem::take(&mut self.selection_instances);
        instances.clear();

        if let Some(sel) = selection {
            let cell_w = self.atlas.cell_width;
            let cell_h = self.atlas.cell_height;
            let y_shift = self.current_y_shift_rows as f32 * cell_h;
            let tint = SELECTION_TINT;

            for pos in sel.cells(grid_cols) {
                let x = self.padding_x + pos.col as f32 * cell_w;
                let y = self.padding_y + pos.row as f32 * cell_h + y_shift;
                instances.push(CellInstance {
                    position: [x, y],
                    size: [cell_w, cell_h],
                    glyph_uv: [0.0, 0.0, 0.0, 0.0],
                    fg_color: [tint.r, tint.g, tint.b, tint.a],
                    bg_color: [tint.r, tint.g, tint.b, tint.a],
                });
            }
        }

        self.selection_instances = instances;
        self.rebuild_combined_buffer();
    }
}
