//! PTY management — spawns shells and handles I/O.
//!
//! Wraps portable-pty to provide:
//! - Shell spawning with proper environment setup
//! - Non-blocking reads from PTY output
//! - Writing user input to PTY
//! - Terminal resize handling

// TODO: Phase 1, Step 3
// - Spawn shell using portable-pty
// - Set up $TERM, $COLORTERM, etc.
// - Create reader thread for PTY output
// - Implement write() for sending input
// - Implement resize() for terminal size changes
