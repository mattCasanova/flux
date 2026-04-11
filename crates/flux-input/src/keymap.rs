//! Key event to terminal escape sequence translation.
//!
//! Maps special keys (arrows, F-keys, Home/End, etc.) to the
//! VT100/xterm escape sequences that the shell expects.

// TODO: Phase 1, Step 6
// - Arrow keys → \x1b[A/B/C/D
// - Home/End → \x1b[H / \x1b[F
// - PageUp/Down → \x1b[5~ / \x1b[6~
// - F1-F12 → \x1bOP through \x1b[24~
// - Delete → \x1b[3~
// - Modifier encoding: ESC [ 1 ; <mods> <terminator>
// See: research/research-winit-keyboard.md for full mapping
