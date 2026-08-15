//! Mux state — tabs, panes, and the split tree (#40, v0.4).
//!
//! The App owns a `MuxState`; every keystroke, PTY read, and render
//! goes through the focused pane of the current tab. A tab holds a
//! binary tree of panes (WezTerm's shape): leaves are shells, inner
//! nodes are splits with an axis and a ratio. Viewports are computed
//! by walking the tree over the content rectangle.

use anyhow::{Context, Result};

use flux_terminal::pty::WakeCallback;
use flux_terminal::state::TerminalState;
use flux_terminal::{Domain, DomainId, PaneId, Pty};
use flux_types::Rect;

/// A single shell instance: its PTY connection and terminal state.
pub struct Pane {
    pub id: PaneId,
    #[allow(dead_code)] // read when multi-domain (ssh) lands in v0.5
    pub domain_id: DomainId,
    pub pty: Box<dyn Pty + Send>,
    pub terminal: TerminalState,
    /// Pixel rectangle this pane paints into (set by layout).
    pub viewport: Rect,
}

/// Which way a split divides its space.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SplitAxis {
    /// Panes side by side (left | right).
    Horizontal,
    /// Panes stacked (top / bottom).
    Vertical,
}

/// The pane tree: leaves are shells, inner nodes split their space
/// between two children at `ratio` (first child's share, 0..1).
pub enum PaneNode {
    Leaf(Box<Pane>),
    Split {
        axis: SplitAxis,
        ratio: f32,
        first: Box<PaneNode>,
        second: Box<PaneNode>,
    },
}

/// Gap between split panes, in pixels (a divider line is drawn in it).
pub const SPLIT_GUTTER: f32 = 6.0;

impl PaneNode {
    /// Assign a viewport to every leaf by walking the tree over
    /// `available`. Split children are separated by `SPLIT_GUTTER`.
    pub fn layout(&mut self, available: Rect) {
        match self {
            PaneNode::Leaf(pane) => pane.viewport = available,
            PaneNode::Split {
                axis,
                ratio,
                first,
                second,
            } => {
                let (a, b) = split_rect(available, *axis, *ratio);
                first.layout(a);
                second.layout(b);
            }
        }
    }

    /// Divider line rectangles between split children (for drawing),
    /// in the same coordinates as the last `layout`.
    pub fn dividers(&self, available: Rect, out: &mut Vec<Rect>) {
        if let PaneNode::Split {
            axis,
            ratio,
            first,
            second,
        } = self
        {
            let (a, b) = split_rect(available, *axis, *ratio);
            out.push(match axis {
                SplitAxis::Horizontal => {
                    Rect::new(a.x + a.width, a.y, b.x - (a.x + a.width), a.height)
                }
                SplitAxis::Vertical => {
                    Rect::new(a.x, a.y + a.height, a.width, b.y - (a.y + a.height))
                }
            });
            first.dividers(a, out);
            second.dividers(b, out);
        }
    }

    pub fn panes(&self) -> Vec<&Pane> {
        let mut out = Vec::new();
        self.collect(&mut out);
        out
    }

    pub fn panes_mut(&mut self) -> Vec<&mut Pane> {
        let mut out = Vec::new();
        self.collect_mut(&mut out);
        out
    }

    fn collect<'a>(&'a self, out: &mut Vec<&'a Pane>) {
        match self {
            PaneNode::Leaf(pane) => out.push(&**pane),
            PaneNode::Split { first, second, .. } => {
                first.collect(out);
                second.collect(out);
            }
        }
    }

    fn collect_mut<'a>(&'a mut self, out: &mut Vec<&'a mut Pane>) {
        match self {
            PaneNode::Leaf(pane) => out.push(&mut **pane),
            PaneNode::Split { first, second, .. } => {
                first.collect_mut(out);
                second.collect_mut(out);
            }
        }
    }

    pub fn find(&self, id: PaneId) -> Option<&Pane> {
        self.panes().into_iter().find(|p| p.id == id)
    }

    pub fn find_mut(&mut self, id: PaneId) -> Option<&mut Pane> {
        self.panes_mut().into_iter().find(|p| p.id == id)
    }

    /// Replace this leaf with a split holding it (first = left/top)
    /// and `new_pane` (second = right/bottom). Caller guarantees `self`
    /// is a leaf (see `split_at`).
    fn split_this_leaf(&mut self, axis: SplitAxis, new_pane: Pane) {
        debug_assert!(matches!(self, PaneNode::Leaf(_)));
        let old = std::mem::replace(self, PaneNode::Leaf(Box::new(dummy_pane())));
        *self = PaneNode::Split {
            axis,
            ratio: 0.5,
            first: Box::new(old),
            second: Box::new(PaneNode::Leaf(Box::new(new_pane))),
        };
    }

    pub fn contains(&self, id: PaneId) -> bool {
        self.find(id).is_some()
    }

    /// Remove leaf `id`; its sibling takes the parent's place. Returns
    /// the removed pane, or None if `id` is the root leaf (a tab can't
    /// be emptied this way) or not present.
    pub fn remove_leaf(&mut self, id: PaneId) -> Option<Pane> {
        match self {
            PaneNode::Leaf(_) => None,
            PaneNode::Split { first, second, .. } => {
                let take_first = matches!(&**first, PaneNode::Leaf(p) if p.id == id);
                let take_second = matches!(&**second, PaneNode::Leaf(p) if p.id == id);
                if take_first || take_second {
                    let (removed, keep) = if take_first {
                        (
                            std::mem::replace(
                                first,
                                Box::new(PaneNode::Leaf(Box::new(dummy_pane()))),
                            ),
                            std::mem::replace(
                                second,
                                Box::new(PaneNode::Leaf(Box::new(dummy_pane()))),
                            ),
                        )
                    } else {
                        (
                            std::mem::replace(
                                second,
                                Box::new(PaneNode::Leaf(Box::new(dummy_pane()))),
                            ),
                            std::mem::replace(
                                first,
                                Box::new(PaneNode::Leaf(Box::new(dummy_pane()))),
                            ),
                        )
                    };
                    *self = *keep;
                    return match *removed {
                        PaneNode::Leaf(pane) => Some(*pane),
                        PaneNode::Split { .. } => unreachable!(),
                    };
                }
                first.remove_leaf(id).or_else(|| second.remove_leaf(id))
            }
        }
    }

    /// Ids of all panes, in layout order (left→right, top→bottom
    /// within splits).
    pub fn ids(&self) -> Vec<PaneId> {
        self.panes().iter().map(|p| p.id).collect()
    }
}

/// Split `r` along `axis` at `ratio`, leaving `SPLIT_GUTTER` between.
fn split_rect(r: Rect, axis: SplitAxis, ratio: f32) -> (Rect, Rect) {
    let g = SPLIT_GUTTER;
    match axis {
        SplitAxis::Horizontal => {
            let w = ((r.width - g) * ratio).max(0.0);
            (
                Rect::new(r.x, r.y, w, r.height),
                Rect::new(r.x + w + g, r.y, (r.width - w - g).max(0.0), r.height),
            )
        }
        SplitAxis::Vertical => {
            let h = ((r.height - g) * ratio).max(0.0);
            (
                Rect::new(r.x, r.y, r.width, h),
                Rect::new(r.x, r.y + h + g, r.width, (r.height - h - g).max(0.0)),
            )
        }
    }
}

/// A placeholder leaf used only mid-mutation; never observable.
fn dummy_pane() -> Pane {
    struct NoPty;
    impl Pty for NoPty {
        fn write(&mut self, _: &[u8]) -> Result<()> {
            Ok(())
        }
        fn read_events(&self) -> Vec<flux_terminal::pty::PtyEvent> {
            Vec::new()
        }
        fn resize(&mut self, _: u16, _: u16) -> Result<()> {
            Ok(())
        }
    }
    Pane {
        id: PaneId::MAX,
        domain_id: 0,
        pty: Box::new(NoPty),
        terminal: TerminalState::new(1, 1, 0, flux_types::ResolvedTheme::default()),
        viewport: Rect::new(0.0, 0.0, 0.0, 0.0),
    }
}

/// A workspace: a pane tree plus which pane has focus.
pub struct Tab {
    #[allow(dead_code)] // stable identity once tabs are reorderable
    pub id: u64,
    pub root: PaneNode,
    pub focus: PaneId,
    /// Last title the shell set via OSC 0/2 — shown in the tab bar and
    /// applied to the window when the tab is focused.
    pub title: Option<String>,
}

impl Tab {
    pub fn focused_pane(&self) -> Option<&Pane> {
        self.root.find(self.focus)
    }

    pub fn focused_pane_mut(&mut self) -> Option<&mut Pane> {
        self.root.find_mut(self.focus)
    }

    /// Move focus to the pane whose viewport is nearest in `direction`
    /// from the focused pane's center. Returns true if focus moved.
    pub fn focus_direction(&mut self, dx: i32, dy: i32) -> bool {
        let Some(cur) = self.focused_pane() else {
            return false;
        };
        let (cx, cy) = center(cur.viewport);
        let mut best: Option<(f32, PaneId)> = None;
        for pane in self.root.panes() {
            if pane.id == self.focus {
                continue;
            }
            let (px, py) = center(pane.viewport);
            let (ox, oy) = (px - cx, py - cy);
            // Must lie in the requested direction.
            let along = ox * dx as f32 + oy * dy as f32;
            if along <= 0.0 {
                continue;
            }
            let across = if dx != 0 { oy.abs() } else { ox.abs() };
            let score = along + across * 2.0;
            if best.is_none_or(|(s, _)| score < s) {
                best = Some((score, pane.id));
            }
        }
        if let Some((_, id)) = best {
            self.focus = id;
            true
        } else {
            false
        }
    }
}

fn center(r: Rect) -> (f32, f32) {
    (r.x + r.width * 0.5, r.y + r.height * 0.5)
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

    fn spawn_pane(
        &mut self,
        domain_id: DomainId,
        cols: u16,
        rows: u16,
        wake: WakeCallback,
        terminal: TerminalState,
    ) -> Result<Pane> {
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
            viewport: Rect::new(0.0, 0.0, 0.0, 0.0),
        };
        self.next_pane_id += 1;
        Ok(pane)
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
        let pane = self.spawn_pane(domain_id, cols, rows, wake, terminal)?;
        let tab = Tab {
            id: self.next_tab_id,
            focus: pane.id,
            root: PaneNode::Leaf(Box::new(pane)),
            title: None,
        };
        self.next_tab_id += 1;
        self.tabs.push(tab);
        self.current_tab = self.tabs.len() - 1;
        Ok(self.tabs.last_mut().expect("just pushed"))
    }

    /// Split the focused pane of the current tab along `axis`; the new
    /// shell goes right/below and takes focus. Returns the new pane id.
    pub fn split_focused(
        &mut self,
        axis: SplitAxis,
        domain_id: DomainId,
        cols: u16,
        rows: u16,
        wake: WakeCallback,
        terminal: TerminalState,
    ) -> Result<PaneId> {
        let target = self
            .focused_tab()
            .map(|t| t.focus)
            .context("no focused pane to split")?;
        let pane = self.spawn_pane(domain_id, cols, rows, wake, terminal)?;
        let new_id = pane.id;
        let tab = self.tabs.get_mut(self.current_tab).context("no tab")?;
        split_at(&mut tab.root, target, axis, pane);
        tab.focus = new_id;
        Ok(new_id)
    }

    /// Close the focused pane of the current tab. Returns true when the
    /// tab is now empty (caller closes the tab instead).
    pub fn close_focused_pane(&mut self) -> bool {
        let Some(tab) = self.tabs.get_mut(self.current_tab) else {
            return false;
        };
        let focus = tab.focus;
        if matches!(tab.root, PaneNode::Leaf(_)) {
            return true;
        }
        // Focus a neighbor first: the pane after (or before) in layout order.
        let ids = tab.root.ids();
        let idx = ids.iter().position(|&i| i == focus).unwrap_or(0);
        let next = ids
            .get(idx + 1)
            .or_else(|| idx.checked_sub(1).and_then(|i| ids.get(i)))
            .copied();
        tab.root.remove_leaf(focus);
        if let Some(next) = next {
            tab.focus = next;
        } else if let Some(first) = tab.root.ids().first() {
            tab.focus = *first;
        }
        false
    }

    /// Remove the pane with `pane_id` from whichever tab holds it (its
    /// shell exited). Returns `Some(tab_index)` when that removal
    /// emptied the tab (caller closes it), `None` otherwise.
    pub fn remove_pane_anywhere(&mut self, pane_id: PaneId) -> Option<usize> {
        let tab_idx = self.tabs.iter().position(|t| t.root.contains(pane_id))?;
        let tab = &mut self.tabs[tab_idx];
        if matches!(&tab.root, PaneNode::Leaf(p) if p.id == pane_id) {
            return Some(tab_idx);
        }
        let ids = tab.root.ids();
        let idx = ids.iter().position(|&i| i == pane_id).unwrap_or(0);
        let neighbor = ids
            .get(idx + 1)
            .or_else(|| idx.checked_sub(1).and_then(|i| ids.get(i)))
            .copied();
        tab.root.remove_leaf(pane_id);
        if tab.focus == pane_id {
            tab.focus = neighbor
                .or_else(|| tab.root.ids().first().copied())
                .unwrap_or(pane_id);
        }
        None
    }

    pub fn focused_pane(&self) -> Option<&Pane> {
        self.tabs
            .get(self.current_tab)
            .and_then(|tab| tab.focused_pane())
    }

    pub fn focused_pane_mut(&mut self) -> Option<&mut Pane> {
        self.tabs
            .get_mut(self.current_tab)
            .and_then(|tab| tab.focused_pane_mut())
    }

    pub fn focused_tab(&self) -> Option<&Tab> {
        self.tabs.get(self.current_tab)
    }

    pub fn focused_tab_mut(&mut self) -> Option<&mut Tab> {
        self.tabs.get_mut(self.current_tab)
    }

    /// Every pane in every tab, mutable — for draining PTYs.
    pub fn all_panes_mut(&mut self) -> Vec<(usize, &mut Pane)> {
        let mut out = Vec::new();
        for (idx, tab) in self.tabs.iter_mut().enumerate() {
            for pane in tab.root.panes_mut() {
                out.push((idx, pane));
            }
        }
        out
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

/// Split the leaf `target` inside `node`, directing the new pane down
/// the child that actually holds the target (so the move-only pane is
/// never lost).
fn split_at(node: &mut PaneNode, target: PaneId, axis: SplitAxis, new_pane: Pane) {
    match node {
        PaneNode::Leaf(pane) => {
            debug_assert_eq!(pane.id, target);
            node.split_this_leaf(axis, new_pane);
        }
        PaneNode::Split { first, second, .. } => {
            if first.contains(target) {
                split_at(first, target, axis, new_pane);
            } else {
                split_at(second, target, axis, new_pane);
            }
        }
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

    fn wake() -> WakeCallback {
        Box::new(|| {})
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

        mux.create_tab(7, 80, 24, wake(), term()).unwrap();
        assert_eq!(spawned.load(Ordering::SeqCst), 1);
        assert_eq!(mux.tabs.len(), 1);
        assert_eq!(mux.current_tab, 0);

        let pane = mux.focused_pane_mut().unwrap();
        pane.pty.write(b"ls\r").unwrap();
        pane.terminal.process_bytes(b"output\r\n");

        // A second tab takes focus; ids stay distinct.
        mux.create_tab(7, 80, 24, wake(), term()).unwrap();
        assert_eq!(mux.current_tab, 1);
        assert_ne!(mux.tabs[0].root.ids(), mux.tabs[1].root.ids());
        assert_ne!(mux.tabs[0].id, mux.tabs[1].id);
    }

    fn mux_with_tabs(n: usize) -> MuxState {
        let mut mux = MuxState::new();
        mux.add_domain(Box::new(FakeDomain {
            id: 0,
            spawned: Arc::new(AtomicU16::new(0)),
        }));
        for _ in 0..n {
            mux.create_tab(0, 80, 24, wake(), term()).unwrap();
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
        assert!(!mux.close_tab(0));
        assert_eq!(mux.current_tab, 0);
        assert_eq!(mux.tabs.len(), 2);
        mux.select_tab(1);
        assert!(!mux.close_tab(1));
        assert_eq!(mux.current_tab, 0);
        assert!(mux.close_tab(0));
        assert!(mux.focused_pane().is_none());
    }

    #[test]
    fn create_tab_with_unknown_domain_errors() {
        let mut mux = MuxState::new();
        let result = mux.create_tab(99, 80, 24, wake(), term());
        let err = result.err().expect("unknown domain must error");
        assert!(err.to_string().contains("no domain"), "{err}");
    }

    // ---- splits ----

    #[test]
    fn split_lays_out_side_by_side_and_focuses_the_new_pane() {
        let mut mux = mux_with_tabs(1);
        let first = mux.focused_pane().unwrap().id;
        let new = mux
            .split_focused(SplitAxis::Horizontal, 0, 40, 24, wake(), term())
            .unwrap();
        assert_ne!(new, first);
        assert_eq!(mux.focused_pane().unwrap().id, new, "new pane takes focus");

        let tab = mux.focused_tab_mut().unwrap();
        tab.root.layout(Rect::new(0.0, 0.0, 206.0, 100.0));
        let a = tab.root.find(first).unwrap().viewport;
        let b = tab.root.find(new).unwrap().viewport;
        assert_eq!(a.x, 0.0);
        assert_eq!(a.width, 100.0);
        assert_eq!(b.x, 106.0, "gutter of {SPLIT_GUTTER} between");
        assert_eq!(b.width, 100.0);
        assert_eq!(a.height, 100.0);
        let mut dividers = Vec::new();
        tab.root
            .dividers(Rect::new(0.0, 0.0, 206.0, 100.0), &mut dividers);
        assert_eq!(dividers.len(), 1);
        assert_eq!(dividers[0].x, 100.0);
        assert_eq!(dividers[0].width, SPLIT_GUTTER);
    }

    #[test]
    fn nested_split_and_directional_focus() {
        let mut mux = mux_with_tabs(1);
        let a = mux.focused_pane().unwrap().id;
        let b = mux
            .split_focused(SplitAxis::Horizontal, 0, 40, 24, wake(), term())
            .unwrap();
        // Split the right pane vertically: c below b.
        let c = mux
            .split_focused(SplitAxis::Vertical, 0, 40, 12, wake(), term())
            .unwrap();
        let tab = mux.focused_tab_mut().unwrap();
        tab.root.layout(Rect::new(0.0, 0.0, 206.0, 106.0));
        assert_eq!(tab.root.ids(), vec![a, b, c]);
        let vb = tab.root.find(b).unwrap().viewport;
        let vc = tab.root.find(c).unwrap().viewport;
        assert_eq!(vb.y, 0.0);
        assert_eq!(vc.y, 56.0);
        assert_eq!(vb.height, 50.0);

        // From c (bottom-right): up → b, left → a.
        assert_eq!(tab.focus, c);
        assert!(tab.focus_direction(0, -1));
        assert_eq!(tab.focus, b);
        assert!(tab.focus_direction(-1, 0));
        assert_eq!(tab.focus, a);
        assert!(!tab.focus_direction(-1, 0), "nothing left of a");
        assert!(tab.focus_direction(1, 0));
        assert!(tab.focus == b || tab.focus == c);
    }

    #[test]
    fn close_pane_collapses_the_split_and_refocuses() {
        let mut mux = mux_with_tabs(1);
        let a = mux.focused_pane().unwrap().id;
        let b = mux
            .split_focused(SplitAxis::Horizontal, 0, 40, 24, wake(), term())
            .unwrap();
        assert_eq!(mux.focused_pane().unwrap().id, b);
        assert!(!mux.close_focused_pane(), "tab still has a pane");
        assert_eq!(mux.focused_pane().unwrap().id, a);
        assert!(matches!(mux.focused_tab().unwrap().root, PaneNode::Leaf(_)));
        assert!(mux.close_focused_pane(), "last pane → tab is empty");
    }

    #[test]
    fn exited_pane_is_removed_from_its_tab() {
        let mut mux = mux_with_tabs(2);
        // Split tab 1 (current), then go to tab 0.
        let b = mux
            .split_focused(SplitAxis::Vertical, 0, 80, 12, wake(), term())
            .unwrap();
        mux.select_tab(0);
        assert_eq!(mux.remove_pane_anywhere(b), None, "tab 1 still has a pane");
        assert!(matches!(mux.tabs[1].root, PaneNode::Leaf(_)));
        let lone = mux.tabs[0].root.ids()[0];
        assert_eq!(
            mux.remove_pane_anywhere(lone),
            Some(0),
            "root leaf → tab empty"
        );
    }
}
