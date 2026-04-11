//! GPU rendering for the Flux terminal emulator.
//!
//! ALL wgpu code lives in this crate. Nothing outside flux-renderer
//! imports wgpu. Other crates interact through [flux_types] data
//! structures only.

mod gpu;
mod atlas;
mod pipeline;
mod cell_renderer;
mod ui_renderer;

use std::sync::Arc;
use anyhow::Result;
use flux_types::Color;
use wgpu::util::DeviceExt;

use cell_renderer::{CellInstance, QUAD_VERTICES};
use pipeline::Uniforms;

/// Cell dimensions in pixels.
pub struct CellMetrics {
    pub width: f32,
    pub height: f32,
}

/// The renderer — owns all GPU state and renders frames.
pub struct Renderer {
    gpu: gpu::GpuContext,
    atlas: atlas::GlyphAtlas,
    pipeline: wgpu::RenderPipeline,
    bind_group_layout: wgpu::BindGroupLayout,
    bind_group: wgpu::BindGroup,
    quad_vertex_buffer: wgpu::Buffer,
    uniform_buffer: wgpu::Buffer,
    sampler: wgpu::Sampler,
    clear_color: Color,
    // Current instance data for rendering
    instance_buffer: Option<wgpu::Buffer>,
    instance_count: u32,
}

impl Renderer {
    /// Create a new renderer attached to a winit window.
    pub fn new(window: Arc<winit::window::Window>, font_family: &str, font_size: f32) -> Result<Self> {
        let gpu = gpu::GpuContext::new(window)?;

        // Create glyph atlas
        let atlas = atlas::GlyphAtlas::new(
            &gpu.device,
            &gpu.queue,
            font_family,
            font_size,
        )?;

        // Create static quad vertex buffer (never changes)
        let quad_vertex_buffer = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Quad Vertex Buffer"),
            contents: bytemuck::cast_slice(QUAD_VERTICES),
            usage: wgpu::BufferUsages::VERTEX,
        });

        // Create uniform buffer with initial projection
        let size = gpu.surface_config.width as f32;
        let height = gpu.surface_config.height as f32;
        let uniforms = Uniforms::ortho(size, height);
        let uniform_buffer = gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Uniform Buffer"),
            contents: bytemuck::cast_slice(&[uniforms]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // Create sampler for atlas texture
        let sampler = gpu.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Atlas Sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        // Create pipeline
        let bind_group_layout = pipeline::create_bind_group_layout(&gpu.device);
        let render_pipeline = pipeline::create_cell_pipeline(
            &gpu.device,
            gpu.format(),
            &bind_group_layout,
        );

        // Create bind group
        let bind_group = pipeline::create_bind_group(
            &gpu.device,
            &bind_group_layout,
            &uniform_buffer,
            &atlas.texture_view,
            &sampler,
        );

        Ok(Self {
            gpu,
            atlas,
            pipeline: render_pipeline,
            bind_group_layout,
            bind_group,
            quad_vertex_buffer,
            uniform_buffer,
            sampler,
            clear_color: Color::from_hex("#24283b").unwrap(),
            instance_buffer: None,
            instance_count: 0,
        })
    }

    /// Get cell dimensions.
    pub fn cell_metrics(&self) -> CellMetrics {
        CellMetrics {
            width: self.atlas.cell_width,
            height: self.atlas.cell_height,
        }
    }

    /// Handle window resize.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.gpu.resize(width, height);
        // Update projection matrix
        let uniforms = Uniforms::ortho(width as f32, height as f32);
        self.gpu.queue.write_buffer(
            &self.uniform_buffer,
            0,
            bytemuck::cast_slice(&[uniforms]),
        );
    }

    /// Set the clear color (background).
    pub fn set_clear_color(&mut self, color: Color) {
        self.clear_color = color;
    }

    /// Render some static text at a given position. For testing.
    pub fn set_text(&mut self, text: &str, x: f32, y: f32, fg: Color, bg: Color) {
        let font_family = "Menlo"; // TODO: from config
        let shaped = self.atlas.shape_text(text, font_family);

        let mut instances: Vec<CellInstance> = Vec::new();

        for glyph in &shaped {
            if let Some(region) = self.atlas.lookup(&self.gpu.queue, glyph.cache_key) {
                instances.push(CellInstance {
                    position: [
                        x + glyph.x + region.placement_left,
                        y + glyph.y - region.placement_top,
                    ],
                    size: [region.pixel_width, region.pixel_height],
                    glyph_uv: region.uv,
                    fg_color: [fg.r, fg.g, fg.b, fg.a],
                    bg_color: [bg.r, bg.g, bg.b, bg.a],
                });
            }
        }

        if instances.is_empty() {
            self.instance_buffer = None;
            self.instance_count = 0;
            return;
        }

        self.instance_buffer = Some(
            self.gpu.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Instance Buffer"),
                contents: bytemuck::cast_slice(&instances),
                usage: wgpu::BufferUsages::VERTEX,
            }),
        );
        self.instance_count = instances.len() as u32;
    }

    /// Render a frame.
    pub fn render(&mut self) -> Result<()> {
        let output = match self.gpu.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture)
            | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => texture,
            wgpu::CurrentSurfaceTexture::Timeout
            | wgpu::CurrentSurfaceTexture::Occluded => return Ok(()),
            other => return Err(anyhow::anyhow!("Surface texture error: {:?}", other)),
        };
        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self.gpu.device.create_command_encoder(
            &wgpu::CommandEncoderDescriptor {
                label: Some("Flux Render Encoder"),
            },
        );

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Main Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: self.clear_color.r as f64,
                            g: self.clear_color.g as f64,
                            b: self.clear_color.b as f64,
                            a: self.clear_color.a as f64,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });

            // Draw glyphs if we have any
            if let Some(ref instance_buffer) = self.instance_buffer {
                if self.instance_count > 0 {
                    render_pass.set_pipeline(&self.pipeline);
                    render_pass.set_bind_group(0, &self.bind_group, &[]);
                    render_pass.set_vertex_buffer(0, self.quad_vertex_buffer.slice(..));
                    render_pass.set_vertex_buffer(1, instance_buffer.slice(..));
                    render_pass.draw(0..4, 0..self.instance_count);
                }
            }
        }

        self.gpu.queue.submit(std::iter::once(encoder.finish()));
        output.present();

        Ok(())
    }
}
