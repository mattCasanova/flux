//! Mouse selection — click/drag state machine and pixel→cell mapping.
//!
//! Cooked mode only for v0.2: raw-mode programs (vim, less) have their
//! own mouse support that would need xterm mouse-protocol encoding to
//! the PTY — deferred (see build plan F12 risks).
//!
//! Selection coordinates are **viewport-relative** grid cells. Anything
//! that shifts what those cells show (scrolling, new PTY output)
//! clears the selection rather than trying to track content — absolute
//! line identity is v0.3 block-spike territory.

use std::time::{Duration, Instant};

use winit::dpi::PhysicalPosition;
use winit::event::{ElementState, MouseButton};

use flux_types::{CellPos, Color, Selection, SelectionMode};

use super::App;

const MULTI_CLICK_WINDOW: Duration = Duration::from_millis(500);
const MULTI_CLICK_DIST_PX: f64 = 5.0;

pub(crate) struct MouseState {
    last_click_at: Option<Instant>,
    last_click_pos: Option<PhysicalPosition<f64>>,
    click_count: u32,
    is_dragging: bool,
    pub(super) last_cursor_pos: PhysicalPosition<f64>,
    /// Left button is held and events are being forwarded to the PTY
    /// (program requested mouse reporting).
    forwarding_drag: bool,
    /// Last cell a forwarded drag event was sent for — dedupes the
    /// sub-cell motion spam.
    last_forwarded_cell: Option<CellPos>,
}

impl Default for MouseState {
    fn default() -> Self {
        Self {
            last_click_at: None,
            last_click_pos: None,
            click_count: 0,
            is_dragging: false,
            last_cursor_pos: PhysicalPosition::new(0.0, 0.0),
            forwarding_drag: false,
            last_forwarded_cell: None,
        }
    }
}

/// xterm mouse protocol button codes.
pub(super) const MOUSE_BTN_LEFT: u8 = 0;
pub(super) const MOUSE_BTN_WHEEL_UP: u8 = 64;
pub(super) const MOUSE_BTN_WHEEL_DOWN: u8 = 65;
const MOUSE_DRAG_FLAG: u8 = 32;

/// SGR encoding (DECSET 1006): `\x1b[<btn;col;row(M|m)`, 1-based cells.
fn encode_sgr(button: u8, cell: CellPos, pressed: bool) -> Vec<u8> {
    format!(
        "\x1b[<{};{};{}{}",
        button,
        cell.col + 1,
        cell.row + 1,
        if pressed { 'M' } else { 'm' }
    )
    .into_bytes()
}

/// Legacy X10 encoding: `\x1b[M` + three bytes offset by 32. Cells past
/// 223 can't be encoded — return None and drop the event.
fn encode_legacy(button: u8, cell: CellPos, pressed: bool) -> Option<Vec<u8>> {
    // Legacy has no per-button release — releases are always button 3.
    let b = if pressed { button } else { 3 };
    let cx = cell.col + 1 + 32;
    let cy = cell.row + 1 + 32;
    if cx > 255 || cy > 255 {
        return None;
    }
    Some(vec![0x1b, b'[', b'M', 32 + b, cx as u8, cy as u8])
}

impl App {
    pub(super) fn handle_mouse_moved(&mut self, pos: PhysicalPosition<f64>) {
        self.mouse.last_cursor_pos = pos;

        // Forwarded drag: stream cell-granular drag events to the
        // program while the left button is held, if it asked for them.
        if self.mouse.forwarding_drag {
            let reports_drag = self
                .terminal
                .as_ref()
                .map(|t| t.reports_mouse_drag())
                .unwrap_or(false);
            if reports_drag
                && let Some(cell) = self.pixel_to_cell(pos)
                && self.mouse.last_forwarded_cell != Some(cell)
            {
                self.mouse.last_forwarded_cell = Some(cell);
                self.forward_mouse(MOUSE_BTN_LEFT | MOUSE_DRAG_FLAG, cell, true);
            }
            return;
        }

        if !self.mouse.is_dragging || self.selection.is_none() {
            return;
        }
        let Some(cell) = self.pixel_to_cell(pos) else {
            return;
        };
        let grid = self.snapshot_for_selection();
        if let Some(sel) = &mut self.selection {
            sel.extend_to(cell);
            if let Some(grid) = &grid {
                sel.snap_to_words(grid);
            }
        }
        self.refresh_selection_render();
        self.request_redraw();
    }

    pub(super) fn handle_mouse_input(&mut self, state: ElementState, button: MouseButton) {
        if !matches!(button, MouseButton::Left) {
            return;
        }

        // When the program requested mouse reporting (vim `mouse=a`,
        // htop, Claude Code), clicks belong to it — encode and forward.
        // Shift is the xterm-standard bypass: Shift+drag always makes a
        // local Flux selection regardless of what the program wants.
        let program_owns_mouse = self.raw_mode
            && !self.modifiers.shift_key()
            && self
                .terminal
                .as_ref()
                .map(|t| t.wants_mouse_reporting())
                .unwrap_or(false);
        if program_owns_mouse {
            let Some(cell) = self.pixel_to_cell(self.mouse.last_cursor_pos) else {
                return;
            };
            match state {
                ElementState::Pressed => {
                    self.mouse.forwarding_drag = true;
                    self.mouse.last_forwarded_cell = Some(cell);
                    self.forward_mouse(MOUSE_BTN_LEFT, cell, true);
                }
                ElementState::Released => {
                    self.mouse.forwarding_drag = false;
                    self.mouse.last_forwarded_cell = None;
                    self.forward_mouse(MOUSE_BTN_LEFT, cell, false);
                }
            }
            return;
        }

        match state {
            ElementState::Pressed => self.handle_mouse_pressed(),
            ElementState::Released => {
                self.mouse.is_dragging = false;
                // A click that never dragged selects nothing — clear the
                // degenerate single-cell selection so Cmd+C falls back to
                // its non-selection behavior.
                if let Some(sel) = &self.selection
                    && sel.is_degenerate()
                {
                    self.clear_selection();
                }
            }
        }
    }

    /// Encode one mouse event in the program's requested protocol and
    /// write it to the PTY.
    pub(super) fn forward_mouse(&mut self, button: u8, cell: CellPos, pressed: bool) {
        let sgr = self
            .terminal
            .as_ref()
            .map(|t| t.sgr_mouse())
            .unwrap_or(false);
        let bytes = if sgr {
            Some(encode_sgr(button, cell, pressed))
        } else {
            encode_legacy(button, cell, pressed)
        };
        if let Some(bytes) = bytes
            && let Some(pty) = &mut self.pty
        {
            let _ = pty.write(&bytes);
        }
    }

    fn handle_mouse_pressed(&mut self) {
        let pos = self.mouse.last_cursor_pos;

        // Multi-click detection: same spot, within the double-click window.
        let now = Instant::now();
        let is_repeat = self
            .mouse
            .last_click_at
            .map(|t| now.duration_since(t) < MULTI_CLICK_WINDOW)
            .unwrap_or(false)
            && self
                .mouse
                .last_click_pos
                .map(|p| {
                    (p.x - pos.x).abs() < MULTI_CLICK_DIST_PX
                        && (p.y - pos.y).abs() < MULTI_CLICK_DIST_PX
                })
                .unwrap_or(false);
        self.mouse.click_count = if is_repeat {
            // 4th click wraps back to a fresh single click.
            if self.mouse.click_count >= 3 {
                1
            } else {
                self.mouse.click_count + 1
            }
        } else {
            1
        };
        self.mouse.last_click_at = Some(now);
        self.mouse.last_click_pos = Some(pos);
        self.mouse.is_dragging = true;

        let Some(cell) = self.pixel_to_cell(pos) else {
            // Click outside the output area (input bar, blank anchor
            // space) clears any existing selection.
            self.clear_selection();
            return;
        };

        let base_mode = match self.mouse.click_count {
            2 => SelectionMode::Word,
            3 => SelectionMode::Line,
            _ => SelectionMode::Character,
        };
        let mode = if self.modifiers.alt_key() {
            SelectionMode::Block
        } else {
            base_mode
        };

        if self.modifiers.shift_key() && self.selection.is_some() {
            if let Some(sel) = &mut self.selection {
                sel.extend_to(cell);
            }
        } else {
            self.selection = Some(Selection::new(cell, mode));
        }

        let grid = self.snapshot_for_selection();
        if let (Some(sel), Some(grid)) = (&mut self.selection, &grid) {
            sel.snap_to_words(grid);
        }

        self.refresh_selection_render();
        self.request_redraw();
    }

    /// Map a physical pixel position to an output-grid cell. Returns
    /// `None` outside the output area (padding, bottom-anchor blank
    /// space above the output, or the input bar below it).
    pub(super) fn pixel_to_cell(&self, pos: PhysicalPosition<f64>) -> Option<CellPos> {
        let renderer = self.renderer.as_ref()?;
        let term = self.terminal.as_ref()?;
        let metrics = renderer.cell_metrics();

        let scale = self
            .window
            .as_ref()
            .map(|w| w.scale_factor())
            .unwrap_or(1.0);
        let pad_x = self.config.window.padding_horizontal as f64 * scale;
        let pad_y = self.config.window.padding_vertical as f64 * scale;

        let x = pos.x - pad_x;
        let y = pos.y - pad_y;
        if x < 0.0 || y < 0.0 {
            return None;
        }

        let col = (x / metrics.width as f64) as usize;
        let visual_row = (y / metrics.height as f64) as usize;

        // Undo the bottom-anchor shift: visual row N shows grid row
        // N - y_shift_rows. Clicks in the blank space above the output
        // have no grid cell.
        let row = visual_row.checked_sub(renderer.current_y_shift_rows())?;

        if row >= term.rows() || col >= term.cols() {
            return None;
        }
        Some(CellPos { col, row })
    }

    /// Grid snapshot for word-boundary snapping and text extraction.
    /// Colors are irrelevant for both, so defaults are fine.
    pub(super) fn snapshot_for_selection(&self) -> Option<flux_types::TerminalGrid> {
        self.terminal
            .as_ref()
            .map(|t| t.grid_snapshot(Color::default(), Color::default()))
    }

    pub(super) fn refresh_selection_render(&mut self) {
        let cols = self.terminal.as_ref().map(|t| t.cols()).unwrap_or(0);
        if let Some(renderer) = &mut self.renderer {
            renderer.set_selection(self.selection.as_ref(), cols);
        }
    }

    pub(super) fn clear_selection(&mut self) {
        if self.selection.is_some() {
            self.selection = None;
            self.refresh_selection_render();
            self.request_redraw();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sgr_press_release_and_wheel() {
        let cell = CellPos { col: 4, row: 9 };
        assert_eq!(
            encode_sgr(MOUSE_BTN_LEFT, cell, true),
            b"\x1b[<0;5;10M".to_vec()
        );
        assert_eq!(
            encode_sgr(MOUSE_BTN_LEFT, cell, false),
            b"\x1b[<0;5;10m".to_vec()
        );
        assert_eq!(
            encode_sgr(MOUSE_BTN_WHEEL_UP, cell, true),
            b"\x1b[<64;5;10M".to_vec()
        );
    }

    #[test]
    fn legacy_encoding_offsets_by_32() {
        let cell = CellPos { col: 0, row: 0 };
        // press left: btn 32+0, col 33, row 33
        assert_eq!(
            encode_legacy(MOUSE_BTN_LEFT, cell, true),
            Some(vec![0x1b, b'[', b'M', 32, 33, 33])
        );
        // release is always button 3 in legacy
        assert_eq!(
            encode_legacy(MOUSE_BTN_LEFT, cell, false),
            Some(vec![0x1b, b'[', b'M', 35, 33, 33])
        );
    }

    #[test]
    fn legacy_encoding_drops_far_cells() {
        let cell = CellPos { col: 300, row: 0 };
        assert_eq!(encode_legacy(MOUSE_BTN_LEFT, cell, true), None);
    }
}
