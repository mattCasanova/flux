//! The Domain abstraction — "how to spawn a PTY in this context".
//!
//! v0.4's single most important decision (see plans/milestones/
//! v0.4-multipane.md): only [`LocalDomain`](crate::local_domain::
//! LocalDomain) exists today, but the mux daemon (UnixDomain), spawned
//! ssh (SshDomainSimple), auto-tmux (SshDomainTmux) and native russh
//! (SshDomainNative) each become a drop-in trait impl instead of a
//! refactor through the event loop.

use anyhow::Result;

use crate::pty::{PtyEvent, PtyManager, WakeCallback};

/// Unique identifier for a domain within a session.
pub type DomainId = u64;
/// Unique identifier for a pane within a session.
pub type PaneId = u64;

/// One end of a spawned shell — everything the app needs from a PTY,
/// abstracted so remote panes can satisfy it too.
pub trait Pty: Send {
    /// Write bytes to the shell (user input).
    fn write(&mut self, data: &[u8]) -> Result<()>;
    /// Drain pending output events. Non-blocking.
    fn read_events(&self) -> Vec<PtyEvent>;
    /// Resize the shell's view.
    fn resize(&mut self, cols: u16, rows: u16) -> Result<()>;
    /// True when the child has the PTY in termios-raw mode. Remote
    /// domains that can't know report `false`.
    fn is_raw_mode(&self) -> bool {
        false
    }
}

impl Pty for PtyManager {
    fn write(&mut self, data: &[u8]) -> Result<()> {
        PtyManager::write(self, data)
    }

    fn read_events(&self) -> Vec<PtyEvent> {
        PtyManager::read_events(self)
    }

    fn resize(&mut self, cols: u16, rows: u16) -> Result<()> {
        PtyManager::resize(self, cols, rows)
    }

    fn is_raw_mode(&self) -> bool {
        PtyManager::is_raw_mode(self)
    }
}

/// A place panes can be spawned — the local machine today; a mux
/// daemon or an SSH host later.
pub trait Domain: Send {
    /// Spawn a new shell in this domain. `wake` is invoked from the
    /// reader side whenever output arrives so the event loop can
    /// schedule a redraw.
    fn spawn_pane(&self, cols: u16, rows: u16, wake: WakeCallback) -> Result<Box<dyn Pty + Send>>;

    /// Unique identifier for this domain.
    fn id(&self) -> DomainId;

    /// Human-readable name ("local", "ssh:devbox", …).
    fn name(&self) -> &str;
}
