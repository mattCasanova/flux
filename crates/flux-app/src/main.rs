//! Flux — a modern, GPU-accelerated terminal with command blocks.
//!
//! "Where we're going, we don't need Electron."

mod app;
mod config;
mod keys;
mod logging;
mod mux;
mod platform;

use anyhow::Result;
use app::App;
use flux_input::CommandHistory;
use winit::event_loop::EventLoop;

fn main() -> Result<()> {
    // `--version` / `-V` prints and exits before ANY init — installers
    // and scripts probe it; without this the probe launched a window
    // (dogfood 08-23, found by the install.sh end-to-end test).
    if std::env::args().any(|a| a == "--version" || a == "-V") {
        println!("{}", version_line());
        return Ok(());
    }

    // Logging + panic hook first, so every subsequent failure leaves a
    // trail in ~/.local/state/flux/.
    logging::init()?;
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

    println!("{}", version_line());

    let event_loop = EventLoop::new()?;
    let proxy = event_loop.create_proxy();
    let mut app = App::new(config, proxy, history);
    event_loop.run_app(&mut app)?;

    Ok(())
}

/// The banner: version + build SHA, self-identifying binaries.
fn version_line() -> String {
    format!(
        "Flux v{} ({}) — 1.21 gigawatts",
        env!("CARGO_PKG_VERSION"),
        option_env!("FLUX_GIT_SHA").unwrap_or("no-git")
    )
}
