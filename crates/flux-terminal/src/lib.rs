//! Terminal state, PTY management, and ANSI parsing for Flux.
//!
//! This crate wraps alacritty_terminal and portable-pty behind a clean
//! interface. No GPU code — produces [flux_types::RenderGrid] for the
//! renderer to consume.

pub mod pty;
pub mod state;

/// The terminal's input mode — determines where keystrokes go.
pub enum InputMode {
    /// Input editor captures keystrokes. Normal operation.
    Editor,
    /// Forward all keys directly to PTY (vim, ssh, fzf, etc.)
    Passthrough,
}
