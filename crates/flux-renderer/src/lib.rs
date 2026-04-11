//! GPU rendering for the Flux terminal emulator.
//!
//! ALL wgpu code lives in this crate. Nothing outside flux-renderer
//! imports wgpu. Other crates interact through [flux_types] data
//! structures only.
//!
//! ## Modules
//!
//! - [gpu] — wgpu device, queue, surface setup
//! - [atlas] — glyph atlas (cosmic-text rasterization + etagere packing)
//! - [pipeline] — render pipeline creation (shaders, bind groups)
//! - [cell_renderer] — instanced cell rendering
//! - [ui_renderer] — non-cell UI (tab bar, block chrome, input area)

mod gpu;
mod atlas;
mod pipeline;
mod cell_renderer;
mod ui_renderer;

use flux_types::{CellData, Color, RenderGrid, Rect};

/// Screen layout — defines where each UI region lives.
pub struct ScreenLayout {
    pub output_area: Rect,
    pub input_area: Rect,
    pub tab_bar: Rect,
}

/// Data needed to render a single tab in the tab bar.
pub struct TabRenderData {
    pub title: String,
    pub is_active: bool,
    pub has_activity: bool,
    pub color: Option<Color>,
}

/// Everything needed to render one frame.
pub struct FrameData {
    pub layout: ScreenLayout,
    pub output_grid: RenderGrid,
    pub input_text: Vec<CellData>,
    pub input_cursor_pos: usize,
    pub tabs: Vec<TabRenderData>,
    pub active_tab: usize,
    pub clear_color: Color,
}

/// Cell dimensions in pixels — needed for grid size calculations.
pub struct CellMetrics {
    pub width: f32,
    pub height: f32,
}

// TODO: Implement the Renderer and wire up all modules.
// Phase 1, Step 1: gpu.rs — create wgpu device + surface
// Phase 1, Step 2: atlas.rs — glyph rasterization + packing
// Phase 1, Step 4: cell_renderer.rs — instanced cell rendering
// Phase 1, Step 5: ui_renderer.rs — input area rendering
