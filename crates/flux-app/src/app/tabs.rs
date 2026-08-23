//! Tab orchestration — create, close, switch (Sprint 2).
//!
//! One pane per tab. The tab bar renders only with 2+ tabs, so a
//! single-tab session keeps the chrome-free look. Shortcuts (handled
//! in keyboard.rs): Cmd+T new, Cmd+W close, Cmd+1-9 jump, Cmd+[ / ]
//! cycle.

use flux_terminal::state::TerminalState;

use super::App;
use crate::mux::SplitAxis;

/// How long a "press again to close" confirmation stays armed.
const CONFIRM_WINDOW: std::time::Duration = std::time::Duration::from_secs(3);

impl App {
    /// Open a new tab with a fresh shell and focus it.
    pub(super) fn new_tab(&mut self) {
        let (cols, rows) = self.grid_dims_for_new_pane();
        let mut terminal = TerminalState::new(
            cols.max(1),
            rows.max(1),
            self.config.scrollback.lines,
            self.config.theme.resolve(),
        );
        terminal.set_blocks_enabled(self.config.blocks.enabled);
        let proxy = self.proxy.clone();
        let wake = Box::new(move || {
            let _ = proxy.send_event(());
        });
        let editor = self.new_editor();
        match self.mux.create_tab(
            0,
            cols.max(1) as u16,
            rows.max(1) as u16,
            wake,
            terminal,
            editor,
        ) {
            Ok(_) => self.after_tab_switch(),
            Err(e) => log::error!("new tab failed: {e:#}"),
        }
    }

    /// Close the focused tab; the app exits when the last one goes.
    /// If any pane in the tab has a command running, the first press
    /// only warns (title flashes "press again to close"); a second
    /// press within `CONFIRM_WINDOW` closes (#58).
    pub(super) fn close_current_tab(&mut self) {
        let index = self.mux.current_tab;
        let busy = self
            .mux
            .focused_tab()
            .map(|tab| tab.root.panes().iter().any(|p| p.terminal.is_executing()))
            .unwrap_or(false);
        if busy && !self.take_close_confirmation(("tab", index as u64)) {
            self.arm_close_confirmation(("tab", index as u64), "tab has a running command");
            return;
        }
        if self.mux.close_tab(index) {
            self.shell_exited = true;
            self.request_redraw();
        } else {
            self.after_tab_switch();
        }
    }

    /// A shell exited: drop its pane; if that empties its tab, close
    /// the tab (and the app when it was the last one).
    pub(super) fn close_tab_with_pane(&mut self, pane_id: u64) {
        if let Some(renderer) = &mut self.renderer {
            renderer.remove_pane(pane_id);
        }
        match self.mux.remove_pane_anywhere(pane_id) {
            Some(tab_index) => {
                let was_current = tab_index == self.mux.current_tab;
                if self.mux.close_tab(tab_index) {
                    self.shell_exited = true;
                    self.request_redraw();
                } else if was_current {
                    self.after_tab_switch();
                } else {
                    self.update_tab_bar();
                }
            }
            None => self.after_tab_switch(),
        }
    }

    // ---- splits ----

    /// Split the focused pane; the new shell goes right (Horizontal)
    /// or below (Vertical) and takes focus.
    pub(super) fn split_focused(&mut self, axis: SplitAxis) {
        let (cols, rows) = self.grid_dims_for_new_pane();
        let mut terminal = TerminalState::new(
            cols.max(1),
            rows.max(1),
            self.config.scrollback.lines,
            self.config.theme.resolve(),
        );
        terminal.set_blocks_enabled(self.config.blocks.enabled);
        let proxy = self.proxy.clone();
        let wake = Box::new(move || {
            let _ = proxy.send_event(());
        });
        let editor = self.new_editor();
        match self.mux.split_focused(
            axis,
            0,
            cols.max(1) as u16,
            rows.max(1) as u16,
            wake,
            terminal,
            editor,
        ) {
            Ok(_) => self.after_tab_switch(),
            Err(e) => log::error!("split failed: {e:#}"),
        }
    }

    /// Close the focused pane; closing a tab's last pane closes the tab.
    /// Same two-press confirmation as tabs when a command is running.
    pub(super) fn close_focused_pane(&mut self) {
        let focused = self.mux.focused_pane().map(|p| p.id);
        let busy = self
            .mux
            .focused_pane()
            .map(|p| p.terminal.is_executing())
            .unwrap_or(false);
        if let Some(id) = focused
            && busy
            && !self.take_close_confirmation(("pane", id))
        {
            self.arm_close_confirmation(("pane", id), "pane has a running command");
            return;
        }
        if self.mux.close_focused_pane() {
            self.close_current_tab();
            return;
        }
        if let (Some(renderer), Some(id)) = (&mut self.renderer, focused) {
            renderer.remove_pane(id);
        }
        self.after_tab_switch();
    }

    /// Focus the pane in direction (dx, dy) from the focused one.
    pub(super) fn focus_pane_direction(&mut self, dx: i32, dy: i32) {
        let moved = self
            .mux
            .focused_tab_mut()
            .map(|tab| tab.focus_direction(dx, dy))
            .unwrap_or(false);
        if moved {
            self.after_tab_switch();
        }
    }

    /// Focus pane `id` in the current tab (mouse click).
    pub(super) fn focus_pane(&mut self, id: u64) {
        if let Some(tab) = self.mux.focused_tab_mut()
            && tab.root.contains(id)
            && tab.focus != id
        {
            tab.focus = id;
            self.after_tab_switch();
        }
    }

    pub(super) fn select_tab(&mut self, index: usize) {
        if self.mux.select_tab(index) {
            self.after_tab_switch();
        }
    }

    pub(super) fn cycle_tab(&mut self, step: i32) {
        if self.mux.cycle_tab(step) {
            self.after_tab_switch();
        }
    }

    /// Arm a close confirmation for `target` and show a notice.
    fn arm_close_confirmation(&mut self, target: (&'static str, u64), why: &str) {
        self.close_confirm = Some((target, std::time::Instant::now()));
        let text = format!("{why} — press again to close");
        if let Some(renderer) = &mut self.renderer {
            renderer.set_notice(&text);
        }
        self.request_redraw();
    }

    /// True (and disarms) if `target` was armed within the window.
    fn take_close_confirmation(&mut self, target: (&'static str, u64)) -> bool {
        let armed = matches!(
            self.close_confirm,
            Some((t, at)) if t == target && at.elapsed() <= CONFIRM_WINDOW
        );
        self.close_confirm = None;
        if let Some(renderer) = &mut self.renderer {
            renderer.hide_notice();
        }
        armed
    }

    /// Everything that must happen when the focused tab changes (or
    /// the tab count does): re-fit the pane to the window, re-sync
    /// raw-mode chrome, window title, tab bar, and both displays.
    pub(super) fn after_tab_switch(&mut self) {
        // The search bar is per-pane state; don't carry it across.
        if matches!(self.popup, super::PopupState::Search) {
            self.close_search();
        }
        // Panes of the previous tab (or a closed pane) must not linger.
        if let Some(renderer) = &mut self.renderer {
            renderer.clear_panes();
        }
        self.apply_window_layout();
        self.sync_raw_mode();
        self.apply_focused_title();
        self.update_tab_bar();
        self.update_display();
        self.update_input_display();
        self.request_redraw();
    }

    /// Push the current tab labels to the renderer.
    pub(super) fn update_tab_bar(&mut self) {
        let titles: Vec<String> = self.mux.tabs.iter().map(tab_label).collect();
        let focused = self.mux.current_tab;
        if let Some(renderer) = &mut self.renderer {
            renderer.set_tab_bar(&titles, focused);
        }
    }

    /// Which tab a click at pixel position lands on, if it's inside
    /// the tab bar. Tabs split the window evenly, so the slot is just
    /// `x / (width / n)` — matching what's painted by construction.
    pub(super) fn tab_at_pixel(&self, x: f64, y: f64) -> Option<usize> {
        let n = self.mux.tabs.len();
        if n < 2 {
            return None;
        }
        let metrics = self.renderer.as_ref()?.cell_metrics();
        if y < 0.0 || y >= metrics.height as f64 {
            return None;
        }
        let width = self.window.as_ref()?.inner_size().width as f64;
        if width <= 0.0 || x < 0.0 || x >= width {
            return None;
        }
        Some(((x / (width / n as f64)) as usize).min(n - 1))
    }

    /// Window title follows the focused tab.
    pub(super) fn apply_focused_title(&mut self) {
        let title = self
            .mux
            .focused_tab()
            .map(tab_label)
            .unwrap_or_else(|| self.config.window.title.clone());
        if let Some(window) = &self.window {
            window.set_title(&title);
        }
    }

    /// Grid dimensions a new pane should get — same math as
    /// `apply_window_layout`, evaluated for the current window.
    fn grid_dims_for_new_pane(&self) -> (usize, usize) {
        self.mux
            .focused_pane()
            .map(|pane| (pane.terminal.cols(), pane.terminal.rows()))
            .unwrap_or((80, 24))
    }
}

/// Label for a tab: the shell-set title if any, else the cwd's last
/// component, else the shell.
fn tab_label(tab: &crate::mux::Tab) -> String {
    if let Some(title) = &tab.title
        && !title.is_empty()
    {
        return title.clone();
    }
    tab.focused_pane()
        .and_then(|p| p.terminal.cwd())
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "shell".to_string())
}
