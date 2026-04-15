//! Per-glyph rasterization and ASCII preload.
//!
//! Free functions that take `&mut GlyphAtlas` — same effect as methods,
//! but keeping them out of the main `impl GlyphAtlas` block reduces the
//! surface area of `mod.rs` and makes it clearer that these are the
//! "slow path" functions hit only on cache miss or initial load.

use cosmic_text::{Attrs, Buffer, CacheKey, Family, Metrics, Style, SwashContent, Weight};
use etagere::size2;

use super::region::GlyphRegion;
use super::style::GlyphStyle;
use super::texture::ATLAS_SIZE;
use super::{GlyphAtlas, ASCII_RANGE};

/// Rasterize a single character in the given style and pack it into the atlas.
/// Returns None for whitespace or unsupported glyph types (color emoji).
pub(crate) fn rasterize(
    atlas: &mut GlyphAtlas,
    queue: &wgpu::Queue,
    ch: char,
    style: GlyphStyle,
) -> Option<GlyphRegion> {
    let cache_key = cache_key_for_char(atlas, ch, style)?;

    let image = atlas
        .swash_cache
        .get_image(&mut atlas.font_system, cache_key)
        .as_ref()?;

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

    let alloc = atlas.allocator.allocate(size2(gw + 2, gh + 2))?;
    let atlas_x = alloc.rectangle.min.x as u32 + 1;
    let atlas_y = alloc.rectangle.min.y as u32 + 1;

    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &atlas.texture,
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

/// Shape a single character in the given style to get its cosmic-text
/// cache key. Called at most once per (char, style) pair.
fn cache_key_for_char(atlas: &mut GlyphAtlas, ch: char, style: GlyphStyle) -> Option<CacheKey> {
    let metrics = Metrics::new(atlas.font_size, atlas.font_size);
    let mut buffer = Buffer::new(&mut atlas.font_system, metrics);
    let mut attrs = Attrs::new().family(Family::Name(&atlas.font_family));
    if style.is_bold() {
        attrs = attrs.weight(Weight::BOLD);
    }
    if style.is_italic() {
        attrs = attrs.style(Style::Italic);
    }
    buffer.set_text(
        &mut atlas.font_system,
        &ch.to_string(),
        &attrs,
        cosmic_text::Shaping::Advanced,
        None,
    );
    buffer.shape_until_scroll(&mut atlas.font_system, false);

    buffer
        .layout_runs()
        .next()?
        .glyphs
        .first()
        .map(|g| g.physical((0.0, 0.0), 1.0).cache_key)
}

/// Pre-rasterize printable ASCII (32-126) in all four styles to avoid
/// first-frame hitches. Missing glyphs stay as `NULL_REGION`.
pub(crate) fn preload_ascii(atlas: &mut GlyphAtlas, queue: &wgpu::Queue) {
    for style in GlyphStyle::ALL {
        let style_idx = style as usize;
        for code in ASCII_RANGE {
            if let Some(ch) = char::from_u32(code)
                && let Some(region) = rasterize(atlas, queue, ch, style)
            {
                atlas.ascii_regions[style_idx][code as usize] = region;
            }
        }
    }
    log::info!("Pre-rasterized ASCII glyphs (regular/bold/italic/bold-italic)");
}
