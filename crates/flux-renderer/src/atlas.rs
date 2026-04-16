//! Glyph atlas — rasterizes glyphs with cosmic-text and packs them
//! into a GPU texture with etagere.
//!
//! Pattern: same as a 2D game engine's sprite atlas.
//! - One GPU texture holds all glyphs, packed by etagere
//! - char → GlyphRegion lookup returns the texture UV rect for that character
//! - ASCII uses a direct array lookup (fast path)
//! - Unicode uses a HashMap fallback (CJK, emoji, box drawing, etc.)
//! - Common ASCII is pre-rasterized at startup
//!
//! The only reason font size lives in the atlas is that fonts are vector data
//! (TrueType/OpenType). We have to rasterize each glyph at a specific pixel
//! size to get bitmap data the GPU can sample. A sprite atlas doesn't need
//! this because sprites are already pixels.

mod measure;
mod raster;
mod region;
mod style;
mod texture;

pub use style::GlyphStyle;

pub(crate) use region::{GlyphRegion, NULL_REGION};
pub(crate) use style::STYLE_COUNT;

use std::collections::HashMap;

use anyhow::Result;
use cosmic_text::{FontSystem, Metrics, SwashCache};
use etagere::{size2, BucketedAtlasAllocator};

pub(crate) const ASCII_COUNT: usize = 128;
pub(crate) const ASCII_RANGE: std::ops::Range<u32> = 32..127;

/// Manages the glyph texture atlas.
///
/// Caches each glyph independently under four styles (Regular, Bold,
/// Italic, BoldItalic). Cells coming out of alacritty_terminal carry
/// `CellFlags::BOLD` / `CellFlags::ITALIC`, which the renderer converts into
/// a `GlyphStyle` and uses to pick the right per-style slot. Without this,
/// vim/nano colorschemes look flat because all the weight distinctions are
/// missing — see issue #18.
pub(crate) struct GlyphAtlas {
    // GPU state
    pub(crate) texture: wgpu::Texture,
    pub texture_view: wgpu::TextureView,
    pub(crate) allocator: BucketedAtlasAllocator,

    // Rasterization backend (cosmic-text)
    pub(crate) font_system: FontSystem,
    pub(crate) swash_cache: SwashCache,
    pub(crate) font_family: String,
    pub(crate) font_size: f32,

    // Per-style glyph caches — four flat ASCII arrays and four HashMaps.
    // Indexed by `GlyphStyle as usize`.
    pub(crate) ascii_regions: [[GlyphRegion; ASCII_COUNT]; STYLE_COUNT],
    pub(crate) unicode_regions: [HashMap<char, GlyphRegion>; STYLE_COUNT],

    // Cell dimensions (for the renderer's layout calculations)
    pub cell_width: f32,
    pub cell_height: f32,
    /// Baseline offset from the top of a cell, in pixels.
    /// Glyph baseline = cell_top + baseline_offset.
    pub baseline_offset: f32,
}

impl GlyphAtlas {
    /// Create a new glyph atlas and pre-rasterize common ASCII characters
    /// in all four styles.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        font_family: &str,
        font_size: f32,
        line_height: f32,
    ) -> Result<Self> {
        let mut font_system = FontSystem::new();
        let swash_cache = SwashCache::new();

        let line_height_px = font_size * line_height;
        let metrics = Metrics::new(font_size, line_height_px);
        let (cell_width, cell_height, baseline_offset) =
            measure::measure_cell(&mut font_system, font_family, &metrics);

        let texture = texture::create_atlas_texture(device);
        texture::clear_atlas_texture(queue, &texture);
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let allocator =
            BucketedAtlasAllocator::new(size2(texture::ATLAS_SIZE as i32, texture::ATLAS_SIZE as i32));

        let mut atlas = Self {
            texture,
            texture_view,
            allocator,
            font_system,
            swash_cache,
            font_family: font_family.to_string(),
            font_size,
            ascii_regions: [[NULL_REGION; ASCII_COUNT]; STYLE_COUNT],
            unicode_regions: std::array::from_fn(|_| HashMap::new()),
            cell_width,
            cell_height,
            baseline_offset,
        };

        raster::preload_ascii(&mut atlas, queue);
        Ok(atlas)
    }

    /// Direct `(char, style)` → GlyphRegion lookup. Returns `NULL_REGION`
    /// for anything that doesn't render (control chars, spaces, missing
    /// glyphs). Fast path (ASCII): one branch + one array access, no
    /// hashing. Slow path (Unicode): HashMap lookup with lazy rasterization.
    pub fn lookup_char(
        &mut self,
        queue: &wgpu::Queue,
        ch: char,
        style: GlyphStyle,
    ) -> GlyphRegion {
        let style_idx = style as usize;
        let code = ch as u32;

        if (code as usize) < ASCII_COUNT {
            return self.ascii_regions[style_idx][code as usize];
        }

        if let Some(&region) = self.unicode_regions[style_idx].get(&ch) {
            return region;
        }
        let region = raster::rasterize(self, queue, ch, style).unwrap_or(NULL_REGION);
        self.unicode_regions[style_idx].insert(ch, region);
        region
    }
}
