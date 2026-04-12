//! Configuration loading.
//!
//! On first run, generates a default config from resources/default-config.toml.
//! All defaults come from that file — nothing is duplicated in code.
//! If the config is corrupted or missing fields, back it up and reset.

use anyhow::Result;
use serde::Deserialize;
use std::path::Path;

use crate::platform;

/// The default config file — compiled in from resources/default-config.toml.
/// This is the single source of truth for all default values.
const DEFAULT_CONFIG_TOML: &str = include_str!("../../../resources/default-config.toml");

#[derive(Debug, Deserialize, Clone)]
pub struct FluxConfig {
    pub font: FontConfig,
    pub window: WindowConfig,
    pub theme: ThemeConfig,
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
