//! Keybinding manager — maps key combos to actions.

/// Actions that keybindings can trigger.
pub enum Action {
    NewTab,
    CloseTab,
    NextTab,
    PrevTab,
    GoToTab(usize),
    SplitHorizontal,
    SplitVertical,
    ToggleBlockCollapse,
    Search,
    CommandPalette,
    Copy,
    Paste,
    ScrollUp,
    ScrollDown,
    ScrollPageUp,
    ScrollPageDown,
}

/// Manages keybinding configuration and lookup.
pub struct KeybindingManager {
    // TODO: Phase 3
    // - HashMap<KeyCombo, Action>
    // - Load from config.toml [keybindings] section
    // - Platform-aware: Cmd on macOS, Ctrl on Linux
}

impl KeybindingManager {
    pub fn new() -> Self {
        Self {}
    }
}
