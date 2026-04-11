//! Configuration loading.
//!
//! On first run, generates a default config from resources/default-config.toml.
//! All defaults come from that file — nothing is duplicated in code.
//! If the config is corrupted or missing fields, back it up and reset.

use anyhow::Result;
use serde::Deserialize;

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

        if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            match toml::from_str::<FluxConfig>(&content) {
                Ok(config) => {
                    log::info!("Loaded config from {}", path.display());
                    return Ok(config);
                }
                Err(e) => {
                    // Backup the broken config
                    let backup_path = path.with_extension("toml.bak");
                    if let Err(backup_err) = std::fs::copy(&path, &backup_path) {
                        log::error!("Failed to backup config: {}", backup_err);
                    }

                    eprintln!();
                    eprintln!("  ⚡ Warning: Config file has errors.");
                    eprintln!("     {}", path.display());
                    eprintln!("     Error: {}", e);
                    eprintln!("     Backed up to: {}", backup_path.display());
                    eprintln!("     Reset to defaults. Your old config is in the backup.");
                    eprintln!();
                }
            }
        }

        // Either no config existed or it was broken — write and parse the default
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, DEFAULT_CONFIG_TOML)?;
        log::info!("Generated default config at {}", path.display());

        let config: FluxConfig = toml::from_str(DEFAULT_CONFIG_TOML)
            .expect("built-in default config must be valid TOML");
        Ok(config)
    }
}
