// cell.wgsl — Flux terminal cell/glyph renderer
//
// Renders glyphs as instanced quads.
// Each glyph is one instance of a shared unit quad.
// Single draw call for all visible glyphs.

// ── Vertex inputs ──────────────────────────────────────────────

// Per-vertex (from quad vertex buffer, step_mode: Vertex)
struct QuadVertex {
    @location(0) quad_pos: vec2<f32>,   // unit quad position (0-1)
    @location(1) quad_uv: vec2<f32>,    // unit quad UV (0-1)
};

// Per-instance (from instance buffer, step_mode: Instance)
struct CellInstance {
    @location(2) cell_pos: vec2<f32>,      // screen position (pixels)
    @location(3) cell_size: vec2<f32>,     // glyph size (pixels)
    @location(4) glyph_uv: vec4<f32>,      // atlas region: [u, v, w, h]
    @location(5) fg_color: vec4<f32>,
    @location(6) bg_color: vec4<f32>,
};

// ── Uniforms ───────────────────────────────────────────────────

struct Uniforms {
    projection: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> uniforms: Uniforms;
@group(0) @binding(1) var glyph_atlas: texture_2d<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;

// ── Vertex stage ───────────────────────────────────────────────

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) fg_color: vec4<f32>,
    @location(2) bg_color: vec4<f32>,
};

@vertex
fn vs_main(vertex: QuadVertex, instance: CellInstance) -> VertexOutput {
    // Scale unit quad to glyph size and offset to glyph position
    let world_pos = instance.cell_pos + vertex.quad_pos * instance.cell_size;

    var out: VertexOutput;
    out.clip_position = uniforms.projection * vec4(world_pos, 0.0, 1.0);

    // Map unit UV (0-1) to this glyph's region in the atlas
    out.uv = instance.glyph_uv.xy + vertex.quad_uv * instance.glyph_uv.zw;

    out.fg_color = instance.fg_color;
    out.bg_color = instance.bg_color;

    return out;
}

// ── Fragment stage ─────────────────────────────────────────────

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Sample glyph alpha from atlas (single-channel grayscale atlas)
    let glyph_alpha = textureSample(glyph_atlas, atlas_sampler, in.uv).r;

    // Where glyph exists -> foreground color. Where empty -> background color.
    return mix(in.bg_color, in.fg_color, glyph_alpha);
}
