//! Mux state — tabs and panes (#40).
//!
//! The foundation for tabs (Sprint 2) and splits (v0.4): the App owns a
//! `MuxState`; every keystroke, PTY read, and render goes through the
//! focused pane. Today there is exactly one tab with one pane, and the
//! app behaves as before — this module exists so tabs are a UI feature,
//! not a refactor. Splits later turn `Tab::pane` into a pane tree; the
//! seam is contained here.

use anyhow::{Context, Result};

use flux_terminal::pty::WakeCallback;
use flux_terminal::state::TerminalState;
use flux_terminal::{Domain, DomainId, PaneId, Pty};

/// A single shell instance: its PTY connection and terminal state.
pub struct Pane {
    #[allow(dead_code)] // addressed by the tab bar / pane routing (Sprint 2 UI)
    pub id: PaneId,
    #[allow(dead_code)] // read when multi-domain (ssh) lands in v0.5
    pub domain_id: DomainId,
    pub pty: Box<dyn Pty + Send>,
    pub terminal: TerminalState,
}

/// A workspace with its own pane. Splits (v0.4) turn this into a tree.
pub struct Tab {
    #[allow(dead_code)] // stable identity once tabs are reorderable
    pub id: u64,
    pub pane: Pane,
    /// Last title the shell set via OSC 0/2 — shown in the tab bar and
    /// applied to the window when the tab is focused.
    pub title: Option<String>,
}

/// All tabs, the focus, and the domains panes can spawn in.
pub struct MuxState {
    pub tabs: Vec<Tab>,
    pub current_tab: usize,
    domains: Vec<Box<dyn Domain>>,
    next_pane_id: PaneId,
    next_tab_id: u64,
}

impl MuxState {
    pub fn new() -> Self {
        Self {
            tabs: Vec::new(),
            current_tab: 0,
            domains: Vec::new(),
            next_pane_id: 0,
            next_tab_id: 0,
        }
    }

    /// Register a domain (its id comes from the domain itself).
    pub fn add_domain(&mut self, domain: Box<dyn Domain>) {
        self.domains.push(domain);
    }

    /// Spawn a shell in `domain_id` and wrap it in a new tab. The
    /// caller supplies the `TerminalState` (it carries config: theme,
    /// scrollback, blocks) and the wake callback for the event loop.
    pub fn create_tab(
        &mut self,
        domain_id: DomainId,
        cols: u16,
        rows: u16,
        wake: WakeCallback,
        terminal: TerminalState,
    ) -> Result<&mut Tab> {
        let domain = self
            .domains
            .iter()
            .find(|d| d.id() == domain_id)
            .with_context(|| format!("no domain with id {domain_id}"))?;
        let pty = domain.spawn_pane(cols, rows, wake)?;
        let pane = Pane {
            id: self.next_pane_id,
            domain_id,
            pty,
            terminal,
        };
        self.next_pane_id += 1;
        let tab = Tab {
            id: self.next_tab_id,
            pane,
            title: None,
        };
        self.next_tab_id += 1;
        self.tabs.push(tab);
        self.current_tab = self.tabs.len() - 1;
        Ok(self.tabs.last_mut().expect("just pushed"))
    }

    pub fn focused_pane(&self) -> Option<&Pane> {
        self.tabs.get(self.current_tab).map(|tab| &tab.pane)
    }

    pub fn focused_pane_mut(&mut self) -> Option<&mut Pane> {
        self.tabs.get_mut(self.current_tab).map(|tab| &mut tab.pane)
    }

    pub fn focused_tab(&self) -> Option<&Tab> {
        self.tabs.get(self.current_tab)
    }

    /// Focus tab `index` (0-based). Out of range is a no-op. Returns
    /// true if the focus changed.
    pub fn select_tab(&mut self, index: usize) -> bool {
        if index < self.tabs.len() && index != self.current_tab {
            self.current_tab = index;
            true
        } else {
            false
        }
    }

    /// Focus the next/previous tab, wrapping. `step` is +1 or -1.
    pub fn cycle_tab(&mut self, step: i32) -> bool {
        let n = self.tabs.len();
        if n < 2 {
            return false;
        }
        self.current_tab = (self.current_tab as i32 + step).rem_euclid(n as i32) as usize;
        true
    }

    /// Remove tab `index`. Focus moves to the tab that took its slot
    /// (or the new last tab). Returns true when no tabs remain.
    pub fn close_tab(&mut self, index: usize) -> bool {
        if index < self.tabs.len() {
            self.tabs.remove(index);
            match index.cmp(&self.current_tab) {
                std::cmp::Ordering::Less => self.current_tab -= 1,
                _ => {
                    self.current_tab = self.current_tab.min(self.tabs.len().saturating_sub(1));
                }
            }
        }
        self.tabs.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use flux_terminal::pty::PtyEvent;
    use flux_types::ResolvedTheme;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU16, Ordering};

    struct FakePty {
        written: Vec<u8>,
        cols: u16,
    }

    impl Pty for FakePty {
        fn write(&mut self, data: &[u8]) -> Result<()> {
            self.written.extend_from_slice(data);
            Ok(())
        }
        fn read_events(&self) -> Vec<PtyEvent> {
            Vec::new()
        }
        fn resize(&mut self, cols: u16, _rows: u16) -> Result<()> {
            self.cols = cols;
            Ok(())
        }
    }

    struct FakeDomain {
        id: DomainId,
        spawned: Arc<AtomicU16>,
    }

    impl Domain for FakeDomain {
        fn spawn_pane(
            &self,
            cols: u16,
            _rows: u16,
            _wake: WakeCallback,
        ) -> Result<Box<dyn Pty + Send>> {
            self.spawned.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(FakePty {
                written: Vec::new(),
                cols,
            }))
        }
        fn id(&self) -> DomainId {
            self.id
        }
        fn name(&self) -> &str {
            "fake"
        }
    }

    fn term() -> TerminalState {
        TerminalState::new(80, 24, 100, ResolvedTheme::default())
    }

    #[test]
    fn create_tab_spawns_through_the_domain_and_focuses_it() {
        let spawned = Arc::new(AtomicU16::new(0));
        let mut mux = MuxState::new();
        mux.add_domain(Box::new(FakeDomain {
            id: 7,
            spawned: spawned.clone(),
        }));
        assert!(mux.focused_pane().is_none(), "no tabs yet");

        mux.create_tab(7, 80, 24, Box::new(|| {}), term()).unwrap();
        assert_eq!(spawned.load(Ordering::SeqCst), 1);
        assert_eq!(mux.tabs.len(), 1);
        assert_eq!(mux.current_tab, 0);

        let pane = mux.focused_pane_mut().unwrap();
        pane.pty.write(b"ls\r").unwrap();
        pane.terminal.process_bytes(b"output\r\n");

        // A second tab takes focus; ids stay distinct.
        mux.create_tab(7, 80, 24, Box::new(|| {}), term()).unwrap();
        assert_eq!(mux.current_tab, 1);
        assert_ne!(mux.tabs[0].pane.id, mux.tabs[1].pane.id);
        assert_ne!(mux.tabs[0].id, mux.tabs[1].id);
    }

    fn mux_with_tabs(n: usize) -> MuxState {
        let mut mux = MuxState::new();
        mux.add_domain(Box::new(FakeDomain {
            id: 0,
            spawned: Arc::new(AtomicU16::new(0)),
        }));
        for _ in 0..n {
            mux.create_tab(0, 80, 24, Box::new(|| {}), term()).unwrap();
        }
        mux
    }

    #[test]
    fn select_and_cycle_wrap_and_ignore_out_of_range() {
        let mut mux = mux_with_tabs(3);
        assert_eq!(mux.current_tab, 2, "newest tab focused");
        assert!(mux.select_tab(0));
        assert!(!mux.select_tab(0), "already focused");
        assert!(!mux.select_tab(9), "out of range is a no-op");
        assert_eq!(mux.current_tab, 0);
        assert!(mux.cycle_tab(-1));
        assert_eq!(mux.current_tab, 2, "wraps backward");
        assert!(mux.cycle_tab(1));
        assert_eq!(mux.current_tab, 0, "wraps forward");
        let mut single = mux_with_tabs(1);
        assert!(!single.cycle_tab(1), "single tab has nothing to cycle");
    }

    #[test]
    fn close_tab_keeps_focus_sensible() {
        let mut mux = mux_with_tabs(3);
        mux.select_tab(1);
        // Closing a tab before the focused one shifts the index.
        assert!(!mux.close_tab(0));
        assert_eq!(mux.current_tab, 0);
        assert_eq!(mux.tabs.len(), 2);
        // Closing the focused last tab moves focus to the new last.
        mux.select_tab(1);
        assert!(!mux.close_tab(1));
        assert_eq!(mux.current_tab, 0);
        // Closing the final tab reports empty.
        assert!(mux.close_tab(0));
        assert!(mux.focused_pane().is_none());
    }

    #[test]
    fn create_tab_with_unknown_domain_errors() {
        let mut mux = MuxState::new();
        let result = mux.create_tab(99, 80, 24, Box::new(|| {}), term());
        let err = result.err().expect("unknown domain must error");
        assert!(err.to_string().contains("no domain"), "{err}");
    }
}
