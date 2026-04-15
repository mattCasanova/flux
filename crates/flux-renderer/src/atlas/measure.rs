//! Cell dimension measurement — runs once at atlas construction to
//! figure out how wide an "average" glyph is in the user's monospace
//! font. We assume the font has identical advance widths across styles,
//! which is true for every sane monospace font.

use cosmic_text::{Attrs, Buffer, Family, FontSystem, Metrics};

/// Defensive fallback for cell width when font shaping fails — rough average
/// ratio of glyph advance to em-size for common monospace fonts.
const FALLBACK_CELL_WIDTH_RATIO: f32 = 0.6;

/// Defensive fallback for baseline position when font shaping fails — typical
/// baseline sits roughly 80% of the way down the line box for Latin fonts.
const FALLBACK_BASELINE_RATIO: f32 = 0.8;

/// Measure cell dimensions and baseline from the font's metrics, using
/// the Regular style. We assume the monospace font has identical advance
/// widths across styles — true for every sane monospace font.
/// Returns `(cell_width, cell_height, baseline_offset)`.
/// `baseline_offset` is the distance from the top of a cell to the glyph baseline,
/// adjusted so the glyph box is vertically centered within the line height.
pub(crate) fn measure_cell(
    font_system: &mut FontSystem,
    font_family: &str,
    metrics: &Metrics,
) -> (f32, f32, f32) {
    let mut buffer = Buffer::new(font_system, *metrics);
    let attrs = Attrs::new().family(Family::Name(font_family));
    buffer.set_text(font_system, "M", &attrs, cosmic_text::Shaping::Advanced, None);
    buffer.shape_until_scroll(font_system, false);

    let mut cell_width = metrics.font_size * FALLBACK_CELL_WIDTH_RATIO;
    let mut baseline_offset = metrics.line_height * FALLBACK_BASELINE_RATIO;

    if let Some(run) = buffer.layout_runs().next() {
        if let Some(glyph) = run.glyphs.first() {
            cell_width = glyph.w;
        }
        // run.line_y is the baseline from line_top (already centered within line_height
        // by cosmic-text's layout algorithm, which accounts for font ascent/descent)
        baseline_offset = run.line_y - run.line_top;
    }

    let cell_height = metrics.line_height;

    log::info!(
        "Cell metrics: {:.1}x{:.1} baseline={:.1} (font: {}, size: {})",
        cell_width,
        cell_height,
        baseline_offset,
        font_family,
        metrics.font_size
    );

    (cell_width, cell_height, baseline_offset)
}
