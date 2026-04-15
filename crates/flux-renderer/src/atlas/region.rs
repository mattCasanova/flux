//! `GlyphRegion` — the UV + placement data the shader needs to draw a
//! single glyph out of the atlas. Null-object pattern: a region with
//! `pixel_width == 0.0` means "nothing to render" (control chars,
//! spaces, missing glyphs).

/// UV coordinates for a glyph in the atlas (normalized 0-1).
///
/// `pixel_width == 0.0` is the null-object sentinel — means "nothing to
/// render here" (used for control chars, spaces, and missing glyphs).
/// The renderer treats any region with zero pixel_width as a skip.
#[derive(Debug, Copy, Clone)]
pub(crate) struct GlyphRegion {
    pub uv: [f32; 4], // [u, v, width, height] in normalized coords
    pub placement_left: f32,
    pub placement_top: f32,
    pub pixel_width: f32,
    pub pixel_height: f32,
}

/// The null-object glyph region — renders nothing.
/// Used for control characters, spaces, and missing font glyphs.
pub(crate) const NULL_REGION: GlyphRegion = GlyphRegion {
    uv: [0.0, 0.0, 0.0, 0.0],
    placement_left: 0.0,
    placement_top: 0.0,
    pixel_width: 0.0,
    pixel_height: 0.0,
};

impl GlyphRegion {
    /// True if this region renders nothing (null object).
    pub fn is_null(&self) -> bool {
        self.pixel_width == 0.0
    }
}
