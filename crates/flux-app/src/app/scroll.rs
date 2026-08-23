//! Scrollback viewport control — mouse wheel and keyboard scrolling.
//!
//! Cooked mode only: alt-screen programs (vim, less) own their own
//! viewport and receive scroll keys directly, so raw mode never routes
//! here. Alacritty keeps the viewport stable when new output lands
//! while scrolled up (the offset grows internally), so there is no
//! snap-back gate on the output path — only an explicit snap when the
//! user submits a command.

use winit::event::MouseScrollDelta;

use super::App;

impl App {
    pub(super) fn handle_mouse_wheel(&mut self, delta: MouseScrollDelta) {
        let cell_h = self
            .renderer
            .as_ref()
            .map(|r| r.cell_metrics().height)
            .unwrap_or(16.0);

        // Both delta kinds accumulate fractionally: trackpads emit many
        // sub-line pixel deltas, and some mice report half-line ticks.
        // Whole lines are consumed; the remainder carries to the next
        // event so slow scrolling still moves.
        let lines = match delta {
            MouseScrollDelta::LineDelta(_, y) => y,
            MouseScrollDelta::PixelDelta(pos) => pos.y as f32 / cell_h,
        };
        self.scroll_accum += lines;
        let whole = self.scroll_accum.trunc();
        if whole == 0.0 {
            return;
        }
        self.scroll_accum -= whole;
        let n = whole as i32;

        if self.raw_mode {
            // Alt screen has no scrollback. The standard behaviors:
            // programs that requested mouse reporting (Claude Code,
            // htop, vim mouse=a) get real wheel events in their
            // protocol; otherwise translate the wheel into arrow keys
            // (DECSET 1007 alternate-scroll) so vim / less scroll
            // their own content.
            let (wants_mouse, alt_scroll, app_cursor) = match self.terminal() {
                Some(t) => (
                    t.wants_mouse_reporting(),
                    t.alternate_scroll(),
                    t.app_cursor_keys(),
                ),
                None => return,
            };

            if wants_mouse {
                use super::mouse::{MOUSE_BTN_WHEEL_DOWN, MOUSE_BTN_WHEEL_UP};
                let Some(cell) = self.pixel_to_cell(self.mouse.last_cursor_pos) else {
                    return;
                };
                let button = if n > 0 {
                    MOUSE_BTN_WHEEL_UP
                } else {
                    MOUSE_BTN_WHEEL_DOWN
                };
                for _ in 0..n.unsigned_abs() {
                    self.forward_mouse(button, cell, true);
                }
                return;
            }

            if !alt_scroll {
                return;
            }
            let seq: &[u8] = match (app_cursor, n > 0) {
                (true, true) => b"\x1bOA",
                (true, false) => b"\x1bOB",
                (false, true) => b"\x1b[A",
                (false, false) => b"\x1b[B",
            };
            if let Some(pane) = self.pane_mut() {
                let pty = &mut pane.pty;
                for _ in 0..n.unsigned_abs() {
                    let _ = pty.write(seq);
                }
            }
            return;
        }

        self.scroll_terminal(n);
    }

    /// Scroll the output viewport by `lines` (positive = into history)
    /// and refresh the grid. Selections survive: they're anchored to
    /// content, not viewport rows.
    pub(super) fn scroll_terminal(&mut self, lines: i32) {
        let Some(pane) = self.pane_mut() else {
            return;
        };
        pane.terminal.scroll_lines(lines);
        self.update_display();
        self.request_redraw();
    }

    pub(super) fn scroll_page(&mut self, up: bool) {
        let Some(pane) = self.pane_mut() else {
            return;
        };
        if up {
            pane.terminal.scroll_page_up();
        } else {
            pane.terminal.scroll_page_down();
        }
        self.update_display();
        self.request_redraw();
    }

    /// Jump back to the live tail. Called when the user submits a
    /// command while scrolled up — the standard "typing returns you to
    /// the present" behavior.
    pub(super) fn snap_to_bottom(&mut self) {
        let Some(pane) = self.pane_mut() else {
            return;
        };
        if pane.terminal.display_offset() > 0 {
            pane.terminal.scroll_to_bottom();
            self.update_display();
        }
    }

    /// Move the block selection up/down (Cmd+Up / Cmd+Down): the
    /// highlight steps to the previous / next block and scrolls it
    /// into view — Warp's block-cursor feel.
    pub(super) fn step_block_selection(&mut self, step: i32) {
        let moved = self
            .pane_mut()
            .map(|pane| pane.terminal.select_block_step(step))
            .unwrap_or(false);
        if moved {
            self.update_display();
            self.request_redraw();
        }
    }
}
