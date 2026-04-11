//! Non-cell UI rendering — tab bar, block chrome, input area, dividers.
//!
//! Renders UI elements that aren't part of the terminal cell grid:
//! - Tab bar (colored rectangles + text labels)
//! - Input editor area (background + border + cursor)
//! - Block borders and headers (Phase 2)
//! - Divider lines between regions
//! - Scrollbar

// TODO: Phase 1, Step 5
// - Render input area background (colored quad)
// - Render divider line between output and input
// - Render cursor in input area
// Phase 3:
// - Render tab bar
// - Render block chrome
