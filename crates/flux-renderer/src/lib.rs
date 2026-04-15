//! GPU rendering for the Flux terminal emulator.
//!
//! ALL wgpu code lives in this crate. Nothing outside flux-renderer
//! imports wgpu. Other crates interact through [flux_types] data
//! structures only.

mod atlas;
mod buffer;
mod cell_renderer;
mod glyph;
mod gpu;
mod gpu_resources;
mod input_chrome;
mod output;
mod pipeline;
mod render_pass;
mod renderer;

pub use atlas::GlyphStyle;
pub use renderer::{CellMetrics, Renderer};
