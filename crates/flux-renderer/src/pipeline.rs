//! Render pipeline creation — shaders, bind groups, vertex layouts.
//!
//! Sets up the wgpu render pipeline for instanced cell rendering:
//! - Loads cell.wgsl shader
//! - Configures vertex buffer layouts (quad geometry + per-cell instance data)
//! - Creates bind groups for uniforms + glyph atlas texture
//! - Uses TriangleStrip topology (4 vertices per quad)

// TODO: Phase 1, Step 2-4
// - Load cell.wgsl shader module
// - Create vertex buffer layout for QuadVertex (step per-vertex)
// - Create vertex buffer layout for CellInstance (step per-instance)
// - Create pipeline with TriangleStrip topology
// - Create uniform bind group (projection + cell_size)
// - Create atlas bind group (texture + sampler)
