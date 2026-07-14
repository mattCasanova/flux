//! Configuration loading.
//!
//! On first run, generates a default config from resources/default-config.toml,
//! which spells out the complete theme so every knob is discoverable.
//! Optional keys additionally have code-side defaults (ResolvedTheme,
//! ScrollbackConfig) so configs written before those keys existed keep
//! parsing — keep the two in sync when either changes.
//! If the config is corrupted or missing required fields, back it up and reset.

use anyhow::Result;
use serde::Deserialize;
use std::path::Path;

use crate::platform;

/// The default config file — compiled in from resources/default-config.toml.
/// This is the single source of truth for all default values.
const DEFAULT_CONFIG_TOML: &str = include_str!("../../../resources/default-config.toml");

/// The config schema version the running binary understands.
///
/// Bumped any time `FluxConfig` grows a field that the old defaults
/// can't reasonably auto-fill. F2 (Phase 1) introduces actual
/// migration logic keyed off this number; R2 only lays the
/// scaffolding so the field is parsed and a default exists for
/// pre-versioned configs.
pub const CURRENT_CONFIG_VERSION: u32 = 1;

fn default_version() -> u32 {
    // Pre-R2 configs have no `version` field. Treat them as v1 —
    // they parsed fine against the v0.1 schema, so nothing has
    // diverged yet.
    CURRENT_CONFIG_VERSION
}

#[derive(Debug, Deserialize, Clone)]
pub struct FluxConfig {
    /// Config schema version. See `CURRENT_CONFIG_VERSION`.
    #[serde(default = "default_version")]
    #[allow(dead_code)] // consumed by F2 config migration (#55)
    pub version: u32,
    pub font: FontConfig,
    pub window: WindowConfig,
    pub theme: ThemeConfig,
    /// Additive section: serde-defaulted so configs written before it
    /// existed still parse (the additive-field pattern from R2; real
    /// migrations land in F2). Defaults here must match
    /// resources/default-config.toml.
    #[serde(default)]
    pub scrollback: ScrollbackConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ScrollbackConfig {
    /// Scrollback capacity in lines. 0 disables history entirely.
    #[serde(default = "default_scrollback_lines")]
    pub lines: usize,
    /// Optional "#rrggbb" padding tint shown while scrolled up into
    /// history — a quiet visual cue that you're not at the live tail.
    /// Unset = padding never changes.
    #[serde(default)]
    pub scrolled_background: Option<String>,
}

fn default_scrollback_lines() -> usize {
    10_000
}

impl Default for ScrollbackConfig {
    fn default() -> Self {
        Self {
            lines: default_scrollback_lines(),
            scrolled_background: None,
        }
    }
}

#[derive(Debug, Deserialize, Clone)]
pub struct FontConfig {
    pub family: String,
    pub size: f32,
    pub weight: String,
    pub style: String,
    pub line_height: f32,
}

#[derive(Debug, Deserialize, Clone)]
pub struct WindowConfig {
    pub title: String,
    pub width: u32,
    pub height: u32,
    pub padding_horizontal: f32,
    pub padding_vertical: f32,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ThemeConfig {
    pub background: String,
    pub foreground: String,
    /// Padding color while an alt-screen program (vim, Claude Code)
    /// runs: "sync" adopts the program's background so its colorscheme
    /// fills edge-to-edge (default); "theme" pins to `background`
    /// above; any "#rrggbb" pins to that color.
    #[serde(default)]
    pub alt_screen_background: Option<String>,
    /// Optional ANSI palette overrides ("#rrggbb"); unset keys keep the
    /// built-in Tokyo Night Storm values. The full F16 theme system
    /// (named theme files) replaces this later — this is the minimal
    /// "colors come from config" step.
    #[serde(default)]
    pub black: Option<String>,
    #[serde(default)]
    pub red: Option<String>,
    #[serde(default)]
    pub green: Option<String>,
    #[serde(default)]
    pub yellow: Option<String>,
    #[serde(default)]
    pub blue: Option<String>,
    #[serde(default)]
    pub magenta: Option<String>,
    #[serde(default)]
    pub cyan: Option<String>,
    #[serde(default)]
    pub white: Option<String>,
    #[serde(default)]
    pub bright_black: Option<String>,
    #[serde(default)]
    pub bright_red: Option<String>,
    #[serde(default)]
    pub bright_green: Option<String>,
    #[serde(default)]
    pub bright_yellow: Option<String>,
    #[serde(default)]
    pub bright_blue: Option<String>,
    #[serde(default)]
    pub bright_magenta: Option<String>,
    #[serde(default)]
    pub bright_cyan: Option<String>,
    #[serde(default)]
    pub bright_white: Option<String>,
    /// Cursor color; defaults to `foreground`.
    #[serde(default)]
    pub cursor: Option<String>,
}

impl ThemeConfig {
    /// Resolve config strings into a validated color theme. Invalid
    /// hex values warn and keep the built-in default for that slot.
    pub fn resolve(&self) -> flux_types::ResolvedTheme {
        use flux_types::Color;
        let mut theme = flux_types::ResolvedTheme::default();

        fn apply(slot: &mut Color, value: Option<&str>, key: &str) {
            let Some(hex) = value else { return };
            match Color::from_hex(hex) {
                Some(color) => *slot = color,
                None => log::warn!("invalid [theme] {} = {:?}; keeping default", key, hex),
            }
        }

        apply(&mut theme.background, Some(&self.background), "background");
        apply(&mut theme.foreground, Some(&self.foreground), "foreground");
        // Cursor follows foreground unless explicitly set.
        theme.cursor = theme.foreground;
        apply(&mut theme.cursor, self.cursor.as_deref(), "cursor");

        let ansi_keys: [(usize, &Option<String>, &str); 16] = [
            (0, &self.black, "black"),
            (1, &self.red, "red"),
            (2, &self.green, "green"),
            (3, &self.yellow, "yellow"),
            (4, &self.blue, "blue"),
            (5, &self.magenta, "magenta"),
            (6, &self.cyan, "cyan"),
            (7, &self.white, "white"),
            (8, &self.bright_black, "bright_black"),
            (9, &self.bright_red, "bright_red"),
            (10, &self.bright_green, "bright_green"),
            (11, &self.bright_yellow, "bright_yellow"),
            (12, &self.bright_blue, "bright_blue"),
            (13, &self.bright_magenta, "bright_magenta"),
            (14, &self.bright_cyan, "bright_cyan"),
            (15, &self.bright_white, "bright_white"),
        ];
        for (idx, value, key) in ansi_keys {
            apply(&mut theme.ansi[idx], value.as_deref(), key);
        }

        theme
    }
}

impl FluxConfig {
    /// Load config from the platform config directory.
    /// If no config exists, generate the default.
    /// If the config is broken, back it up and reset.
    pub fn load() -> Result<Self> {
        let path = platform::config_dir().join("config.toml");

        if !path.exists() {
            return Self::write_default(&path);
        }

        let content = std::fs::read_to_string(&path)?;
        match toml::from_str::<FluxConfig>(&content) {
            Ok(config) => {
                log::info!("Loaded config from {}", path.display());
                Ok(config)
            }
            Err(e) => {
                Self::backup_and_reset(&path, &e);
                Self::write_default(&path)
            }
        }
    }

    /// Write the default config file and return the parsed defaults.
    fn write_default(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, DEFAULT_CONFIG_TOML)?;
        log::info!("Generated default config at {}", path.display());

        let config: FluxConfig = toml::from_str(DEFAULT_CONFIG_TOML)
            .expect("built-in default config must be valid TOML");
        Ok(config)
    }

    #[cfg(test)]
    pub fn from_str_for_test(toml_str: &str) -> Result<Self> {
        Ok(toml::from_str(toml_str)?)
    }

    /// Back up a broken config file and warn the user.
    fn backup_and_reset(path: &Path, error: &toml::de::Error) {
        let backup_path = path.with_extension("toml.bak");
        if let Err(e) = std::fs::copy(path, &backup_path) {
            log::error!("Failed to backup config: {}", e);
        }

        eprintln!();
        eprintln!("  ⚡ Warning: Config file has errors.");
        eprintln!("     {}", path.display());
        eprintln!("     Error: {}", error);
        eprintln!("     Backed up to: {}", backup_path.display());
        eprintln!("     Reset to defaults. Your old config is in the backup.");
        eprintln!();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pre_theme_config_still_parses_and_resolves_defaults() {
        let config = FluxConfig::from_str_for_test(
            r##"
            [font]
            family = "Menlo"
            size = 14.0
            weight = "normal"
            style = "normal"
            line_height = 1.2
            [window]
            title = "t"
            width = 100
            height = 100
            padding_horizontal = 0
            padding_vertical = 0
            [theme]
            background = "#24283b"
            foreground = "#c0caf5"
            "##,
        )
        .expect("old-style config must parse");
        let theme = config.theme.resolve();
        // Untouched palette slots keep Tokyo Night Storm.
        assert_eq!(
            theme.ansi(1),
            flux_types::Color::from_hex("#f7768e").unwrap()
        );
        assert_eq!(theme.cursor, theme.foreground);
    }

    #[test]
    fn palette_overrides_apply_and_bad_hex_keeps_default() {
        let config = FluxConfig::from_str_for_test(
            r##"
            [font]
            family = "Menlo"
            size = 14.0
            weight = "normal"
            style = "normal"
            line_height = 1.2
            [window]
            title = "t"
            width = 100
            height = 100
            padding_horizontal = 0
            padding_vertical = 0
            [theme]
            background = "#000000"
            foreground = "#ffffff"
            red = "#ff0000"
            green = "not-a-color"
            "##,
        )
        .unwrap();
        let theme = config.theme.resolve();
        assert_eq!(
            theme.ansi(1),
            flux_types::Color::from_hex("#ff0000").unwrap()
        );
        // Invalid hex warned and kept the built-in green.
        assert_eq!(
            theme.ansi(2),
            flux_types::Color::from_hex("#73daca").unwrap()
        );
        assert_eq!(
            theme.background,
            flux_types::Color::from_hex("#000000").unwrap()
        );
    }
}
