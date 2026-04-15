//! Popup / overlay state — the single source of truth for "is there
//! an overlay UI eating keystrokes right now?" One popup at a time;
//! features add variants to this enum as they land.
//!
//! **All variants are unit variants.** The per-popup data (the
//! `Autocomplete` struct, the `SearchState` struct, etc.) lives in
//! separate fields on `App`, not inside the enum. This means:
//!
//! 1. `match self.popup` can iterate all variants in ~3 lines without
//!    nested destructuring.
//! 2. The popup intercept in `handle_keyboard` uses an exhaustive
//!    `match` over `&self.popup` with no wildcard arm, so adding a
//!    variant is a compile error until its intercept is wired up.
//! 3. Borrow-checker lifetimes are cleaner — popup-state fields on
//!    `App` can be borrowed independently of the enum tag.
//!
//! # Evolution across v0.2
//!
//! - **R6** (this file) — introduces the enum with `Hidden` only.
//!   Vacuous match in `handle_keyboard`.
//! - **F7** — adds `Autocomplete`; `App` gains an `autocomplete:
//!   Autocomplete` field and an intercept arm.
//! - **F14** — adds `Search`; `App` gains `search: Option<SearchState>`
//!   and a second intercept arm.
//! - **v0.5+** — `CommandPalette`, `ConnectionPicker`, etc.
//!
//! **Any feature that adds a variant must update this enum in place**
//! rather than redeclaring it. Later features assume earlier variants
//! still exist.

pub enum PopupState {
    /// No overlay. Keystrokes flow to the normal raw/cooked handlers.
    Hidden,
    // Added in F7: Autocomplete — state lives in App.autocomplete
    // Added in F14: Search      — state lives in App.search
}
