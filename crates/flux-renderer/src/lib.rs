//! GPU rendering for the Flux terminal emulator.
//!
//! ALL wgpu code lives in this crate. Nothing outside flux-renderer
//! imports wgpu. Other crates interact through [flux_types] data
//! structures only.

mod atlas;
mod cell_renderer;
mod gpu;
mod pipeline;
mod ui_renderer;

use std::sync::Arc;

use anyhow::Result;
use wgpu::util::DeviceExt;

use cell_renderer::{CellInstance, QUAD_VERTICES};
use flux_types::Color;
use pipeline::Uniforms;

/// Cell dimensions in pixels.
pub struct CellMetrics {
    pub width: f32,
    pub height: f32,
}

/// Maximum cells we pre-allocate for. Grows if needed.
const INITIAL_MAX_CELLS: usize = 200 * 60; // 200 cols x 60 rows

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
    /// Pre-allocated instance buffer — written to, never recreated during normal use.
    instance_buffer: wgpu::Buffer,
    instance_capacity: usize,
    instance_count: u32,
}

impl Renderer {
    /// Create a new renderer attached to a winit window.
    /// Called once at startup. All GPU resources are allocated here.
    pub fn new(
        window: Arc<winit::window::Window>,
        font_family: &str,
        font_size: f32,
        line_height: f32,
        bold: bool,
    ) -> Result<Self> {
        let gpu = gpu::GpuContext::new(window)?;
        let atlas = atlas::GlyphAtlas::new(&gpu.device, &gpu.queue, font_family, font_size, line_height, bold)?;
        let quad_vertex_buffer = Self::create_quad_buffer(&gpu.device);
        let uniform_buffer = Self::create_uniform_buffer(&gpu.device, &gpu.surface_config);
        let sampler = Self::create_sampler(&gpu.device);
        let bind_group_layout = pipeline::create_bind_group_layout(&gpu.device);
        let pipeline = pipeline::create_cell_pipeline(&gpu.device, gpu.format(), &bind_group_layout);
        let bind_group = pipeline::create_bind_group(&gpu.device, &bind_group_layout, &uniform_buffer, &atlas.texture_view, &sampler);

        // Pre-allocate instance buffer — sized for INITIAL_MAX_CELLS, grows if needed.
        let instance_buffer = Self::create_instance_buffer(&gpu.device, INITIAL_MAX_CELLS);

        Ok(Self {
            gpu,
            atlas,
            pipeline,
            bind_group_layout,
            bind_group,
            quad_vertex_buffer,
            uniform_buffer,
            sampler,
            clear_color: Color::from_hex("#24283b").unwrap(),
            instance_buffer,
            instance_capacity: INITIAL_MAX_CELLS,
            instance_count: 0,
        })
    }

    pub fn cell_metrics(&self) -> CellMetrics {
        CellMetrics {
            width: self.atlas.cell_width,
            height: self.atlas.cell_height,
        }
    }

    /// Rebuild the glyph atlas with a new font size (e.g., after scale factor change).
    /// Called only when moving between monitors with different DPI.
    pub fn rebuild_font(&mut self, font_family: &str, font_size: f32, line_height: f32, bold: bool) -> Result<()> {
        self.atlas = atlas::GlyphAtlas::new(&self.gpu.device, &self.gpu.queue, font_family, font_size, line_height, bold)?;
        self.bind_group = pipeline::create_bind_group(&self.gpu.device, &self.bind_group_layout, &self.uniform_buffer, &self.atlas.texture_view, &self.sampler);
        self.instance_count = 0;
        Ok(())
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.gpu.resize(width, height);
        let uniforms = Uniforms::ortho(width as f32, height as f32);
        self.gpu.queue.write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));
    }

    pub fn set_clear_color(&mut self, color: Color) {
        self.clear_color = color;
    }

    /// Render some static text at a given position. Temporary — for testing only.
    pub fn set_text(&mut self, text: &str, x: f32, y: f32, fg: Color, bg: Color) {
        let shaped = self.atlas.shape_text(text);
        let instances = self.build_instances(&shaped, x, y, fg, bg);
        self.write_instances(&instances);
    }

    /// Render a terminal grid — each cell at its grid position.
    pub fn set_grid(&mut self, grid: &flux_types::RenderGrid) {
        let cell_w = self.atlas.cell_width;
        let cell_h = self.atlas.cell_height;

        let mut instances: Vec<CellInstance> = Vec::with_capacity(grid.cols * grid.rows);

        for row in 0..grid.rows {
            for col in 0..grid.cols {
                let cell = grid.get(row, col);
                let x = col as f32 * cell_w;
                let y = row as f32 * cell_h;

                let is_cursor = grid.cursor == Some((col, row));

                if is_cursor {
                    // Render cursor as a filled block (inverted colors).
                    // Use a space glyph's UV (or zero UV) with solid fg as bg.
                    let cursor_color = Color::from_hex("#c0caf5").unwrap_or(Color::default());

                    // First: draw cursor background block
                    instances.push(CellInstance {
                        position: [x, y],
                        size: [cell_w, cell_h],
                        glyph_uv: [0.0, 0.0, 0.0, 0.0], // no glyph — solid color
                        fg_color: [cursor_color.r, cursor_color.g, cursor_color.b, cursor_color.a],
                        bg_color: [cursor_color.r, cursor_color.g, cursor_color.b, cursor_color.a],
                    });

                    // Then: draw the character under the cursor with inverted colors
                    if cell.character != ' ' && cell.character != '\0' {
                        let bg_color = Color::from_hex("#24283b").unwrap_or(Color::new(0.0, 0.0, 0.0, 1.0));
                        self.render_glyph(cell.character, x, y, cell_h, bg_color, cursor_color, &mut instances);
                    }
                } else if cell.character != ' ' && cell.character != '\0' {
                    self.render_glyph(cell.character, x, y, cell_h, cell.fg, cell.bg, &mut instances);
                }
            }
        }

        self.write_instances(&instances);
    }

    /// Render a single glyph character at a grid position.
    fn render_glyph(
        &mut self,
        character: char,
        x: f32,
        y: f32,
        cell_h: f32,
        fg: Color,
        bg: Color,
        instances: &mut Vec<CellInstance>,
    ) {
        let shaped = self.atlas.shape_text(&character.to_string());
        for glyph in &shaped {
            if let Some(region) = self.atlas.lookup(&self.gpu.queue, glyph.cache_key) {
                instances.push(CellInstance {
                    position: [
                        x + region.placement_left,
                        y + cell_h - region.placement_top,
                    ],
                    size: [region.pixel_width, region.pixel_height],
                    glyph_uv: region.uv,
                    fg_color: [fg.r, fg.g, fg.b, fg.a],
                    bg_color: [bg.r, bg.g, bg.b, bg.a],
                });
            }
        }
    }

    /// Write instance data to the pre-allocated GPU buffer.
    /// Grows the buffer if needed (rare — only on first oversized write).
    fn write_instances(&mut self, instances: &[CellInstance]) {
        if instances.is_empty() {
            self.instance_count = 0;
            return;
        }

        // Grow buffer if needed
        if instances.len() > self.instance_capacity {
            self.instance_capacity = instances.len() * 2;
            self.instance_buffer = Self::create_instance_buffer(&self.gpu.device, self.instance_capacity);
            log::info!("Grew instance buffer to {} cells", self.instance_capacity);
        }

        // Write to existing buffer — no allocation
        self.gpu.queue.write_buffer(
            &self.instance_buffer,
            0,
            bytemuck::cast_slice(instances),
        );
        self.instance_count = instances.len() as u32;
    }

    /// Render a frame.
    pub fn render(&mut self) -> Result<()> {
        let output = self.acquire_surface_texture()?;
        let Some(output) = output else { return Ok(()) };

        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self.gpu.device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Flux Render Encoder"),
        });

        self.render_pass(&mut encoder, &view);

        self.gpu.queue.submit(std::iter::once(encoder.finish()));
        output.present();
        Ok(())
    }
}

// --- Private helpers (GPU resource creation — called once) ---

impl Renderer {
    fn create_quad_buffer(device: &wgpu::Device) -> wgpu::Buffer {
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Quad Vertex Buffer"),
            contents: bytemuck::cast_slice(QUAD_VERTICES),
            usage: wgpu::BufferUsages::VERTEX,
        })
    }

    fn create_uniform_buffer(device: &wgpu::Device, config: &wgpu::SurfaceConfiguration) -> wgpu::Buffer {
        let uniforms = Uniforms::ortho(config.width as f32, config.height as f32);
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Uniform Buffer"),
            contents: bytemuck::cast_slice(&[uniforms]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        })
    }

    fn create_instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
        let size = (capacity * std::mem::size_of::<CellInstance>()) as u64;
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Cell Instance Buffer"),
            size,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    }

    fn create_sampler(device: &wgpu::Device) -> wgpu::Sampler {
        device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Atlas Sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        })
    }
}

// --- Private helpers (per-frame rendering) ---

impl Renderer {
    /// Acquire the next surface texture. Returns None if the frame should be skipped.
    fn acquire_surface_texture(&mut self) -> Result<Option<wgpu::SurfaceTexture>> {
        match self.gpu.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture)
            | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => Ok(Some(texture)),
            wgpu::CurrentSurfaceTexture::Timeout
            | wgpu::CurrentSurfaceTexture::Occluded => Ok(None),
            other => Err(anyhow::anyhow!("Surface texture error: {:?}", other)),
        }
    }

    /// Execute the render pass — clear screen and draw glyphs.
    fn render_pass(&self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Main Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(self.wgpu_clear_color()),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });

        self.draw_glyphs(&mut pass);
    }

    /// Draw all glyph instances in a single draw call.
    fn draw_glyphs<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        if self.instance_count == 0 { return }

        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.quad_vertex_buffer.slice(..));
        pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
        pass.draw(0..4, 0..self.instance_count);
    }

    /// Convert our Color to wgpu::Color.
    fn wgpu_clear_color(&self) -> wgpu::Color {
        wgpu::Color {
            r: self.clear_color.r as f64,
            g: self.clear_color.g as f64,
            b: self.clear_color.b as f64,
            a: self.clear_color.a as f64,
        }
    }

    /// Build glyph instances from shaped text.
    fn build_instances(&mut self, shaped: &[atlas::ShapedGlyph], x: f32, y: f32, fg: Color, bg: Color) -> Vec<CellInstance> {
        let mut instances = Vec::with_capacity(shaped.len());

        for glyph in shaped {
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

        instances
    }
}
