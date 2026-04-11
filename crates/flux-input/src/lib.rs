//! Input editor, keybindings, and completion for Flux.
//!
//! Handles the fixed input prompt at the bottom of the screen:
//! - Text buffer with cursor positioning
//! - Key event routing (editor vs PTY passthrough)
//! - Command history navigation
//! - Keybinding management
//!
//! ## Modules
//!
//! - [editor] — input editor buffer
//! - [keybindings] — keybinding manager
//! - [keymap] — key event to terminal escape sequence translation

mod editor;
mod keybindings;
mod keymap;
// mod completion;  // Phase 4

pub use editor::{InputEditor, InputAction};
pub use keybindings::{KeybindingManager, Action};
