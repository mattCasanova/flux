//! Popup / overlay state — the single source of truth for "is there
//! an overlay UI eating keystrokes right now?" One popup at a time;
//! features add variants to this enum as they land.

pub enum PopupState {
    /// No overlay. Keystrokes flow to the normal raw/cooked handlers.
    Hidden,
    /// Autocomplete popup is visible. Data lives in `App.autocomplete`.
    Autocomplete,
}
