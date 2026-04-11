//! Configuration loading.
//!
//! On first run, generates a default config from resources/default-config.toml.
//! All defaults come from that file — nothing is duplicated in code.
//! Missing fields fall back to defaults. Corrupted files fall back entirely.

use anyhow::Result;
use serde::Deserialize;
use std::sync::LazyLock;

use crate::platform;

/// The default config file — compiled in from resources/default-config.toml.
/// This is the single source of truth for all default values.
const DEFAULT_CONFIG_TOML: &str = include_str!("../../../resources/default-config.toml");

/// Parsed defaults — computed once, used for serde fallbacks.
static DEFAULTS: LazyLock<FluxConfig> = LazyLock::new(|| {
    toml::from_str(DEFAULT_CONFIG_TOML).expect("built-in default config must be valid TOML")
});

#[derive(Debug, Deserialize, Clone)]
pub struct FluxConfig {
    #[serde(default = "default_font")]
    pub font: FontConfig,
    #[serde(default = "default_window")]
    pub window: WindowConfig,
    #[serde(default = "default_theme")]
    pub theme: ThemeConfig,
}

#[derive(Debug, Deserialize, Clone)]
pub struct FontConfig {
    #[serde(default = "default_font_family")]
    pub family: String,
    #[serde(default = "default_font_size")]
    pub size: f32,
    #[serde(default = "default_font_weight")]
    pub weight: String,
    #[serde(default = "default_font_style")]
    pub style: String,
    #[serde(default = "default_line_height")]
    pub line_height: f32,
}

#[derive(Debug, Deserialize, Clone)]
pub struct WindowConfig {
    #[serde(default = "default_window_title")]
    pub title: String,
    #[serde(default = "default_window_width")]
    pub width: u32,
    #[serde(default = "default_window_height")]
    pub height: u32,
}

#[derive(Debug, Deserialize, Clone)]
pub struct ThemeConfig {
    #[serde(default = "default_background")]
    pub background: String,
    #[serde(default = "default_foreground")]
    pub foreground: String,
}

// All defaults pull from the parsed resource file — not hardcoded.
fn default_font() -> FontConfig { DEFAULTS.font.clone() }
fn default_window() -> WindowConfig { DEFAULTS.window.clone() }
fn default_theme() -> ThemeConfig { DEFAULTS.theme.clone() }
fn default_font_family() -> String { DEFAULTS.font.family.clone() }
fn default_font_size() -> f32 { DEFAULTS.font.size }
fn default_font_weight() -> String { DEFAULTS.font.weight.clone() }
fn default_font_style() -> String { DEFAULTS.font.style.clone() }
fn default_line_height() -> f32 { DEFAULTS.font.line_height }
fn default_window_title() -> String { DEFAULTS.window.title.clone() }
fn default_window_width() -> u32 { DEFAULTS.window.width }
fn default_window_height() -> u32 { DEFAULTS.window.height }
fn default_background() -> String { DEFAULTS.theme.background.clone() }
fn default_foreground() -> String { DEFAULTS.theme.foreground.clone() }

impl FluxConfig {
    /// Load config from the platform config directory.
    /// If no config exists, generate the default one on first run.
    /// If the config is corrupted or missing fields, fall back gracefully.
    pub fn load() -> Result<Self> {
        let path = platform::config_dir().join("config.toml");

        if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            match toml::from_str::<FluxConfig>(&content) {
                Ok(config) => {
                    log::info!("Loaded config from {}", path.display());
                    Ok(config)
                }
                Err(e) => {
                    // Backup the broken config, reset to defaults
                    let backup_path = path.with_extension("toml.bak");
                    if let Err(backup_err) = std::fs::copy(&path, &backup_path) {
                        log::error!("Failed to backup config: {}", backup_err);
                    }
                    std::fs::write(&path, DEFAULT_CONFIG_TOML)?;

                    eprintln!();
                    eprintln!("  ⚡ Warning: Config file has errors.");
                    eprintln!("     {}", path.display());
                    eprintln!("     Error: {}", e);
                    eprintln!("     Backed up to: {}", backup_path.display());
                    eprintln!("     Reset to defaults. Your old config is in the backup.");
                    eprintln!();

                    Ok(DEFAULTS.clone())
                }
            }
        } else {
            // First run — generate default config file
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, DEFAULT_CONFIG_TOML)?;
            log::info!("Generated default config at {}", path.display());
            Ok(DEFAULTS.clone())
        }
    }
}
