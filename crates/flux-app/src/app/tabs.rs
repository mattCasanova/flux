//! Tab orchestration — create, close, switch (Sprint 2).
//!
//! One pane per tab. The tab bar renders only with 2+ tabs, so a
//! single-tab session keeps the chrome-free look. Shortcuts (handled
//! in keyboard.rs): Cmd+T new, Cmd+W close, Cmd+1-9 jump, Cmd+[ / ]
//! cycle.

use flux_terminal::state::TerminalState;

use super::App;

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
        match self
            .mux
            .create_tab(0, cols.max(1) as u16, rows.max(1) as u16, wake, terminal)
        {
            Ok(_) => self.after_tab_switch(),
            Err(e) => log::error!("new tab failed: {e:#}"),
        }
    }

    /// Close the focused tab; the app exits when the last one goes.
    /// (Confirm-close with running processes is #58.)
    pub(super) fn close_current_tab(&mut self) {
        let index = self.mux.current_tab;
        if self.mux.close_tab(index) {
            self.shell_exited = true;
            self.request_redraw();
        } else {
            self.after_tab_switch();
        }
    }

    /// Close the tab whose shell exited (found by pane id).
    pub(super) fn close_tab_with_pane(&mut self, pane_id: u64) {
        let Some(index) = self.mux.tabs.iter().position(|tab| tab.pane.id == pane_id) else {
            return;
        };
        let was_focused = index == self.mux.current_tab;
        if self.mux.close_tab(index) {
            self.shell_exited = true;
            self.request_redraw();
        } else if was_focused {
            self.after_tab_switch();
        } else {
            self.update_tab_bar();
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

    /// Everything that must happen when the focused tab changes (or
    /// the tab count does): re-fit the pane to the window, re-sync
    /// raw-mode chrome, window title, tab bar, and both displays.
    pub(super) fn after_tab_switch(&mut self) {
        self.apply_window_layout();
        self.sync_raw_mode();
        self.apply_focused_title();
        self.update_tab_bar();
        self.update_display();
        if !self.raw_mode {
            self.update_input_display();
        }
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
    tab.pane
        .terminal
        .cwd()
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "shell".to_string())
}
