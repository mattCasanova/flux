//! Instanced cell rendering.
//!
//! Renders text as instanced quads:
//! - Static quad vertex buffer (created once, triangle strip, 4 vertices)
//! - Per-glyph instance buffer (position, atlas UV, color)
//! - Single draw call for all visible glyphs

use bytemuck::{Pod, Zeroable};

/// A vertex of the unit quad — created once at init, never changes.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub(crate) struct QuadVertex {
    pub position: [f32; 2],
    pub uv: [f32; 2],
}

/// Triangle strip quad: 4 vertices.
///
///   2---3       Winding order for TriangleStrip:
///   |  /|       Triangle 1: 0 → 1 → 2
///   | / |       Triangle 2: 1 → 3 → 2 (implicit from strip)
///   |/  |
///   0---1
pub(crate) const QUAD_VERTICES: &[QuadVertex] = &[
    QuadVertex { position: [0.0, 1.0], uv: [0.0, 1.0] }, // bottom-left
    QuadVertex { position: [1.0, 1.0], uv: [1.0, 1.0] }, // bottom-right
    QuadVertex { position: [0.0, 0.0], uv: [0.0, 0.0] }, // top-left
    QuadVertex { position: [1.0, 0.0], uv: [1.0, 0.0] }, // top-right
];

/// Per-glyph instance data — one per visible glyph on screen.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub(crate) struct CellInstance {
    /// Screen position of this glyph (pixels, top-left corner)
    pub position: [f32; 2],
    /// Size of this glyph in pixels [width, height]
    pub size: [f32; 2],
    /// Atlas UV region: [u, v, width, height] (normalized 0-1)
    pub glyph_uv: [f32; 4],
    /// Foreground color RGBA (0.0-1.0)
    pub fg_color: [f32; 4],
    /// Background color RGBA (0.0-1.0)
    pub bg_color: [f32; 4],
}

impl QuadVertex {
    pub fn buffer_layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<QuadVertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: std::mem::offset_of!(QuadVertex, position) as u64,
                    shader_location: 0,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: std::mem::offset_of!(QuadVertex, uv) as u64,
                    shader_location: 1,
                },
            ],
        }
    }
}

impl CellInstance {
    pub fn buffer_layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<CellInstance>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &[
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: std::mem::offset_of!(CellInstance, position) as u64,
                    shader_location: 2,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x2,
                    offset: std::mem::offset_of!(CellInstance, size) as u64,
                    shader_location: 3,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: std::mem::offset_of!(CellInstance, glyph_uv) as u64,
                    shader_location: 4,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: std::mem::offset_of!(CellInstance, fg_color) as u64,
                    shader_location: 5,
                },
                wgpu::VertexAttribute {
                    format: wgpu::VertexFormat::Float32x4,
                    offset: std::mem::offset_of!(CellInstance, bg_color) as u64,
                    shader_location: 6,
                },
            ],
        }
    }
}
