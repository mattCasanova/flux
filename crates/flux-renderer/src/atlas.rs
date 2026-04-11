//! Glyph atlas — rasterizes glyphs with cosmic-text and packs them
//! into a GPU texture with etagere.
//!
//! Uses a grayscale (R8Unorm) atlas for regular text.
//! Color emoji atlas (Rgba8Unorm) will be added later.

use std::collections::HashMap;
use anyhow::Result;
use cosmic_text::{
    Attrs, Buffer, CacheKey, Family, FontSystem, Metrics,
    SwashCache, SwashContent,
};
use etagere::{size2, BucketedAtlasAllocator};

/// UV coordinates for a glyph in the atlas (normalized 0-1).
#[derive(Debug, Copy, Clone)]
pub(crate) struct GlyphRegion {
    pub uv: [f32; 4], // [u, v, width, height] in normalized coords
    pub placement_left: f32,
    pub placement_top: f32,
    pub pixel_width: f32,
    pub pixel_height: f32,
}

/// Key for looking up cached glyphs.
#[derive(Debug, Clone, Copy, Hash, Eq, PartialEq)]
struct GlyphKey {
    cache_key: CacheKey,
}

/// Manages the glyph texture atlas.
pub(crate) struct GlyphAtlas {
    // Font system and rasterizer
    pub font_system: FontSystem,
    swash_cache: SwashCache,

    // Atlas texture on GPU
    texture: wgpu::Texture,
    pub texture_view: wgpu::TextureView,
    allocator: BucketedAtlasAllocator,
    atlas_size: u32,

    // Glyph cache
    cache: HashMap<GlyphKey, GlyphRegion>,

    // Font settings
    pub cell_width: f32,
    pub cell_height: f32,
    pub font_size: f32,
    pub line_height: f32,
    font_family: String,
    bold: bool,

    // Fast char → CacheKey lookup (avoids full shaping for single characters)
    char_cache: HashMap<char, CacheKey>,
}

impl GlyphAtlas {
    /// Create a new glyph atlas.
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

        // Calculate cell dimensions from font metrics
        let line_height_px = font_size * line_height;
        let metrics = Metrics::new(font_size, line_height_px);
        let (cell_width, cell_height) = Self::measure_cell(&mut font_system, font_family, bold, &metrics);

        // Create atlas texture (1024x1024 R8Unorm — grayscale)
        let atlas_size = 1024u32;
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Glyph Atlas"),
            size: wgpu::Extent3d {
                width: atlas_size,
                height: atlas_size,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        // Clear atlas to zero
        let zeros = vec![0u8; (atlas_size * atlas_size) as usize];
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &zeros,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(atlas_size),
                rows_per_image: None,
            },
            wgpu::Extent3d {
                width: atlas_size,
                height: atlas_size,
                depth_or_array_layers: 1,
            },
        );

        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        let allocator = BucketedAtlasAllocator::new(size2(atlas_size as i32, atlas_size as i32));

        Ok(Self {
            font_system,
            swash_cache,
            texture,
            texture_view,
            allocator,
            atlas_size,
            cache: HashMap::new(),
            cell_width,
            cell_height,
            font_size,
            line_height: line_height_px,
            font_family: font_family.to_string(),
            bold,
            char_cache: HashMap::new(),
        })
    }

    /// Measure cell dimensions using the font's monospace advance width.
    fn measure_cell(
        font_system: &mut FontSystem,
        font_family: &str,
        bold: bool,
        metrics: &Metrics,
    ) -> (f32, f32) {
        // Create a buffer with a single character to measure
        let mut buffer = Buffer::new(font_system, *metrics);
        let mut attrs = Attrs::new().family(Family::Name(font_family));
        if bold {
            attrs = attrs.weight(cosmic_text::fontdb::Weight::BOLD);
        }
        buffer.set_text(
            font_system,
            "M",
            &attrs,
            cosmic_text::Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(font_system, false);

        // Get the advance width from the first glyph
        let mut cell_width = metrics.font_size * 0.6; // fallback
        for run in buffer.layout_runs() {
            for glyph in run.glyphs.iter() {
                cell_width = glyph.w;
                break;
            }
            break;
        }

        let cell_height = metrics.line_height;

        log::info!(
            "Cell metrics: {}x{} (font: {}, size: {})",
            cell_width,
            cell_height,
            font_family,
            metrics.font_size
        );

        (cell_width, cell_height)
    }

    /// Look up or rasterize a glyph, returning its atlas region.
    /// Returns None for space characters (nothing to render).
    pub fn lookup(
        &mut self,
        queue: &wgpu::Queue,
        cache_key: CacheKey,
    ) -> Option<GlyphRegion> {
        let key = GlyphKey { cache_key };

        // Check cache first
        if let Some(region) = self.cache.get(&key) {
            return Some(*region);
        }

        // Rasterize the glyph
        let image = match self.swash_cache.get_image(&mut self.font_system, cache_key) {
            Some(image) => image,
            None => return None,
        };

        if image.placement.width == 0 || image.placement.height == 0 {
            return None; // space or empty glyph
        }

        // Only handle mask (grayscale) glyphs for now
        let glyph_data = match image.content {
            SwashContent::Mask => &image.data,
            SwashContent::Color => return None, // TODO: color emoji atlas
            SwashContent::SubpixelMask => return None, // not used
        };

        let gw = image.placement.width as i32;
        let gh = image.placement.height as i32;

        // Allocate space in atlas (add 1px padding to avoid bleeding)
        let alloc = self
            .allocator
            .allocate(size2(gw + 2, gh + 2))?;

        // Upload glyph pixels to atlas texture (offset by 1px for padding)
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

        // Compute UV coordinates (normalized 0-1)
        let region = GlyphRegion {
            uv: [
                atlas_x as f32 / self.atlas_size as f32,
                atlas_y as f32 / self.atlas_size as f32,
                gw as f32 / self.atlas_size as f32,
                gh as f32 / self.atlas_size as f32,
            ],
            placement_left: image.placement.left as f32,
            placement_top: image.placement.top as f32,
            pixel_width: gw as f32,
            pixel_height: gh as f32,
        };

        self.cache.insert(key, region);
        Some(region)
    }

    /// Fast single-character lookup. Caches the char → CacheKey mapping
    /// so we only shape each character once, then it's a HashMap lookup.
    pub fn lookup_char(
        &mut self,
        queue: &wgpu::Queue,
        ch: char,
    ) -> Option<GlyphRegion> {
        // Check char cache first — avoids the full shaping pipeline
        if let Some(&cache_key) = self.char_cache.get(&ch) {
            return self.lookup(queue, cache_key);
        }

        // First time seeing this character — shape it once to get the cache key
        let shaped = self.shape_text(&ch.to_string());
        if let Some(glyph) = shaped.first() {
            self.char_cache.insert(ch, glyph.cache_key);
            self.lookup(queue, glyph.cache_key)
        } else {
            None
        }
    }

    /// Rasterize text and return glyph positions + cache keys for rendering.
    /// This uses cosmic-text's shaping to handle ligatures, kerning, etc.
    pub fn shape_text(
        &mut self,
        text: &str,
    ) -> Vec<ShapedGlyph> {
        let metrics = Metrics::new(self.font_size, self.line_height);
        let mut buffer = Buffer::new(&mut self.font_system, metrics);
        let mut attrs = Attrs::new().family(Family::Name(&self.font_family));
        if self.bold {
            attrs = attrs.weight(cosmic_text::fontdb::Weight::BOLD);
        }
        buffer.set_text(
            &mut self.font_system,
            text,
            &attrs,
            cosmic_text::Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(&mut self.font_system, false);

        let mut glyphs = Vec::new();

        for run in buffer.layout_runs() {
            for glyph in run.glyphs.iter() {
                // Get the physical glyph with cache key for rasterization
                let physical = glyph.physical((0.0, 0.0), 1.0);
                glyphs.push(ShapedGlyph {
                    cache_key: physical.cache_key,
                    x: glyph.x,
                    y: run.line_y,
                    w: glyph.w,
                });
            }
        }

        glyphs
    }
}

/// A positioned glyph from text shaping.
pub(crate) struct ShapedGlyph {
    pub cache_key: CacheKey,
    pub x: f32,
    pub y: f32,
    pub w: f32,
}
