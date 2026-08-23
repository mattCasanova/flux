//! Flux-owned UI elements — everything drawn on top of the terminal
//! output that isn't shell content. Each submodule adds an
//! `impl Renderer` block with methods for a specific UI element.

pub mod input_bar;
mod popup;
pub mod sidebar;
mod titlebar;

pub use popup::PopupKind;
