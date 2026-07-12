//! Flux — a modern, GPU-accelerated terminal with command blocks.
//!
//! "Where we're going, we don't need Electron."

mod app;
mod config;
mod platform;

use anyhow::Result;
use app::App;
use flux_input::{CommandHistory, InputEditor};
use winit::event_loop::EventLoop;

fn main() -> Result<()> {
    env_logger::init();
    platform::ensure_layout();

    let config = config::FluxConfig::load()?;
    log::info!(
        "Config: {} {}pt {}",
        config.font.family,
        config.font.size,
        config.font.weight
    );

    let shell = flux_shell::detect_shell();
    log::info!("Shell: {} ({})", shell.name(), shell.binary().display());

    let history_path = platform::history_file();
    let shell_history = shell.load_history();
    log::info!("Loaded {} entries from shell history", shell_history.len());
    let history = CommandHistory::load(history_path, 10_000, shell_history);
    let input = InputEditor::with_history(history);

    println!("Flux v0.1.0 — 1.21 gigawatts");

    let event_loop = EventLoop::new()?;
    let proxy = event_loop.create_proxy();
    let mut app = App::new(config, proxy, input);
    event_loop.run_app(&mut app)?;

    Ok(())
}
