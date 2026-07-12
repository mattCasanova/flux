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
        if self.raw_mode {
            return;
        }

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
        if whole != 0.0 {
            self.scroll_accum -= whole;
            self.scroll_terminal(whole as i32);
        }
    }

    /// Scroll the output viewport by `lines` (positive = into history)
    /// and refresh the grid. Clears the selection — its coordinates are
    /// viewport-relative and would highlight the wrong text after the
    /// content shifts.
    pub(super) fn scroll_terminal(&mut self, lines: i32) {
        let Some(term) = &mut self.terminal else {
            return;
        };
        term.scroll_lines(lines);
        self.clear_selection();
        self.update_display();
        self.request_redraw();
    }

    pub(super) fn scroll_page(&mut self, up: bool) {
        let Some(term) = &mut self.terminal else {
            return;
        };
        if up {
            term.scroll_page_up();
        } else {
            term.scroll_page_down();
        }
        self.clear_selection();
        self.update_display();
        self.request_redraw();
    }

    /// Jump back to the live tail. Called when the user submits a
    /// command while scrolled up — the standard "typing returns you to
    /// the present" behavior.
    pub(super) fn snap_to_bottom(&mut self) {
        let Some(term) = &mut self.terminal else {
            return;
        };
        if term.display_offset() > 0 {
            term.scroll_to_bottom();
            self.update_display();
        }
    }
}
