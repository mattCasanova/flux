//! Configuration loading from ~/.config/flux/config.toml

use anyhow::Result;
use serde::Deserialize;
use std::path::PathBuf;

#[derive(Debug, Deserialize)]
pub struct FluxConfig {
    #[serde(default)]
    pub font: FontConfig,
    #[serde(default)]
    pub window: WindowConfig,
    #[serde(default)]
    pub theme: ThemeConfig,
}

#[derive(Debug, Deserialize)]
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

#[derive(Debug, Deserialize)]
pub struct WindowConfig {
    #[serde(default = "default_window_title")]
    pub title: String,
    #[serde(default = "default_window_width")]
    pub width: u32,
    #[serde(default = "default_window_height")]
    pub height: u32,
}

#[derive(Debug, Deserialize)]
pub struct ThemeConfig {
    #[serde(default = "default_background")]
    pub background: String,
    #[serde(default = "default_foreground")]
    pub foreground: String,
}

// Defaults
fn default_font_family() -> String { "Fira Code".into() }
fn default_font_size() -> f32 { 14.0 }
fn default_font_weight() -> String { "normal".into() }
fn default_font_style() -> String { "normal".into() }
fn default_line_height() -> f32 { 1.2 }
fn default_window_title() -> String { "Flux — 1.21 gigawatts".into() }
fn default_window_width() -> u32 { 1200 }
fn default_window_height() -> u32 { 800 }
fn default_background() -> String { "#24283b".into() }
fn default_foreground() -> String { "#c0caf5".into() }

impl Default for FluxConfig {
    fn default() -> Self {
        Self {
            font: FontConfig::default(),
            window: WindowConfig::default(),
            theme: ThemeConfig::default(),
        }
    }
}

impl Default for FontConfig {
    fn default() -> Self {
        Self {
            family: default_font_family(),
            size: default_font_size(),
            weight: default_font_weight(),
            style: default_font_style(),
            line_height: default_line_height(),
        }
    }
}

impl Default for WindowConfig {
    fn default() -> Self {
        Self {
            title: default_window_title(),
            width: default_window_width(),
            height: default_window_height(),
        }
    }
}

impl Default for ThemeConfig {
    fn default() -> Self {
        Self {
            background: default_background(),
            foreground: default_foreground(),
        }
    }
}

impl FluxConfig {
    /// Load config from ~/.config/flux/config.toml, falling back to defaults.
    pub fn load() -> Result<Self> {
        let path = Self::config_path();

        if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            let config: FluxConfig = toml::from_str(&content)?;
            log::info!("Loaded config from {}", path.display());
            Ok(config)
        } else {
            log::info!("No config file found, using defaults");
            Ok(Self::default())
        }
    }

    fn config_path() -> PathBuf {
        // Prefer ~/.config/flux/ (conventional for CLI tools on all platforms)
        let dot_config = dirs::home_dir().unwrap().join(".config/flux/config.toml");
        if dot_config.exists() {
            return dot_config;
        }

        // Fall back to platform config dir (~/Library/Application Support on macOS)
        let platform = dirs::config_dir()
            .unwrap_or_else(|| dirs::home_dir().unwrap().join(".config"))
            .join("flux/config.toml");
        if platform.exists() {
            return platform;
        }

        // Default to ~/.config even if neither exists (for error message)
        dot_config
    }
}
