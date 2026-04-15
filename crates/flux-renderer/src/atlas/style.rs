//! `GlyphStyle` — one of four independently-cached glyph styles
//! (Regular, Bold, Italic, BoldItalic). Used as an index into the
//! per-style arrays on `GlyphAtlas`.

/// One of the four glyph styles we cache independently. Keyed as a
/// `usize` so it can index directly into the atlas' per-style slot arrays.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
#[repr(usize)]
pub enum GlyphStyle {
    Regular = 0,
    Bold = 1,
    Italic = 2,
    BoldItalic = 3,
}

pub(crate) const STYLE_COUNT: usize = 4;

impl GlyphStyle {
    /// All four styles in the canonical order matching `GlyphStyle as usize`.
    pub const ALL: [GlyphStyle; 4] = [
        GlyphStyle::Regular,
        GlyphStyle::Bold,
        GlyphStyle::Italic,
        GlyphStyle::BoldItalic,
    ];

    pub fn from_flags(bold: bool, italic: bool) -> Self {
        match (bold, italic) {
            (false, false) => GlyphStyle::Regular,
            (true, false) => GlyphStyle::Bold,
            (false, true) => GlyphStyle::Italic,
            (true, true) => GlyphStyle::BoldItalic,
        }
    }

    pub fn is_bold(self) -> bool {
        matches!(self, GlyphStyle::Bold | GlyphStyle::BoldItalic)
    }

    pub fn is_italic(self) -> bool {
        matches!(self, GlyphStyle::Italic | GlyphStyle::BoldItalic)
    }
}
