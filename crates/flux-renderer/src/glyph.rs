//! Per-glyph rendering helper shared by output and input_chrome.
//!
//! Lives in its own module so `set_grid` and `set_input_line` can both
//! call into it without either module needing to know about the other.
//! Mutates `self.atlas` (lazy-loads Unicode glyphs on first use) and
//! appends to the caller's `&mut Vec<CellInstance>`.

use crate::atlas::GlyphStyle;
use crate::cell_renderer::CellInstance;
use crate::renderer::Renderer;
use flux_types::Color;

impl Renderer {
    /// Render a single glyph character at a grid position.
    ///
    /// `cell_x`, `cell_y` = top-left corner of the cell in screen pixels.
    /// `baseline_offset` = distance from cell top to the glyph baseline.
    /// The glyph's `placement_top` is the distance from baseline to top of the bitmap,
    /// `placement_left` is the horizontal offset from the cell's origin.
    //
    // Nine arguments is over clippy's default cap but bundling them into a
    // struct would just move the ceremony to the caller (set_grid /
    // set_input_line each invoke this in a hot loop). The hot-path shape
    // is the right tradeoff here.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn render_glyph(
        &mut self,
        character: char,
        style: GlyphStyle,
        cell_x: f32,
        cell_y: f32,
        baseline_offset: f32,
        fg: Color,
        bg: Color,
        instances: &mut Vec<CellInstance>,
    ) {
        let region = self.atlas.lookup_char(&self.gpu.queue, character, style);
        if region.is_null() {
            return;
        }

        instances.push(CellInstance {
            position: [
                cell_x + region.placement_left,
                cell_y + baseline_offset - region.placement_top,
            ],
            size: [region.pixel_width, region.pixel_height],
            glyph_uv: region.uv,
            fg_color: [fg.r, fg.g, fg.b, fg.a],
            bg_color: [bg.r, bg.g, bg.b, bg.a],
        });
    }
}
