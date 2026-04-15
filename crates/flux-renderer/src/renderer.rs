//! Renderer struct + lifecycle (new, resize, config setters, font rebuild).
//!
//! This module owns the struct definition and the methods that aren't
//! tied to a specific rendering pass: construction, configuration,
//! resize, font rebuild, metrics. The actual drawing lives in
//! `output.rs` / `input_chrome.rs` / `render_pass.rs` and hangs off
//! additional `impl Renderer` blocks there.
//!
//! All struct fields are `pub(crate)` so sibling modules can read and
//! mutate them directly — the alternative would be dozens of getter
//! methods and a flood of `mem::take` tricks. Since this is a single
//! crate, `pub(crate)` is the right visibility.

use std::sync::Arc;

use anyhow::Result;

use crate::atlas::{self, GlyphStyle};
use crate::buffer::INITIAL_MAX_CELLS;
use crate::cell_renderer::CellInstance;
use crate::gpu;
use crate::gpu_resources::{
    create_instance_buffer, create_quad_buffer, create_sampler, create_uniform_buffer,
};
use crate::pipeline::{self, Uniforms};
use flux_types::Color;

/// Cell dimensions in pixels.
pub struct CellMetrics {
    pub width: f32,
    pub height: f32,
    pub baseline_offset: f32,
}

/// The renderer — owns all GPU state and renders frames.
pub struct Renderer {
    pub(crate) gpu: gpu::GpuContext,
    pub(crate) atlas: atlas::GlyphAtlas,
    pub(crate) pipeline: wgpu::RenderPipeline,
    pub(crate) bind_group_layout: wgpu::BindGroupLayout,
    pub(crate) bind_group: wgpu::BindGroup,
    pub(crate) quad_vertex_buffer: wgpu::Buffer,
    pub(crate) uniform_buffer: wgpu::Buffer,
    pub(crate) sampler: wgpu::Sampler,
    pub(crate) clear_color: Color,
    /// The effective clear color used by the next render pass. Normally
    /// equals `clear_color`, but in raw mode we sync it to the alt-screen
    /// program's Normal bg so the sub-cell leftover space on the right and
    /// bottom edges of the grid (from integer cell math) doesn't leak the
    /// Flux theme color through vim's colorscheme.
    pub(crate) effective_clear_color: Color,
    /// Pre-allocated instance buffer — written to, never recreated during normal use.
    pub(crate) instance_buffer: wgpu::Buffer,
    pub(crate) instance_capacity: usize,
    pub(crate) instance_count: u32,
    /// Instances for the output grid — rebuilt on `set_grid`.
    pub(crate) output_instances: Vec<CellInstance>,
    /// Instances for the fixed input chrome — rebuilt on `set_input_line`.
    pub(crate) input_instances: Vec<CellInstance>,
    /// Instances for popup overlays (F7 autocomplete, F14 search
    /// overlay, future command palette, etc.). Paint order is
    /// output → input → popup, so popups render on top of everything
    /// else. Empty in R4 — no feature writes to it yet.
    pub(crate) popup_instances: Vec<CellInstance>,
    /// Default glyph style applied to cells with no bold/italic flags.
    /// Driven by `[font] weight = "bold"` / `style = "italic"` in the config
    /// file, so users can set a baseline weight the whole terminal inherits.
    pub(crate) default_style: GlyphStyle,
    /// Padding around the terminal grid (pixels).
    pub(crate) padding_x: f32,
    pub(crate) padding_y: f32,
    /// When true, `set_grid` shifts rows down so the shell's cursor row
    /// ends up at the bottom of the output area. Disabled in raw mode so
    /// alt-screen programs (vim, less) fill the full grid top-to-bottom.
    pub(crate) bottom_anchor: bool,
    /// When true, `set_grid` draws the shell's cursor block. Off by default
    /// because Flux's input editor owns cursor display in cooked mode; on in
    /// raw mode so alt-screen programs can show their own cursor.
    pub(crate) show_shell_cursor: bool,
}

impl Renderer {
    /// Create a new renderer attached to a winit window.
    /// Called once at startup. All GPU resources are allocated here.
    pub fn new(
        window: Arc<winit::window::Window>,
        font_family: &str,
        font_size: f32,
        line_height: f32,
        default_style: GlyphStyle,
    ) -> Result<Self> {
        let gpu = gpu::GpuContext::new(window)?;
        let atlas = atlas::GlyphAtlas::new(
            &gpu.device,
            &gpu.queue,
            font_family,
            font_size,
            line_height,
        )?;
        let quad_vertex_buffer = create_quad_buffer(&gpu.device);
        let uniform_buffer = create_uniform_buffer(&gpu.device, &gpu.surface_config);
        let sampler = create_sampler(&gpu.device);
        let bind_group_layout = pipeline::create_bind_group_layout(&gpu.device);
        let pipeline =
            pipeline::create_cell_pipeline(&gpu.device, gpu.format(), &bind_group_layout);
        let bind_group = pipeline::create_bind_group(
            &gpu.device,
            &bind_group_layout,
            &uniform_buffer,
            &atlas.texture_view,
            &sampler,
        );

        // Pre-allocate instance buffer — sized for INITIAL_MAX_CELLS, grows if needed.
        let instance_buffer = create_instance_buffer(&gpu.device, INITIAL_MAX_CELLS);

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
            effective_clear_color: Color::from_hex("#24283b").unwrap(),
            instance_buffer,
            instance_capacity: INITIAL_MAX_CELLS,
            instance_count: 0,
            output_instances: Vec::with_capacity(INITIAL_MAX_CELLS),
            input_instances: Vec::with_capacity(64),
            popup_instances: Vec::new(),
            padding_x: 0.0,
            padding_y: 0.0,
            bottom_anchor: true,
            show_shell_cursor: false,
            default_style,
        })
    }

    /// Set the horizontal and vertical padding between the window edge and the grid.
    pub fn set_padding(&mut self, horizontal: f32, vertical: f32) {
        self.padding_x = horizontal;
        self.padding_y = vertical;
    }

    /// Toggle bottom-anchor rendering of the output grid. Disable for
    /// raw-mode (alt-screen) programs so they fill the grid top-down.
    pub fn set_bottom_anchor(&mut self, enabled: bool) {
        self.bottom_anchor = enabled;
    }

    /// Toggle rendering of the shell's cursor block. Enable in raw mode so
    /// alt-screen programs can show their cursor; leave off in cooked mode
    /// where Flux's own input editor owns the cursor.
    pub fn set_show_shell_cursor(&mut self, enabled: bool) {
        self.show_shell_cursor = enabled;
    }

    pub fn cell_metrics(&self) -> CellMetrics {
        CellMetrics {
            baseline_offset: self.atlas.baseline_offset,
            width: self.atlas.cell_width,
            height: self.atlas.cell_height,
        }
    }

    /// Rebuild the glyph atlas with a new font size (e.g., after scale factor change).
    /// Called only when moving between monitors with different DPI.
    pub fn rebuild_font(
        &mut self,
        font_family: &str,
        font_size: f32,
        line_height: f32,
    ) -> Result<()> {
        self.atlas = atlas::GlyphAtlas::new(
            &self.gpu.device,
            &self.gpu.queue,
            font_family,
            font_size,
            line_height,
        )?;
        self.bind_group = pipeline::create_bind_group(
            &self.gpu.device,
            &self.bind_group_layout,
            &self.uniform_buffer,
            &self.atlas.texture_view,
            &self.sampler,
        );
        self.instance_count = 0;
        Ok(())
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.gpu.resize(width, height);
        let uniforms = Uniforms::ortho(width as f32, height as f32);
        self.gpu.queue.write_buffer(
            &self.uniform_buffer,
            0,
            bytemuck::cast_slice(&[uniforms]),
        );
    }

    pub fn set_clear_color(&mut self, color: Color) {
        self.clear_color = color;
        self.effective_clear_color = color;
    }
}
