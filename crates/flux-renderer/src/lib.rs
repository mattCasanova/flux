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
use flux_types::{CellFlags, Color};
use pipeline::Uniforms;

pub use atlas::GlyphStyle;

/// Cell dimensions in pixels.
pub struct CellMetrics {
    pub width: f32,
    pub height: f32,
    pub baseline_offset: f32,
}

/// Approximate color equality — used to skip painting cell backgrounds that
/// match the window clear color, since those are already filled by the
/// render pass' clear step.
fn color_matches(a: Color, b: Color) -> bool {
    const EPS: f32 = 1.0 / 512.0;
    (a.r - b.r).abs() < EPS
        && (a.g - b.g).abs() < EPS
        && (a.b - b.b).abs() < EPS
        && (a.a - b.a).abs() < EPS
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
    /// The effective clear color used by the next render pass. Normally
    /// equals `clear_color`, but in raw mode we sync it to the alt-screen
    /// program's Normal bg so the sub-cell leftover space on the right and
    /// bottom edges of the grid (from integer cell math) doesn't leak the
    /// Flux theme color through vim's colorscheme.
    effective_clear_color: Color,
    /// Pre-allocated instance buffer — written to, never recreated during normal use.
    instance_buffer: wgpu::Buffer,
    instance_capacity: usize,
    instance_count: u32,
    /// Instances for the output grid — rebuilt on `set_grid`.
    output_instances: Vec<CellInstance>,
    /// Instances for the fixed input chrome — rebuilt on `set_input_line`.
    input_instances: Vec<CellInstance>,
    /// Default glyph style applied to cells with no bold/italic flags.
    /// Driven by `[font] weight = "bold"` / `style = "italic"` in the config
    /// file, so users can set a baseline weight the whole terminal inherits.
    default_style: GlyphStyle,
    /// Padding around the terminal grid (pixels).
    padding_x: f32,
    padding_y: f32,
    /// When true, `set_grid` shifts rows down so the shell's cursor row
    /// ends up at the bottom of the output area. Disabled in raw mode so
    /// alt-screen programs (vim, less) fill the full grid top-to-bottom.
    bottom_anchor: bool,
    /// When true, `set_grid` draws the shell's cursor block. Off by default
    /// because Flux's input editor owns cursor display in cooked mode; on in
    /// raw mode so alt-screen programs can show their own cursor.
    show_shell_cursor: bool,
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
        let atlas = atlas::GlyphAtlas::new(&gpu.device, &gpu.queue, font_family, font_size, line_height)?;
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
            effective_clear_color: Color::from_hex("#24283b").unwrap(),
            instance_buffer,
            instance_capacity: INITIAL_MAX_CELLS,
            instance_count: 0,
            output_instances: Vec::with_capacity(INITIAL_MAX_CELLS),
            input_instances: Vec::with_capacity(64),
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

    /// Clear the fixed input editor chrome from the next frame. Use when
    /// handing the screen to a full-screen program (vim, less, etc).
    pub fn hide_input_line(&mut self) {
        if self.input_instances.is_empty() {
            return;
        }
        self.input_instances.clear();
        self.rebuild_combined_buffer();
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
    pub fn rebuild_font(&mut self, font_family: &str, font_size: f32, line_height: f32) -> Result<()> {
        self.atlas = atlas::GlyphAtlas::new(&self.gpu.device, &self.gpu.queue, font_family, font_size, line_height)?;
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
        self.effective_clear_color = color;
    }

    /// Render a terminal grid — each cell at its grid position.
    ///
    /// The grid is bottom-anchored on the shell's cursor row: whichever row
    /// the shell is currently writing to is placed at the bottom of the
    /// output area, and earlier rows stack above it. Before the shell has
    /// produced enough output to fill the grid, the top of the output area
    /// is just blank. Once the shell overflows the grid and alacritty_terminal
    /// starts scrolling, the anchor is at row `rows-1` and behavior matches a
    /// normal top-anchored terminal.
    ///
    /// The shell's own cursor block is intentionally not drawn — Flux owns
    /// input via the fixed editor below, so the shell cursor is redundant
    /// noise. (Raw-mode bypass will need to re-enable it — see the raw-mode
    /// item on Phase 1.)
    pub fn set_grid(&mut self, grid: &flux_types::RenderGrid) {
        let cell_w = self.atlas.cell_width;
        let cell_h = self.atlas.cell_height;
        let baseline = self.atlas.baseline_offset;
        let pad_x = self.padding_x;
        let pad_y = self.padding_y;

        let y_shift = if self.bottom_anchor {
            let anchor_row = grid.cursor.map(|(_, r)| r).unwrap_or(grid.rows.saturating_sub(1));
            let last_row = grid.rows.saturating_sub(1);
            (last_row.saturating_sub(anchor_row)) as f32 * cell_h
        } else {
            0.0
        };

        // In raw mode, sync the effective clear to whatever bg the alt-screen
        // program is using — sampled from the top-left cell, which reliably
        // carries the Normal highlight bg for vim, nano, less, etc. In cooked
        // mode, reset to the user's configured theme bg.
        self.effective_clear_color = if !self.bottom_anchor && grid.rows > 0 && grid.cols > 0 {
            grid.get(0, 0).bg
        } else {
            self.clear_color
        };

        let mut instances = std::mem::take(&mut self.output_instances);
        instances.clear();

        // Draw the shell's cursor block first (so any underlying glyph paints on top).
        // Uses the full cell height so the cursor matches the line grid
        // uniformly regardless of which glyph sits under it.
        if self.show_shell_cursor {
            if let Some((col, row)) = grid.cursor {
                let cursor_x = pad_x + col as f32 * cell_w;
                let cursor_y = pad_y + row as f32 * cell_h + y_shift;
                let cursor_color = Color::from_hex("#c0caf5").unwrap_or(Color::default());
                instances.push(CellInstance {
                    position: [cursor_x, cursor_y],
                    size: [cell_w, cell_h],
                    glyph_uv: [0.0, 0.0, 0.0, 0.0],
                    fg_color: [cursor_color.r, cursor_color.g, cursor_color.b, cursor_color.a],
                    bg_color: [cursor_color.r, cursor_color.g, cursor_color.b, cursor_color.a],
                });
            }
        }

        let clear = self.effective_clear_color;

        for row in 0..grid.rows {
            for col in 0..grid.cols {
                let cell = grid.get(row, col);
                let cell_x = pad_x + col as f32 * cell_w;
                let cell_y = pad_y + row as f32 * cell_h + y_shift;
                let is_under_cursor = self.show_shell_cursor && grid.cursor == Some((col, row));

                // Paint the cell's background across the whole cell rect
                // when it differs from the effective clear color. Skip the
                // cursor cell — the cursor block drawn earlier is already
                // filling that cell, and overdrawing it here would erase the
                // cursor and leave only the small inverted-color glyph rect.
                if !is_under_cursor && !color_matches(cell.bg, clear) {
                    instances.push(CellInstance {
                        position: [cell_x, cell_y],
                        size: [cell_w, cell_h],
                        glyph_uv: [0.0, 0.0, 0.0, 0.0],
                        fg_color: [cell.bg.r, cell.bg.g, cell.bg.b, cell.bg.a],
                        bg_color: [cell.bg.r, cell.bg.g, cell.bg.b, cell.bg.a],
                    });
                }

                if cell.character == ' ' || cell.character == '\0' {
                    continue;
                }

                // When a cell has no style flags, fall back to the config-
                // level default — this is how `[font] weight` / `style` act
                // as a terminal-wide baseline (matching iTerm's profile
                // "default font" behavior). When the shell or an alt-screen
                // program sets BOLD or ITALIC explicitly, those win, which
                // lets vim/nano render a differentiated statusline against
                // an italic-baseline welcome screen.
                let has_flags = cell.flags.intersects(CellFlags::BOLD | CellFlags::ITALIC);
                let style = if has_flags {
                    GlyphStyle::from_flags(
                        cell.flags.contains(CellFlags::BOLD),
                        cell.flags.contains(CellFlags::ITALIC),
                    )
                } else {
                    self.default_style
                };

                // When the shell cursor sits on a glyph, invert its colors so it
                // reads against the cursor block (already pushed above).
                let (fg, bg) = if is_under_cursor {
                    let cursor_bg = Color::from_hex("#24283b").unwrap_or(Color::new(0.0, 0.0, 0.0, 1.0));
                    let cursor_fg = Color::from_hex("#c0caf5").unwrap_or(Color::default());
                    (cursor_bg, cursor_fg)
                } else {
                    (cell.fg, cell.bg)
                };
                self.render_glyph(cell.character, style, cell_x, cell_y, baseline, fg, bg, &mut instances);
            }
        }

        self.output_instances = instances;
        self.rebuild_combined_buffer();
    }

    /// Render the fixed input editor chrome at the bottom of the window.
    ///
    /// The input line is Flux chrome — it is not part of the shell grid. It
    /// lives at a fixed Y (one cell above the bottom padding), prefixed by
    /// `❯ `, with a block cursor at `cursor_col` and a dim divider row one
    /// cell above it.
    pub fn set_input_line(&mut self, text: &str, cursor_col: usize) {
        let cell_w = self.atlas.cell_width;
        let cell_h = self.atlas.cell_height;
        let baseline = self.atlas.baseline_offset;
        let pad_x = self.padding_x;
        let pad_y = self.padding_y;
        let window_w = self.gpu.surface_config.width as f32;
        let window_h = self.gpu.surface_config.height as f32;

        let mut instances = std::mem::take(&mut self.input_instances);
        instances.clear();

        // Input line sits on the bottom row (above bottom padding).
        let input_y = window_h - pad_y - cell_h;
        // Divider cell sits one row above the input.
        let divider_y = input_y - cell_h;

        // Dim horizontal rule centered in the divider row.
        let divider_color = Color::new(0.30, 0.33, 0.45, 1.0);
        let divider_thickness = 1.0;
        instances.push(CellInstance {
            position: [pad_x, divider_y + cell_h * 0.5 - divider_thickness * 0.5],
            size: [(window_w - pad_x * 2.0).max(0.0), divider_thickness],
            glyph_uv: [0.0, 0.0, 0.0, 0.0],
            fg_color: [divider_color.r, divider_color.g, divider_color.b, divider_color.a],
            bg_color: [divider_color.r, divider_color.g, divider_color.b, divider_color.a],
        });

        // Draw the prompt prefix in a slightly different color from the text.
        let prompt_color = Color::from_hex("#7aa2f7").unwrap_or(Color::default());
        let fg_color = Color::from_hex("#c0caf5").unwrap_or(Color::default());
        let bg_color = Color::from_hex("#24283b").unwrap_or(Color::new(0.0, 0.0, 0.0, 1.0));
        let cursor_color = Color::from_hex("#c0caf5").unwrap_or(Color::default());

        let prefix = "❯ ";
        let prefix_len = prefix.chars().count();
        let style = self.default_style;
        let mut col = 0;
        for ch in prefix.chars() {
            let x = pad_x + col as f32 * cell_w;
            if ch != ' ' {
                self.render_glyph(ch, style, x, input_y, baseline, prompt_color, bg_color, &mut instances);
            }
            col += 1;
        }

        // Cursor column in the full input row (after the prefix).
        let cursor_display_col = prefix_len + cursor_col;
        let cursor_x = pad_x + cursor_display_col as f32 * cell_w;

        // Cursor block — drawn before the glyph underneath so the glyph paints on top.
        // Full cell height so it matches the line grid uniformly.
        instances.push(CellInstance {
            position: [cursor_x, input_y],
            size: [cell_w, cell_h],
            glyph_uv: [0.0, 0.0, 0.0, 0.0],
            fg_color: [cursor_color.r, cursor_color.g, cursor_color.b, cursor_color.a],
            bg_color: [cursor_color.r, cursor_color.g, cursor_color.b, cursor_color.a],
        });

        // Draw the buffer text, inverting the glyph that sits under the cursor.
        for (i, ch) in text.chars().enumerate() {
            let x = pad_x + (prefix_len + i) as f32 * cell_w;
            if ch == ' ' {
                continue;
            }
            let is_under_cursor = i == cursor_col;
            let (fg, bg) = if is_under_cursor {
                (bg_color, cursor_color)
            } else {
                (fg_color, bg_color)
            };
            self.render_glyph(ch, style, x, input_y, baseline, fg, bg, &mut instances);
        }

        self.input_instances = instances;
        self.rebuild_combined_buffer();
    }

    /// Render a single glyph character at a grid position.
    /// `cell_x`, `cell_y` = top-left corner of the cell in screen pixels.
    /// `baseline_offset` = distance from cell top to the glyph baseline.
    /// The glyph's `placement_top` is the distance from baseline to top of the bitmap,
    /// `placement_left` is the horizontal offset from the cell's origin.
    fn render_glyph(
        &mut self,
        character: char,
        style: GlyphStyle,
        cell_x: f32,
        cell_y: f32,
        baseline_offset: f32,
        fg: Color,
        bg: Color,
        instances: &mut Vec<CellInstance>,
    ) {
        let region = self.atlas.lookup_char(&self.gpu.queue, character, style);
        if region.is_null() {
            return;
        }

        instances.push(CellInstance {
            position: [
                cell_x + region.placement_left,
                cell_y + baseline_offset - region.placement_top,
            ],
            size: [region.pixel_width, region.pixel_height],
            glyph_uv: region.uv,
            fg_color: [fg.r, fg.g, fg.b, fg.a],
            bg_color: [bg.r, bg.g, bg.b, bg.a],
        });
    }

    /// Rebuild the combined instance buffer from the persistent output and input vecs.
    /// Output instances come first, input instances second. Grows the GPU buffer if needed.
    fn rebuild_combined_buffer(&mut self) {
        let total = self.output_instances.len() + self.input_instances.len();
        if total == 0 {
            self.instance_count = 0;
            return;
        }

        if total > self.instance_capacity {
            self.instance_capacity = total * 2;
            self.instance_buffer = Self::create_instance_buffer(&self.gpu.device, self.instance_capacity);
            log::info!("Grew instance buffer to {} cells", self.instance_capacity);
        }

        if !self.output_instances.is_empty() {
            self.gpu.queue.write_buffer(
                &self.instance_buffer,
                0,
                bytemuck::cast_slice(&self.output_instances),
            );
        }

        if !self.input_instances.is_empty() {
            let offset = (self.output_instances.len() * std::mem::size_of::<CellInstance>()) as u64;
            self.gpu.queue.write_buffer(
                &self.instance_buffer,
                offset,
                bytemuck::cast_slice(&self.input_instances),
            );
        }

        self.instance_count = total as u32;
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

    /// Convert our Color to wgpu::Color. Uses the effective clear so
    /// raw-mode alt-screen programs fill the whole window with their bg,
    /// not just the cells they happen to cover.
    fn wgpu_clear_color(&self) -> wgpu::Color {
        wgpu::Color {
            r: self.effective_clear_color.r as f64,
            g: self.effective_clear_color.g as f64,
            b: self.effective_clear_color.b as f64,
            a: self.effective_clear_color.a as f64,
        }
    }

}
