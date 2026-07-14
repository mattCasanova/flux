//! Resolved terminal color theme — plain data, no TOML/config logic.
//!
//! This is the *resolved* form: every color already parsed and
//! validated. Config parsing (and later, the F16 theme system with
//! named theme files) constructs one of these and hands it to
//! `flux-terminal`. `Default` is Tokyo Night Storm — the palette Flux
//! shipped with from day one.

use crate::Color;

#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedTheme {
    /// ANSI colors 0–15: black, red, green, yellow, blue, magenta,
    /// cyan, white, then the bright variants in the same order.
    pub ansi: [Color; 16],
    pub foreground: Color,
    pub background: Color,
    pub cursor: Color,
}

impl ResolvedTheme {
    /// ANSI palette lookup for indices 0–15.
    pub fn ansi(&self, idx: usize) -> Color {
        self.ansi[idx & 0xf]
    }
}

impl Default for ResolvedTheme {
    fn default() -> Self {
        let hex = |s: &str| Color::from_hex(s).expect("built-in palette hex is valid");
        Self {
            // Tokyo Night Storm
            ansi: [
                hex("#414868"), // black
                hex("#f7768e"), // red
                hex("#73daca"), // green
                hex("#e0af68"), // yellow
                hex("#7aa2f7"), // blue
                hex("#bb9af7"), // magenta
                hex("#7dcfff"), // cyan
                hex("#c0caf5"), // white
                hex("#6a7099"), // bright black
                hex("#ff9999"), // bright red
                hex("#b9e986"), // bright green
                hex("#f4e070"), // bright yellow
                hex("#9cc1ff"), // bright blue
                hex("#d6b4ff"), // bright magenta
                hex("#a3e6ff"), // bright cyan
                hex("#e0e6ff"), // bright white
            ],
            foreground: hex("#c0caf5"),
            background: hex("#24283b"),
            cursor: hex("#c0caf5"),
        }
    }
}
