//! Terminal state, PTY management, and ANSI parsing for Flux.
//!
//! This crate wraps alacritty_terminal and portable-pty behind a clean
//! interface. No GPU code — produces [flux_types::RenderGrid] for the
//! renderer to consume.
//!
//! ## Modules
//!
//! - [state] — wraps alacritty_terminal::Term
//! - [pty] — wraps portable-pty for shell spawning
//! - [block] — block data model + BlockManager (Phase 2)
//! - [osc] — OSC 133 parser for shell integration (Phase 2)
//! - [mode] — terminal mode detection (raw, canonical, alt screen)

mod state;
mod pty;
// mod block;  // Phase 2
// mod osc;    // Phase 2
// mod mode;   // Phase 1, Step 7

/// The terminal's input mode — determines where keystrokes go.
pub enum InputMode {
    /// Input editor captures keystrokes. Normal operation.
    Editor,
    /// Forward all keys directly to PTY (vim, ssh, fzf, etc.)
    Passthrough,
}

// TODO: Phase 1, Step 3-4
// - Implement TerminalState wrapping alacritty_terminal::Term
// - Implement PtyManager wrapping portable-pty
// - Feed PTY output through vte parser into Term
// - Convert Term grid to RenderGrid for the renderer
// - Detect InputMode from ALT_SCREEN + tcgetattr
