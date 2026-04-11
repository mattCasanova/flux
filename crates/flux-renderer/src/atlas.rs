//! Glyph atlas — rasterizes glyphs with cosmic-text and packs them
//! into GPU textures with etagere.
//!
//! Two atlases:
//! - Mask atlas (R8Unorm) for regular text
//! - Color atlas (Rgba8UnormSrgb) for emoji
//!
//! Glyph lookup is cached in a HashMap. Cache misses trigger
//! rasterization and atlas packing on demand.

// TODO: Phase 1, Step 2
// - GlyphAtlas struct with mask + color textures
// - GlyphKey (font_id, glyph_id, size) -> AtlasRegion mapping
// - Rasterize with cosmic-text SwashCache
// - Pack with etagere BucketedAtlasAllocator
// - Upload glyph pixels to GPU texture
