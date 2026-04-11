//! Instanced cell rendering.
//!
//! Renders the terminal grid as instanced quads:
//! - Static quad vertex buffer (created once at init, never changes)
//! - Per-cell instance buffer (rebuilt only for dirty rows)
//! - Single draw call for the entire visible grid
//!
//! Same pattern as LiquidMetal2D's instanced sprite rendering.

// TODO: Phase 1, Step 4
// - QuadVertex struct (position + uv, 4 vertices for triangle strip)
// - CellInstance struct (position, glyph_uv, fg, bg, flags — 64 bytes)
// - Create static quad vertex buffer at init
// - CellGrid: sync_from_term() diffs terminal grid, marks dirty rows
// - CellGrid: upload_dirty() writes only changed rows to GPU buffer
// - Draw call: render_pass.draw(0..4, 0..cell_count)
