//! Render pipeline creation — shaders, bind groups, vertex layouts.

use crate::cell_renderer::{CellInstance, QuadVertex};
use bytemuck::{Pod, Zeroable};

/// Projection matrix uniform.
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub(crate) struct Uniforms {
    pub projection: [[f32; 4]; 4],
}

impl Uniforms {
    /// Create an orthographic projection matrix for 2D rendering.
    /// Maps pixel coordinates (0,0 = top-left) to clip space (-1..1).
    pub fn ortho(width: f32, height: f32) -> Self {
        Self {
            projection: [
                [2.0 / width, 0.0, 0.0, 0.0],
                [0.0, -2.0 / height, 0.0, 0.0],
                [0.0, 0.0, 1.0, 0.0],
                [-1.0, 1.0, 0.0, 1.0],
            ],
        }
    }
}

/// Creates the render pipeline for cell/glyph rendering.
pub(crate) fn create_cell_pipeline(
    device: &wgpu::Device,
    surface_format: wgpu::TextureFormat,
    bind_group_layout: &wgpu::BindGroupLayout,
) -> wgpu::RenderPipeline {
    let shader_source = include_str!("../../../resources/shaders/cell.wgsl");
    let shader_module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Cell Shader"),
        source: wgpu::ShaderSource::Wgsl(shader_source.into()),
    });

    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Cell Pipeline"),
        layout: Some(
            &device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("Cell Pipeline Layout"),
                bind_group_layouts: &[Some(bind_group_layout)],
                immediate_size: 0,
            }),
        ),
        vertex: wgpu::VertexState {
            module: &shader_module,
            entry_point: Some("vs_main"),
            buffers: &[QuadVertex::buffer_layout(), CellInstance::buffer_layout()],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader_module,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format: surface_format,
                blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleStrip,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            unclipped_depth: false,
            polygon_mode: wgpu::PolygonMode::Fill,
            conservative: false,
        },
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    })
}

/// Create the bind group layout for cell rendering.
pub(crate) fn create_bind_group_layout(device: &wgpu::Device) -> wgpu::BindGroupLayout {
    device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Cell Bind Group Layout"),
        entries: &[
            // Uniforms (projection matrix)
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            // Glyph atlas texture
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
            // Atlas sampler
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    })
}

/// Create the bind group for cell rendering.
pub(crate) fn create_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    uniform_buffer: &wgpu::Buffer,
    atlas_view: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Cell Bind Group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: uniform_buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(atlas_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    })
}
