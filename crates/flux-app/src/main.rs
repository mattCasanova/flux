//! Flux — a modern, GPU-accelerated terminal with command blocks.
//!
//! "Where we're going, we don't need Electron."

use anyhow::Result;

fn main() -> Result<()> {
    env_logger::init();

    // Detect the user's shell
    let shell = flux_shell::detect_shell();
    log::info!("Detected shell: {} ({})", shell.name(), shell.binary().display());

    // TODO: Phase 1 steps
    // 1. Load theme from default-theme.toml
    // 2. Create winit event loop + window
    // 3. Initialize flux-renderer (wgpu device, surface, atlas, pipeline)
    // 4. Spawn PTY via flux-terminal
    // 5. Create input editor via flux-input
    // 6. Run event loop:
    //    - Route keyboard input (editor vs PTY based on mode)
    //    - Process PTY output → update terminal state
    //    - Render frame when dirty (output grid + input editor)
    //    - Handle resize

    println!("Flux v0.1.0 — 1.21 gigawatts");
    println!("Great Scott! The terminal is not implemented yet.");
    println!("Shell: {} ({})", shell.name(), shell.binary().display());

    Ok(())
}
