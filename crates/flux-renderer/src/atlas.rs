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

use std::collections::HashMap;

use anyhow::Result;
use cosmic_text::{
    Attrs, Buffer, CacheKey, Family, FontSystem, Metrics, SwashCache, SwashContent,
};
use etagere::{size2, BucketedAtlasAllocator};

const ATLAS_SIZE: u32 = 1024;
const ASCII_RANGE: std::ops::Range<u32> = 32..127;

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

/// Manages the glyph texture atlas.
pub(crate) struct GlyphAtlas {
    // GPU state
    texture: wgpu::Texture,
    pub texture_view: wgpu::TextureView,
    allocator: BucketedAtlasAllocator,

    // Rasterization backend (cosmic-text)
    font_system: FontSystem,
    swash_cache: SwashCache,
    font_family: String,
    font_size: f32,
    bold: bool,

    // Glyph caches — direct char → GlyphRegion lookup
    // ASCII slots default to NULL_REGION (null object pattern — no Option overhead).
    ascii_regions: [GlyphRegion; 128],
    unicode_regions: HashMap<char, GlyphRegion>,

    // Cell dimensions (for the renderer's layout calculations)
    pub cell_width: f32,
    pub cell_height: f32,
}

impl GlyphAtlas {
    /// Create a new glyph atlas and pre-rasterize common ASCII characters.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        font_family: &str,
        font_size: f32,
        line_height: f32,
        bold: bool,
    ) -> Result<Self> {
        let mut font_system = FontSystem::new();
        let swash_cache = SwashCache::new();

        let line_height_px = font_size * line_height;
        let metrics = Metrics::new(font_size, line_height_px);
        let (cell_width, cell_height) =
            Self::measure_cell(&mut font_system, font_family, bold, &metrics);

        let texture = Self::create_atlas_texture(device);
        Self::clear_atlas_texture(queue, &texture);
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let allocator = BucketedAtlasAllocator::new(size2(ATLAS_SIZE as i32, ATLAS_SIZE as i32));

        let mut atlas = Self {
            texture,
            texture_view,
            allocator,
            font_system,
            swash_cache,
            font_family: font_family.to_string(),
            font_size,
            bold,
            ascii_regions: [NULL_REGION; 128],
            unicode_regions: HashMap::new(),
            cell_width,
            cell_height,
        };

        atlas.preload_ascii(queue);

        Ok(atlas)
    }

    /// Direct char → GlyphRegion lookup. Returns NULL_REGION for anything
    /// that doesn't render (control chars, spaces, missing glyphs).
    ///
    /// Fast path (ASCII): one branch + one array access, no hashing.
    /// Slow path (Unicode): HashMap lookup with lazy rasterization.
    pub fn lookup_char(&mut self, queue: &wgpu::Queue, ch: char) -> GlyphRegion {
        let code = ch as u32;

        // Fast path: ASCII direct array lookup
        if code < 128 {
            return self.ascii_regions[code as usize];
        }

        // Slow path: Unicode HashMap with lazy caching
        if let Some(&region) = self.unicode_regions.get(&ch) {
            return region;
        }
        let region = self.rasterize(queue, ch).unwrap_or(NULL_REGION);
        self.unicode_regions.insert(ch, region);
        region
    }
}

// --- Private helpers (construction) ---

impl GlyphAtlas {
    fn create_atlas_texture(device: &wgpu::Device) -> wgpu::Texture {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Glyph Atlas"),
            size: wgpu::Extent3d {
                width: ATLAS_SIZE,
                height: ATLAS_SIZE,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        })
    }

    fn clear_atlas_texture(queue: &wgpu::Queue, texture: &wgpu::Texture) {
        let zeros = vec![0u8; (ATLAS_SIZE * ATLAS_SIZE) as usize];
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &zeros,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(ATLAS_SIZE),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: ATLAS_SIZE,
                height: ATLAS_SIZE,
                depth_or_array_layers: 1,
            },
        );
    }

    /// Measure cell dimensions from the font's monospace advance width.
    fn measure_cell(
        font_system: &mut FontSystem,
        font_family: &str,
        bold: bool,
        metrics: &Metrics,
    ) -> (f32, f32) {
        let mut buffer = Buffer::new(font_system, *metrics);
        let mut attrs = Attrs::new().family(Family::Name(font_family));
        if bold {
            attrs = attrs.weight(cosmic_text::fontdb::Weight::BOLD);
        }
        buffer.set_text(font_system, "M", &attrs, cosmic_text::Shaping::Advanced, None);
        buffer.shape_until_scroll(font_system, false);

        let mut cell_width = metrics.font_size * 0.6; // fallback
        if let Some(run) = buffer.layout_runs().next() {
            if let Some(glyph) = run.glyphs.first() {
                cell_width = glyph.w;
            }
        }

        let cell_height = metrics.line_height;

        log::info!(
            "Cell metrics: {:.1}x{:.1} (font: {}, size: {})",
            cell_width,
            cell_height,
            font_family,
            metrics.font_size
        );

        (cell_width, cell_height)
    }

    /// Pre-rasterize printable ASCII (32-126) to avoid first-frame hitches.
    /// Missing glyphs stay as NULL_REGION (already the default).
    fn preload_ascii(&mut self, queue: &wgpu::Queue) {
        for code in ASCII_RANGE {
            if let Some(ch) = char::from_u32(code) {
                if let Some(region) = self.rasterize(queue, ch) {
                    self.ascii_regions[code as usize] = region;
                }
                // If rasterize returns None, the slot stays as NULL_REGION.
            }
        }
        log::info!("Pre-rasterized ASCII glyphs");
    }
}

// --- Private helpers (rasterization) ---

impl GlyphAtlas {
    /// Rasterize a single character and pack it into the atlas.
    /// Returns None for whitespace or unsupported glyph types (color emoji).
    fn rasterize(&mut self, queue: &wgpu::Queue, ch: char) -> Option<GlyphRegion> {
        let cache_key = self.cache_key_for_char(ch)?;

        let image = self.swash_cache.get_image(&mut self.font_system, cache_key).as_ref()?;

        if image.placement.width == 0 || image.placement.height == 0 {
            return None;
        }

        let glyph_data = match image.content {
            SwashContent::Mask => &image.data,
            SwashContent::Color => return None, // TODO: color emoji atlas
            SwashContent::SubpixelMask => return None,
        };

        let gw = image.placement.width as i32;
        let gh = image.placement.height as i32;

        let alloc = self.allocator.allocate(size2(gw + 2, gh + 2))?;
        let atlas_x = alloc.rectangle.min.x as u32 + 1;
        let atlas_y = alloc.rectangle.min.y as u32 + 1;

        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: atlas_x,
                    y: atlas_y,
                    z: 0,
                },
                aspect: wgpu::TextureAspect::All,
            },
            glyph_data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(gw as u32),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: gw as u32,
                height: gh as u32,
                depth_or_array_layers: 1,
            },
        );

        Some(GlyphRegion {
            uv: [
                atlas_x as f32 / ATLAS_SIZE as f32,
                atlas_y as f32 / ATLAS_SIZE as f32,
                gw as f32 / ATLAS_SIZE as f32,
                gh as f32 / ATLAS_SIZE as f32,
            ],
            placement_left: image.placement.left as f32,
            placement_top: image.placement.top as f32,
            pixel_width: gw as f32,
            pixel_height: gh as f32,
        })
    }

    /// Shape a single character to get its cosmic-text cache key.
    /// Called at most once per unique character (during first rasterization).
    fn cache_key_for_char(&mut self, ch: char) -> Option<CacheKey> {
        let metrics = Metrics::new(self.font_size, self.font_size);
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        let mut attrs = Attrs::new().family(Family::Name(&self.font_family));
        if self.bold {
            attrs = attrs.weight(cosmic_text::fontdb::Weight::BOLD);
        }
        buffer.set_text(
            &mut self.font_system,
            &ch.to_string(),
            &attrs,
            cosmic_text::Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(&mut self.font_system, false);

        buffer
            .layout_runs()
            .next()?
            .glyphs
            .first()
            .map(|g| g.physical((0.0, 0.0), 1.0).cache_key)
    }
}
