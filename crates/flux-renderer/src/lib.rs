//! GPU rendering for the Flux terminal emulator.
//!
//! ALL wgpu code lives in this crate. Nothing outside flux-renderer
//! imports wgpu. Other crates interact through [flux_types] data
//! structures only.

mod atlas;
mod core;
mod glyph;
mod output;
mod renderer;
mod selection;
mod ui;

pub use atlas::GlyphStyle;
pub use renderer::{AltBgPolicy, CellMetrics, Renderer};
pub use ui::PopupKind;
