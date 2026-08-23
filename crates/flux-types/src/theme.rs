//! Resolved terminal color theme — plain data, no TOML/config logic.
//!
//! This is the *resolved* form: every color already parsed and
//! validated. Config parsing (and later, the F16 theme system with
//! named theme files) constructs one of these and hands it to
//! `flux-terminal`. `Default` is Tokyo Night Storm — the palette Flux
//! shipped with from day one.

use crate::Color;

/// Every chrome color the renderer paints — resolved, with one field
/// per knob so a config key (or, later, a theme-editing UI) can
/// override each individually. Defaults derive from the base palette
/// where a relationship is natural (accent = blue, dim = bright
/// black) and are fixed values matching the shipped look otherwise.
/// Alpha matters on the blend tints (selection, search) and the
/// scrollbar.
#[derive(Debug, Clone, PartialEq)]
pub struct UiColors {
    /// The one accent: prompt ❯, focused tab text, pane accent,
    /// sticky-header marker.
    pub accent: Color,
    /// Thin rules: input-bar divider, tab separators, split dividers.
    pub divider: Color,
    /// Shell cursor block (raw/passthrough modes) and the input bar's
    /// block cursor.
    pub cursor: Color,
    /// Glyph color drawn INSIDE the cursor block.
    pub cursor_text: Color,
    /// Selection tint blended over cell backgrounds (alpha applies).
    pub selection: Color,
    /// Whole-block highlight when a block is selected (click /
    /// Cmd+Up/Down). Alpha applies.
    pub block_selected: Color,
    /// Search match tint (alpha applies).
    pub search_match: Color,
    /// Focused search match tint (alpha applies).
    pub search_focus: Color,
    pub scrollbar_thumb: Color,
    pub scrollbar_track: Color,
    /// Sidebar panel background (the `tab_*` names predate the
    /// sidebar; they style it now).
    pub tab_bg: Color,
    pub tab_text: Color,
    pub tab_focused_bg: Color,
    pub tab_focused_text: Color,
    /// Running-command indicator dot in the sidebar.
    pub sidebar_running: Color,
    /// Custom titlebar strip (macOS unified chrome). Defaults to the
    /// main terminal background so the window reads as one surface.
    pub titlebar_bg: Color,
    /// Input bar text (focused pane).
    pub input_text: Color,
    /// Input bar text on unfocused panes; also secondary UI text.
    pub input_dim: Color,
    pub popup_bg: Color,
    pub popup_selected_bg: Color,
    pub popup_directory: Color,
    pub popup_file: Color,
    pub popup_symlink: Color,
    pub notice_bg: Color,
    pub notice_text: Color,
    pub sticky_bg: Color,
    pub sticky_failed_bg: Color,
    pub sticky_text: Color,
    pub sticky_failed_text: Color,
    pub pane_accent: Color,
}

impl UiColors {
    /// Derive the full chrome palette from the base theme. Called
    /// after base-color overrides so a custom background/blue flows
    /// into the chrome automatically; individual `[theme.ui]` keys
    /// then override single slots.
    pub fn derive(ansi: &[Color; 16], foreground: Color, background: Color, cursor: Color) -> Self {
        let hex = |s: &str| Color::from_hex(s).expect("built-in ui hex is valid");
        let accent = ansi[4];
        Self {
            accent,
            divider: hex("#4d5473"),
            cursor,
            cursor_text: background,
            selection: with_alpha(accent, 0.30),
            block_selected: with_alpha(accent, 0.20),
            search_match: with_alpha(ansi[3], 0.35),
            search_focus: with_alpha(hex("#ff9e64"), 0.65),
            scrollbar_thumb: Color::new(0.60, 0.65, 0.80, 0.55),
            scrollbar_track: Color::new(0.60, 0.65, 0.80, 0.10),
            tab_bg: Color::new(0.10, 0.11, 0.17, 1.0),
            tab_text: Color::new(0.55, 0.60, 0.75, 1.0),
            tab_focused_bg: Color::new(0.16, 0.18, 0.28, 1.0),
            tab_focused_text: accent,
            sidebar_running: ansi[2],
            titlebar_bg: background,
            input_text: foreground,
            input_dim: ansi[8],
            popup_bg: hex("#1f2335"),
            popup_selected_bg: hex("#3b4261"),
            popup_directory: accent,
            popup_file: foreground,
            popup_symlink: ansi[5],
            notice_bg: hex("#3b2f2f"),
            notice_text: ansi[1],
            sticky_bg: Color::new(0.14, 0.16, 0.26, 0.96),
            sticky_failed_bg: Color::new(0.30, 0.16, 0.20, 0.96),
            sticky_text: Color::new(0.75, 0.79, 0.96, 1.0),
            sticky_failed_text: Color::new(0.97, 0.46, 0.56, 1.0),
            pane_accent: with_alpha(accent, 0.9),
        }
    }

    /// Override one slot by its config key name. Unknown names return
    /// false so the caller can warn.
    pub fn set_by_name(&mut self, name: &str, color: Color) -> bool {
        let slot = match name {
            "accent" => &mut self.accent,
            "divider" => &mut self.divider,
            "cursor" => &mut self.cursor,
            "cursor_text" => &mut self.cursor_text,
            "selection" => &mut self.selection,
            "block_selected" => &mut self.block_selected,
            "search_match" => &mut self.search_match,
            "search_focus" => &mut self.search_focus,
            "scrollbar_thumb" => &mut self.scrollbar_thumb,
            "scrollbar_track" => &mut self.scrollbar_track,
            "tab_bg" => &mut self.tab_bg,
            "tab_text" => &mut self.tab_text,
            "tab_focused_bg" => &mut self.tab_focused_bg,
            "tab_focused_text" => &mut self.tab_focused_text,
            "sidebar_running" => &mut self.sidebar_running,
            "titlebar_bg" => &mut self.titlebar_bg,
            "input_text" => &mut self.input_text,
            "input_dim" => &mut self.input_dim,
            "popup_bg" => &mut self.popup_bg,
            "popup_selected_bg" => &mut self.popup_selected_bg,
            "popup_directory" => &mut self.popup_directory,
            "popup_file" => &mut self.popup_file,
            "popup_symlink" => &mut self.popup_symlink,
            "notice_bg" => &mut self.notice_bg,
            "notice_text" => &mut self.notice_text,
            "sticky_bg" => &mut self.sticky_bg,
            "sticky_failed_bg" => &mut self.sticky_failed_bg,
            "sticky_text" => &mut self.sticky_text,
            "sticky_failed_text" => &mut self.sticky_failed_text,
            "pane_accent" => &mut self.pane_accent,
            _ => return false,
        };
        *slot = color;
        true
    }
}

fn with_alpha(color: Color, alpha: f32) -> Color {
    Color::new(color.r, color.g, color.b, alpha)
}

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
    /// Every chrome color (input bar, tabs, popups, …). Derived from
    /// the base palette; each slot overridable via `[theme.ui]`.
    pub ui: UiColors,
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

    /// Recompute the chrome palette from the current base colors.
    pub fn rederive_ui(&mut self) {
        self.ui = UiColors::derive(&self.ansi, self.foreground, self.background, self.cursor);
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
            ui: UiColors::derive(
                &[Color::default(); 16],
                Color::default(),
                Color::default(),
                Color::default(),
            ),
        };
        theme.rederive_block_colors();
        theme.rederive_ui();
        theme
    }
}
