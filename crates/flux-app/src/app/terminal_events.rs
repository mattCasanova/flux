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
    /// Process pending PTY output through alacritty_terminal.
    pub(super) fn process_pty_output(&mut self) {
        let Some(pty) = &self.pty else { return };
        let Some(terminal) = &mut self.terminal else { return };

        let mut dirty = false;

        for event in pty.read_events() {
            match event {
                PtyEvent::Output(bytes) => {
                    terminal.process_bytes(&bytes);
                    dirty = true;
                }
                PtyEvent::Exited => {
                    log::info!("Shell exited");
                }
            }
        }

        // Handle events from alacritty_terminal (PtyWrite responses)
        if dirty {
            for event in terminal.drain_events() {
                match event {
                    TermEvent::PtyWrite(text) => {
                        if let Some(pty) = &mut self.pty {
                            let _ = pty.write(text.as_bytes());
                        }
                    }
                    TermEvent::Title(title) => {
                        if let Some(window) = &self.window {
                            window.set_title(&title);
                        }
                    }
                    TermEvent::Bell => {
                        log::debug!("Bell");
                    }
                }
            }

            // Raw-mode state can change on any PTY output (vim enters alt
            // screen on launch, fzf flips termios, etc.). Re-check before
            // rendering the next frame.
            self.sync_raw_mode();
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
    fn sync_raw_mode(&mut self) {
        let Some(terminal) = &self.terminal else { return };
        let raw = terminal.is_alt_screen();
        if raw == self.raw_mode {
            return;
        }
        self.raw_mode = raw;
        log::info!("Raw mode: {}", raw);

        if let Some(renderer) = &mut self.renderer {
            renderer.set_bottom_anchor(!raw);
            renderer.set_show_shell_cursor(raw);
        }

        // Recompute the grid dimensions so alt-screen programs get every
        // row, and restore the 2-row chrome when they exit.
        self.apply_window_layout();

        if raw {
            if let Some(renderer) = &mut self.renderer {
                renderer.hide_input_line();
            }
        } else {
            self.update_input_display();
        }
    }
}
