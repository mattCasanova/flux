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
    /// Background tint for a completed command's header rows (prompt +
    /// echo) in the semantic stream. Derived from `background` unless
    /// the config sets it.
    pub block_header: Color,
    /// Header tint for a command that exited nonzero. Derived from
    /// `block_header` and the red ANSI slot.
    pub block_failed: Color,
}

impl ResolvedTheme {
    /// ANSI palette lookup for indices 0–15.
    pub fn ansi(&self, idx: usize) -> Color {
        self.ansi[idx & 0xf]
    }

    /// The default header tint for a background: nudged a step toward
    /// the foreground so it reads as "a shade lighter" on dark themes
    /// and "a shade darker" on light ones.
    pub fn derive_block_header(background: Color, foreground: Color) -> Color {
        blend(background, foreground, 0.08)
    }

    /// The default failed-header tint: the header tint with a little of
    /// the theme's red in it.
    pub fn derive_block_failed(block_header: Color, red: Color) -> Color {
        blend(block_header, red, 0.22)
    }

    /// Recompute the derived block colors from the current base colors.
    /// Config resolution calls this after applying overrides to
    /// `background` / `foreground` / `red`, then applies any explicit
    /// `block_header` on top.
    pub fn rederive_block_colors(&mut self) {
        self.block_header = Self::derive_block_header(self.background, self.foreground);
        self.block_failed = Self::derive_block_failed(self.block_header, self.ansi(1));
    }
}

/// Opaque source-over blend: `base` with `amount` of `tint` on top.
fn blend(base: Color, tint: Color, amount: f32) -> Color {
    Color::new(
        base.r + (tint.r - base.r) * amount,
        base.g + (tint.g - base.g) * amount,
        base.b + (tint.b - base.b) * amount,
        1.0,
    )
}

impl Default for ResolvedTheme {
    fn default() -> Self {
        let hex = |s: &str| Color::from_hex(s).expect("built-in palette hex is valid");
        let mut theme = Self {
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
                hex("#6a7799"), // bright black
                hex("#ff99a8"), // bright red
                hex("#b8e986"), // bright green
                hex("#f4cc70"), // bright yellow
                hex("#9cc1ff"), // bright blue
                hex("#d6b3ff"), // bright magenta
                hex("#a3e6ff"), // bright cyan
                hex("#e0e6ff"), // bright white
            ],
            foreground: hex("#c0caf5"),
            background: hex("#24283b"),
            cursor: hex("#c0caf5"),
            block_header: Color::default(),
            block_failed: Color::default(),
        };
        theme.rederive_block_colors();
        theme
    }
}
