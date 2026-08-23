//! PTY output processing + alt-screen detection.
//!
//! `process_pty_output` drains bytes from the PTY into the terminal
//! state each time a `user_event` wakeup fires, then forwards any
//! shell-generated responses (title changes, `PtyWrite` replies) back
//! out. `sync_raw_mode` watches for alt-screen transitions (vim / less
//! / htop / fzf) and reconfigures the renderer chrome accordingly.

use flux_terminal::pty::PtyEvent;
use flux_terminal::state::TermEvent;

use super::App;

impl App {
    /// Process pending PTY output through alacritty_terminal — for
    /// EVERY pane, not just the focused one, so a build running in a
    /// background tab keeps flowing (and its channel doesn't grow
    /// without bound). Only the focused pane triggers a repaint.
    pub(super) fn process_pty_output(&mut self) {
        let mut focused_dirty = false;
        let mut any_title = false;
        let mut exited_panes: Vec<u64> = Vec::new();
        let focused_id = self.mux.focused_pane().map(|pane| pane.id);
        let current_tab = self.mux.current_tab;
        let mut titles: Vec<(usize, String)> = Vec::new();

        for (tab_idx, pane) in self.mux.all_panes_mut() {
            let mut dirty = false;
            for event in pane.pty.read_events() {
                match event {
                    PtyEvent::Output(bytes) => {
                        pane.terminal.process_bytes(&bytes);
                        dirty = true;
                    }
                    PtyEvent::Exited => {
                        exited_panes.push(pane.id);
                    }
                }
            }
            if !dirty {
                continue;
            }
            // Any pane of the current tab is on screen.
            if tab_idx == current_tab {
                focused_dirty = true;
            }
            for event in pane.terminal.drain_events() {
                match event {
                    TermEvent::PtyWrite(text) => {
                        let _ = pane.pty.write(text.as_bytes());
                    }
                    TermEvent::Title(title) => {
                        // Only the focused pane names its tab.
                        if Some(pane.id) == focused_id {
                            titles.push((tab_idx, title));
                            any_title = true;
                        }
                    }
                    TermEvent::Bell => {
                        log::debug!("Bell");
                    }
                }
            }
        }
        for (tab_idx, title) in titles {
            if let Some(tab) = self.mux.tabs.get_mut(tab_idx) {
                tab.title = Some(title);
            }
        }

        for pane_id in exited_panes {
            self.close_tab_with_pane(pane_id);
        }

        if any_title {
            self.apply_focused_title();
            self.update_tab_bar();
        }

        if focused_dirty {
            // Raw-mode state can change on any PTY output (vim enters alt
            // screen on launch, fzf flips termios, etc.). Re-check before
            // rendering the next frame.
            self.sync_raw_mode();

            // Selections are content-anchored (alacritty's model), so
            // new output does NOT invalidate them — alacritty rotates
            // the anchors with the scrollback and drops the selection
            // itself if the content scrolls out of history.
            self.update_display();
        }
    }

    /// Detect whether a full-screen program is on the other end of the PTY
    /// and, if the state just changed, resize the grid and toggle chrome.
    ///
    /// Uses `TermMode::ALT_SCREEN` as the sole signal — vim, less, man, htop,
    /// tmux, fzf (default) and top all set the alt-screen bit. We deliberately
    /// do NOT check termios here: every interactive shell (zsh zle, bash
    /// readline, fish) keeps the PTY in termios-raw mode whenever it's ready
    /// for input, so `tcgetattr` is a false-positive trap — it fires as soon
    /// as the shell prints its first prompt. Password prompts and other
    /// termios-only raw-mode programs that skip alt-screen are a follow-up
    /// (tracked separately).
    pub(super) fn sync_raw_mode(&mut self) {
        let Some(terminal) = self.terminal() else {
            return;
        };
        let raw = terminal.is_alt_screen();
        if raw == self.raw_mode {
            return;
        }
        self.raw_mode = raw;
        log::info!("Raw mode (focused pane alt screen): {}", raw);

        // The grid contents are about to be swapped wholesale (entering
        // or leaving the alt screen) — any selection now points at the
        // wrong content.
        self.clear_selection();

        // Chrome is per-pane: update_input_display notices the pane's
        // alt transition (chrome_dirty), relayouts JUST that pane, and
        // rebuilds the bars. Bottom-anchor and cursor visibility are
        // decided per pane per frame in update_display.
        self.update_input_display();
    }
}
