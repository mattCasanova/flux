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

pub mod autocomplete;
mod editor;
pub mod history;
mod keybindings;
mod keymap;

pub use autocomplete::{Autocomplete, CandidateKind};
pub use editor::InputEditor;
pub use history::CommandHistory;
pub use keybindings::{Action, KeybindingManager};
