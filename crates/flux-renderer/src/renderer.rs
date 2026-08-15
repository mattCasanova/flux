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
use crate::core::{
    CellInstance, GpuContext, INITIAL_MAX_CELLS, Uniforms, create_bind_group,
    create_bind_group_layout, create_cell_pipeline, create_instance_buffer, create_quad_buffer,
    create_sampler, create_uniform_buffer,
};
use flux_types::Color;

/// Cell dimensions in pixels.
pub struct CellMetrics {
    pub width: f32,
    pub height: f32,
    pub baseline_offset: f32,
}

/// How the padding / clear color behaves while an alt-screen program
/// owns the grid.
#[derive(Debug, Copy, Clone, PartialEq)]
pub enum AltBgPolicy {
    /// Adopt the program's background (majority vote over the grid
    /// perimeter) so vim et al fill the window edge-to-edge.
    Sync,
    /// Keep the user's theme background.
    Theme,
    /// Always use this fixed color.
    Fixed(Color),
}

/// The renderer — owns all GPU state and renders frames.
pub struct Renderer {
    pub(crate) gpu: GpuContext,
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
    /// Per-pane output instances — rebuilt on `set_pane_grid`.
    pub(crate) pane_instances: std::collections::HashMap<u64, Vec<CellInstance>>,
    /// Per-pane bottom-anchor shift in rows (mouse mapping reads it).
    pub(crate) pane_y_shift: std::collections::HashMap<u64, usize>,
    /// Per-pane scrollbar instances.
    pub(crate) pane_scrollbars: std::collections::HashMap<u64, Vec<CellInstance>>,
    /// Split dividers + focused-pane accent, rebuilt by `set_pane_frames`.
    pub(crate) frame_instances: Vec<CellInstance>,
    /// Instances for the fixed input chrome — rebuilt on `set_input_line`.
    pub(crate) input_instances: Vec<CellInstance>,
    /// Instances for popup overlays (F7 autocomplete, F14 search
    /// overlay, future command palette, etc.). Paint order is
    /// output → selection → input → popup, so popups render on top of
    /// everything else.
    pub(crate) popup_instances: Vec<CellInstance>,
    /// Translucent highlight rects for the active mouse selection
    /// (F12). Drawn after output so the tint composites over the grid,
    /// before input/popup so chrome stays readable.
    pub(crate) selection_instances: Vec<CellInstance>,
    /// Tab bar across the top (only with 2+ tabs) — rebuilt by
    /// `set_tab_bar`, drawn with the input chrome.
    pub(crate) tab_instances: Vec<CellInstance>,
    /// Pixels the terminal content area is pushed down from the top
    /// padding — the tab bar's height when it's visible. Applied by
    /// output/selection/scrollbar rendering; the app adds the same
    /// offset in pixel→cell mapping.
    pub(crate) content_top: f32,
    /// Padding behavior under alt-screen programs. See [AltBgPolicy].
    pub(crate) alt_bg_policy: AltBgPolicy,
    /// Optional padding tint while the viewport is scrolled into
    /// history (cooked mode) — a "not at the live tail" cue.
    pub(crate) scrolled_bg: Option<Color>,
    /// Debounce state for Sync mode: the currently winning perimeter
    /// color (quantized) and how many consecutive frames it has won.
    /// A new color is only committed to the padding after a stable
    /// streak, so partial repaints can't flicker the frame.
    pub(crate) alt_bg_candidate: Option<[u8; 4]>,
    pub(crate) alt_bg_streak: u32,
    /// The committed alt-screen padding color. None = no stable
    /// majority yet → fall back to the theme background.
    pub(crate) alt_bg_committed: Option<Color>,
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
        let gpu = GpuContext::new(window)?;
        let atlas =
            atlas::GlyphAtlas::new(&gpu.device, &gpu.queue, font_family, font_size, line_height)?;
        let quad_vertex_buffer = create_quad_buffer(&gpu.device);
        let uniform_buffer = create_uniform_buffer(&gpu.device, &gpu.surface_config);
        let sampler = create_sampler(&gpu.device);
        let bind_group_layout = create_bind_group_layout(&gpu.device);
        let pipeline = create_cell_pipeline(&gpu.device, gpu.format(), &bind_group_layout);
        let bind_group = create_bind_group(
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
            pane_instances: std::collections::HashMap::new(),
            pane_y_shift: std::collections::HashMap::new(),
            pane_scrollbars: std::collections::HashMap::new(),
            frame_instances: Vec::new(),
            input_instances: Vec::with_capacity(64),
            popup_instances: Vec::new(),
            selection_instances: Vec::new(),
            tab_instances: Vec::new(),
            content_top: 0.0,
            alt_bg_policy: AltBgPolicy::Sync,
            scrolled_bg: None,
            alt_bg_candidate: None,
            alt_bg_streak: 0,
            alt_bg_committed: None,
            padding_x: 0.0,
            padding_y: 0.0,
            bottom_anchor: true,
            show_shell_cursor: false,
            default_style,
        })
    }

    /// Set the horizontal and vertical padding between the window edge and the grid.
    /// Push the content area down by `pixels` (the tab bar height).
    pub fn set_content_top(&mut self, pixels: f32) {
        self.content_top = pixels;
    }

    pub fn content_top(&self) -> f32 {
        self.content_top
    }

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

    /// Rows of blank space above the output in bottom-anchor mode, as
    /// of the last `set_grid`. The app's pixel→cell mapping subtracts
    /// this so clicks land on the grid rows actually shown.
    /// Bottom-anchor shift (rows) of `pane_id`'s last frame — mouse
    /// mapping needs it to invert the layout.
    pub fn pane_y_shift_rows(&self, pane_id: u64) -> usize {
        self.pane_y_shift.get(&pane_id).copied().unwrap_or(0)
    }

    /// Forget every pane (tab switch): the new tab's panes repaint from
    /// scratch and nothing from the old tab lingers.
    pub fn clear_panes(&mut self) {
        self.pane_instances.clear();
        self.pane_y_shift.clear();
        self.pane_scrollbars.clear();
        self.frame_instances.clear();
        self.rebuild_combined_buffer();
    }

    /// Forget a pane that closed.
    pub fn remove_pane(&mut self, pane_id: u64) {
        self.pane_instances.remove(&pane_id);
        self.pane_y_shift.remove(&pane_id);
        self.pane_scrollbars.remove(&pane_id);
        self.rebuild_combined_buffer();
    }

    /// Draw split dividers (`gutters`) and an accent along the focused
    /// pane's top edge when there is more than one pane.
    pub fn set_pane_frames(
        &mut self,
        gutters: &[flux_types::Rect],
        focused: Option<flux_types::Rect>,
    ) {
        let divider = Color::new(0.30, 0.33, 0.45, 1.0);
        let accent = Color::new(0.478, 0.635, 0.969, 0.9);
        let rect = |r: flux_types::Rect, c: Color| CellInstance {
            position: [r.x, r.y],
            size: [r.width, r.height],
            glyph_uv: [0.0, 0.0, 0.0, 0.0],
            fg_color: [c.r, c.g, c.b, c.a],
            bg_color: [c.r, c.g, c.b, c.a],
        };
        self.frame_instances.clear();
        for g in gutters {
            // A 1px line centered in the gutter.
            let line = if g.width < g.height {
                flux_types::Rect::new(g.x + (g.width - 1.0) * 0.5, g.y, 1.0, g.height)
            } else {
                flux_types::Rect::new(g.x, g.y + (g.height - 1.0) * 0.5, g.width, 1.0)
            };
            self.frame_instances.push(rect(line, divider));
        }
        if let Some(f) = focused
            && !gutters.is_empty()
        {
            self.frame_instances.push(rect(
                flux_types::Rect::new(f.x, f.y - 2.0, f.width, 1.5),
                accent,
            ));
        }
        self.rebuild_combined_buffer();
    }

    #[allow(dead_code)] // single-pane callers; kept for the wrapper API
    pub fn current_y_shift_rows(&self) -> usize {
        self.pane_y_shift_rows(0)
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
        self.bind_group = create_bind_group(
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
        self.gpu
            .queue
            .write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));
    }

    pub fn set_clear_color(&mut self, color: Color) {
        self.clear_color = color;
        self.effective_clear_color = color;
    }

    pub fn set_alt_bg_policy(&mut self, policy: AltBgPolicy) {
        self.alt_bg_policy = policy;
    }

    pub fn set_scrolled_background(&mut self, color: Option<Color>) {
        self.scrolled_bg = color;
    }
}
