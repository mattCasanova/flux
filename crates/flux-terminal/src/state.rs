//! Wraps alacritty_terminal::Term with a clean interface.
//!
//! Handles:
//! - Creating the Term instance
//! - Feeding PTY bytes through the vte parser
//! - Converting the grid to a RenderGrid
//! - Tracking dirty state for efficient rendering

// TODO: Phase 1, Step 4
// - Create Term with Config, dimensions, EventListener
// - Implement EventListener to handle PtyWrite, Bell, Title, etc.
// - Process PTY output: processor.advance(&mut term, &bytes)
// - Convert grid to RenderGrid: iterate term.renderable_content()
