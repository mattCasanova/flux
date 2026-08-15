//! Mouse selection — click/drag state machine, pixel→cell mapping,
//! xterm mouse-protocol forwarding, and drag-to-autoscroll.
//!
//! Selection lives in the TERMINAL (alacritty's content-anchored
//! model, absolute scrollback coordinates), not in viewport cells:
//! it survives scrolling and new output, and dragging past the edge
//! of the output area auto-scrolls so a selection can span far more
//! than one screen. Highlighting arrives back via the SELECTION cell
//! flag on grid snapshots.
//!
//! When a raw-mode program has requested mouse reporting (vim
//! `mouse=a`, htop, Claude Code), events are encoded and forwarded to
//! the PTY instead; Shift bypasses forwarding for a local selection.

use std::time::{Duration, Instant};

use winit::dpi::PhysicalPosition;
use winit::event::{ElementState, MouseButton};

use flux_terminal::state::SelectMode;
use flux_types::CellPos;

use super::App;

const MULTI_CLICK_WINDOW: Duration = Duration::from_millis(500);
const MULTI_CLICK_DIST_PX: f64 = 5.0;
/// Minimum interval between drag-autoscroll steps (per scrolled line).
const AUTOSCROLL_TICK: Duration = Duration::from_millis(30);

pub(crate) struct MouseState {
    last_click_at: Option<Instant>,
    last_click_pos: Option<PhysicalPosition<f64>>,
    click_count: u32,
    is_dragging: bool,
    pub(super) last_cursor_pos: PhysicalPosition<f64>,
    /// Lines-per-tick drag-autoscroll: positive = scrolling up into
    /// history (pointer above the output), negative = down, 0 = off.
    autoscroll: i32,
    last_autoscroll: Option<Instant>,
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
            autoscroll: 0,
            last_autoscroll: None,
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
/// "No button" + motion flag — hover events for DECSET 1003 tracking.
const MOUSE_BTN_HOVER: u8 = 3 + MOUSE_DRAG_FLAG;

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
                .terminal()
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

        // Hover motion (DECSET 1003 any-event tracking): programs like
        // Claude Code want to see the pointer move even with no button
        // held. Shift suppresses forwarding, consistent with the
        // selection bypass.
        if !self.mouse.is_dragging
            && self.raw_mode
            && !self.modifiers.shift_key()
            && self
                .terminal()
                .map(|t| t.reports_mouse_motion())
                .unwrap_or(false)
        {
            if let Some(cell) = self.pixel_to_cell(pos)
                && self.mouse.last_forwarded_cell != Some(cell)
            {
                self.mouse.last_forwarded_cell = Some(cell);
                self.forward_mouse(MOUSE_BTN_HOVER, cell, true);
            }
            return;
        }

        if !self.mouse.is_dragging {
            return;
        }

        // Local selection drag. Pointer past the top/bottom edge of
        // the output area arms autoscroll (stepped from redraws so it
        // keeps scrolling while the pointer rests at the edge).
        self.mouse.autoscroll = self.autoscroll_demand(pos);
        let (cell, right_side) = self.pixel_to_cell_clamped(pos);
        if let Some(pane) = self.pane_mut() {
            let term = &mut pane.terminal;
            term.update_selection(cell.col, cell.row, right_side);
        }
        self.update_display();
        if self.mouse.autoscroll != 0 {
            self.step_drag_autoscroll();
        }
        self.request_redraw();
    }

    pub(super) fn handle_mouse_input(&mut self, state: ElementState, button: MouseButton) {
        if !matches!(button, MouseButton::Left) {
            return;
        }

        // Tab bar clicks switch tabs — checked first, in every mode
        // (the bar sits outside the grid, so nothing below cares).
        if matches!(state, ElementState::Pressed) {
            let pos = self.mouse.last_cursor_pos;
            if let Some(index) = self.tab_at_pixel(pos.x, pos.y) {
                self.select_tab(index);
                return;
            }
            // Clicking another pane focuses it; the click continues as a
            // click in that pane.
            if let Some(id) = self.pane_at_pixel(pos)
                && self.mux.focused_pane().map(|p| p.id) != Some(id)
            {
                self.focus_pane(id);
            }
        }

        // When the program requested mouse reporting (vim `mouse=a`,
        // htop, Claude Code), clicks belong to it — encode and forward.
        // Shift is the xterm-standard bypass: Shift+drag always makes a
        // local Flux selection regardless of what the program wants.
        let program_owns_mouse = self.raw_mode
            && !self.modifiers.shift_key()
            && self
                .terminal()
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
                self.mouse.autoscroll = 0;
                self.maybe_load_block_command();
            }
        }
    }

    /// A plain single click (no drag) on a block's header row loads
    /// that command into the input bar — re-run is then just Enter
    /// (issue #26's minimal form). Anything that selected text is a
    /// selection, not a click.
    fn maybe_load_block_command(&mut self) {
        if self.raw_mode || self.mouse.click_count != 1 {
            return;
        }
        let selected = self
            .terminal()
            .map(|t| t.selection_text().is_some())
            .unwrap_or(false);
        if selected {
            return;
        }
        let Some(cell) = self.pixel_to_cell(self.mouse.last_cursor_pos) else {
            return;
        };
        let Some(command) = self
            .terminal()
            .and_then(|t| t.block_command_at_row(cell.row))
        else {
            return;
        };
        self.clear_selection();
        self.input.clear();
        self.input.insert_str(&command);
        self.update_input_display();
        self.request_redraw();
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
        let right_side = self.pointer_in_right_half(pos);

        let base_mode = match self.mouse.click_count {
            2 => SelectMode::Word,
            3 => SelectMode::Line,
            _ => SelectMode::Char,
        };
        let mode = if self.modifiers.alt_key() {
            SelectMode::Block
        } else {
            base_mode
        };

        let shift = self.modifiers.shift_key();
        if let Some(pane) = self.pane_mut() {
            let term = &mut pane.terminal;
            if shift && term.has_selection() {
                term.update_selection(cell.col, cell.row, right_side);
            } else {
                term.start_selection(mode, cell.col, cell.row, right_side);
            }
        }
        self.update_display();
        self.request_redraw();
    }

    /// Lines-per-tick the drag wants to autoscroll, from how far the
    /// pointer sits past the output area's top (positive, into
    /// history) or bottom (negative). Farther past the edge = faster.
    fn autoscroll_demand(&self, pos: PhysicalPosition<f64>) -> i32 {
        let Some(renderer) = self.renderer.as_ref() else {
            return 0;
        };
        let Some(term) = self.terminal() else {
            return 0;
        };
        let cell_h = renderer.cell_metrics().height as f64;
        let scale = self
            .window
            .as_ref()
            .map(|w| w.scale_factor())
            .unwrap_or(1.0);
        let pad_y = self.config.window.padding_vertical as f64 * scale;
        let top = pad_y;
        let bottom = pad_y + term.rows() as f64 * cell_h;

        if pos.y < top {
            (((top - pos.y) / cell_h).ceil() as i32).min(5)
        } else if pos.y > bottom {
            -((((pos.y - bottom) / cell_h).ceil() as i32).min(5))
        } else {
            0
        }
    }

    /// One throttled autoscroll step while a drag rests past the edge.
    /// Called from mouse-move and from redraw (which re-arms itself via
    /// request_redraw, so scrolling continues while the pointer is
    /// still).
    pub(super) fn step_drag_autoscroll(&mut self) {
        if !self.mouse.is_dragging || self.mouse.autoscroll == 0 {
            return;
        }
        let now = Instant::now();
        let due = self
            .mouse
            .last_autoscroll
            .map(|t| now.duration_since(t) >= AUTOSCROLL_TICK)
            .unwrap_or(true);
        if !due {
            // Keep frames coming so the next due tick fires.
            self.request_redraw();
            return;
        }
        self.mouse.last_autoscroll = Some(now);

        let lines = self.mouse.autoscroll;
        let (cell, right_side) = self.pixel_to_cell_clamped(self.mouse.last_cursor_pos);
        if let Some(pane) = self.pane_mut() {
            let term = &mut pane.terminal;
            term.scroll_lines(lines);
            // Re-pin the head to the edge cell — the content moved
            // beneath the pointer, extending the selection.
            term.update_selection(cell.col, cell.row, right_side);
        }
        self.update_display();
        self.request_redraw();
    }

    /// The focused pane's viewport, cell metrics, and bottom-anchor
    /// shift — everything pixel→cell mapping needs.
    fn focused_pane_geometry(&self) -> Option<(flux_types::Rect, f64, f64, usize, usize, usize)> {
        let renderer = self.renderer.as_ref()?;
        let pane = self.mux.focused_pane()?;
        let m = renderer.cell_metrics();
        Some((
            pane.viewport,
            m.width as f64,
            m.height as f64,
            renderer.pane_y_shift_rows(pane.id),
            pane.terminal.cols(),
            pane.terminal.rows(),
        ))
    }

    /// Which pane of the current tab a pixel lands in, if any.
    pub(super) fn pane_at_pixel(&self, pos: PhysicalPosition<f64>) -> Option<u64> {
        let tab = self.mux.focused_tab()?;
        tab.root.panes().into_iter().find_map(|p| {
            let r = p.viewport;
            let inside = pos.x >= r.x as f64
                && pos.x < (r.x + r.width) as f64
                && pos.y >= r.y as f64
                && pos.y < (r.y + r.height) as f64;
            inside.then_some(p.id)
        })
    }

    /// Map a physical pixel position to a cell of the FOCUSED pane's
    /// grid. Returns `None` outside that pane (padding, other panes,
    /// bottom-anchor blank space above the output, the input bar).
    pub(super) fn pixel_to_cell(&self, pos: PhysicalPosition<f64>) -> Option<CellPos> {
        let (vp, cell_w, cell_h, y_shift_rows, cols, rows) = self.focused_pane_geometry()?;

        let x = pos.x - vp.x as f64;
        let y = pos.y - vp.y as f64;
        if x < 0.0 || y < 0.0 || x >= vp.width as f64 || y >= vp.height as f64 {
            return None;
        }

        let col = (x / cell_w) as usize;
        let visual_row = (y / cell_h) as usize;

        // Undo the bottom-anchor shift: visual row N shows grid row
        // N - y_shift_rows. Clicks in the blank space above the output
        // have no grid cell.
        let row = visual_row.checked_sub(y_shift_rows)?;

        if row >= rows || col >= cols {
            return None;
        }
        Some(CellPos { col, row })
    }

    /// Like `pixel_to_cell`, but clamps to the nearest edge cell so a
    /// drag past the boundary keeps extending the selection. Also
    /// reports which half of the cell the pointer is in.
    fn pixel_to_cell_clamped(&self, pos: PhysicalPosition<f64>) -> (CellPos, bool) {
        let (vp, cell_w, cell_h, y_shift_rows, cols, rows) =
            self.focused_pane_geometry().unwrap_or((
                flux_types::Rect::new(0.0, 0.0, 0.0, 0.0),
                8.0,
                16.0,
                0,
                1,
                1,
            ));

        let x = (pos.x - vp.x as f64).max(0.0);
        let y = (pos.y - vp.y as f64).max(0.0);
        let col = ((x / cell_w) as usize).min(cols.saturating_sub(1));
        let visual_row = (y / cell_h) as usize;
        let row = visual_row
            .saturating_sub(y_shift_rows)
            .min(rows.saturating_sub(1));
        let right_side = (x / cell_w).fract() >= 0.5;
        (CellPos { col, row }, right_side)
    }

    fn pointer_in_right_half(&self, pos: PhysicalPosition<f64>) -> bool {
        self.pixel_to_cell_clamped(pos).1
    }

    /// Encode one mouse event in the program's requested protocol and
    /// write it to the PTY.
    pub(super) fn forward_mouse(&mut self, button: u8, cell: CellPos, pressed: bool) {
        let sgr = self.terminal().map(|t| t.sgr_mouse()).unwrap_or(false);
        let bytes = if sgr {
            Some(encode_sgr(button, cell, pressed))
        } else {
            encode_legacy(button, cell, pressed)
        };
        if let Some(bytes) = bytes
            && let Some(pane) = self.pane_mut()
        {
            let _ = pane.pty.write(&bytes);
        }
    }

    pub(super) fn clear_selection(&mut self) {
        let had = self.terminal().map(|t| t.has_selection()).unwrap_or(false);
        if had {
            if let Some(pane) = self.pane_mut() {
                let term = &mut pane.terminal;
                term.clear_terminal_selection();
            }
            self.update_display();
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
