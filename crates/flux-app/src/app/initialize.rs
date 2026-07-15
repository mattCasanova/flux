//! App startup — window + renderer + PTY + terminal wiring.
//!
//! Runs once on the first `resumed` event. Builds the wgpu surface via
//! `Renderer::new`, spawns the shell in a PTY with matching grid
//! dimensions, and installs the initial frame.

use std::sync::Arc;

use winit::event_loop::ActiveEventLoop;
use winit::window::Window;

use flux_terminal::pty::PtyManager;
use flux_terminal::state::TerminalState;
use flux_types::Color;

use super::{App, MIN_INPUT_BAR_ROWS};

impl App {
    pub(super) fn initialize(&mut self, event_loop: &ActiveEventLoop) -> anyhow::Result<()> {
        let window_attrs = Window::default_attributes()
            .with_title(&self.config.window.title)
            .with_inner_size(winit::dpi::LogicalSize::new(
                self.config.window.width,
                self.config.window.height,
            ));

        let window = Arc::new(event_loop.create_window(window_attrs)?);
        let mut renderer = self.create_renderer(&window)?;

        // Apply padding from config
        let scale_factor = window.scale_factor() as f32;
        let pad_x = self.config.window.padding_horizontal * scale_factor;
        let pad_y = self.config.window.padding_vertical * scale_factor;
        renderer.set_padding(pad_x, pad_y);

        // Calculate grid dimensions from window size, padding, and cell metrics.
        // Reserve `MIN_INPUT_BAR_ROWS` rows at the bottom for the divider + input editor.
        let metrics = renderer.cell_metrics();
        let inner_size = window.inner_size();
        let usable_w = (inner_size.width as f32 - pad_x * 2.0).max(0.0);
        let usable_h = (inner_size.height as f32 - pad_y * 2.0).max(0.0);
        let cols = (usable_w / metrics.width) as usize;
        let total_rows = (usable_h / metrics.height) as usize;
        let rows = total_rows.saturating_sub(MIN_INPUT_BAR_ROWS);
        log::info!(
            "Grid: {}x{} (padding {}x{}, chrome {} rows)",
            cols,
            rows,
            pad_x,
            pad_y,
            MIN_INPUT_BAR_ROWS
        );

        // Create terminal state
        let terminal = TerminalState::new(
            cols.max(1),
            rows.max(1),
            self.config.scrollback.lines,
            self.config.theme.resolve(),
        );

        // Spawn the PTY with matching dimensions. Shell integration is
        // installed invisibly where the shell supports it: zsh gets a
        // ZDOTDIR bootstrap (nothing typed, nothing echoed at startup);
        // bash/fish still get the script sourced via the PTY below.
        let shell = flux_shell::detect_shell();
        let integration = shell.integration_script();
        let script_path = if integration.is_empty() {
            None
        } else {
            let script_dir = crate::platform::shell_integration_dir();
            let path = script_dir.join(format!("flux-integration.{}", shell.name()));
            match std::fs::write(&path, integration) {
                Ok(()) => Some(path),
                Err(e) => {
                    log::warn!("Failed to write integration script: {}", e);
                    None
                }
            }
        };

        let mut extra_env: Vec<(String, String)> = Vec::new();
        let mut inject_via_pty = script_path.is_some();
        if shell.name() == "zsh"
            && let Some(script) = &script_path
        {
            match Self::write_zsh_bootstrap(script) {
                Ok(zdotdir) => {
                    if let Ok(orig) = std::env::var("ZDOTDIR") {
                        extra_env.push(("FLUX_ORIG_ZDOTDIR".into(), orig));
                    }
                    extra_env.push(("ZDOTDIR".into(), zdotdir.display().to_string()));
                    inject_via_pty = false;
                    log::info!(
                        "zsh integration via ZDOTDIR bootstrap: {}",
                        zdotdir.display()
                    );
                }
                Err(e) => {
                    log::warn!("ZDOTDIR bootstrap failed, falling back to injection: {}", e);
                }
            }
        }

        let proxy = self.proxy.clone();
        let wake = Box::new(move || {
            let _ = proxy.send_event(());
        });
        let mut pty = PtyManager::spawn(
            shell.binary().to_str().unwrap_or("/bin/zsh"),
            cols.max(1) as u16,
            rows.max(1) as u16,
            wake,
            &extra_env,
        )?;

        if inject_via_pty && let Some(script) = &script_path {
            // Fallback path (bash/fish): source the script in the new
            // shell. Visible at startup — their invisible bootstraps
            // (--rcfile / vendor_conf.d) are future work.
            let source_cmd = format!("source '{}'\n", script.display());
            let _ = pty.write(source_cmd.as_bytes());
            log::info!("Shell integration injected: {}", script.display());
        }

        log::info!("Renderer + PTY initialized");
        self.renderer = Some(renderer);
        self.terminal = Some(terminal);
        self.pty = Some(pty);
        self.window = Some(window);

        self.update_display();
        self.update_input_display();
        self.request_redraw();

        Ok(())
    }

    /// Write the ZDOTDIR bootstrap directory: a `.zshenv` that restores
    /// the user's real ZDOTDIR, chains their own `.zshenv`, and sources
    /// the integration script — all before the first prompt, invisibly.
    fn write_zsh_bootstrap(script_path: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
        let dir = crate::platform::shell_integration_dir().join("zsh");
        std::fs::create_dir_all(&dir)?;
        let content = flux_shell::integration::ZSH_BOOTSTRAP_TEMPLATE.replace(
            "__FLUX_INTEGRATION_PATH__",
            &script_path.display().to_string(),
        );
        std::fs::write(dir.join(".zshenv"), content)?;
        Ok(dir)
    }

    fn create_renderer(&self, window: &Arc<Window>) -> anyhow::Result<flux_renderer::Renderer> {
        let scale_factor = window.scale_factor() as f32;
        let font_size_px = self.config.font.size * scale_factor;

        let mut renderer = flux_renderer::Renderer::new(
            window.clone(),
            &self.config.font.family,
            font_size_px,
            self.config.font.line_height,
            self.default_glyph_style(),
        )?;

        if let Some(bg) = Color::from_hex(&self.config.theme.background) {
            renderer.set_clear_color(bg);
        }

        let policy = match self.config.theme.alt_screen_background.as_deref() {
            None | Some("sync") => flux_renderer::AltBgPolicy::Sync,
            Some("theme") => flux_renderer::AltBgPolicy::Theme,
            Some(hex) => match Color::from_hex(hex) {
                Some(color) => flux_renderer::AltBgPolicy::Fixed(color),
                None => {
                    log::warn!(
                        "invalid [theme] alt_screen_background {:?}; using \"sync\"",
                        hex
                    );
                    flux_renderer::AltBgPolicy::Sync
                }
            },
        };
        renderer.set_alt_bg_policy(policy);

        if let Some(hex) = &self.config.scrollback.scrolled_background {
            match Color::from_hex(hex) {
                Some(color) => renderer.set_scrolled_background(Some(color)),
                None => log::warn!("invalid [scrollback] scrolled_background {:?}", hex),
            }
        }

        Ok(renderer)
    }

    /// Default glyph style applied to cells with no BOLD/ITALIC flag —
    /// read from `[font] weight` and `[font] style` in the config file.
    fn default_glyph_style(&self) -> flux_renderer::GlyphStyle {
        let bold = self.config.font.weight.eq_ignore_ascii_case("bold");
        let italic = self.config.font.style.eq_ignore_ascii_case("italic");
        flux_renderer::GlyphStyle::from_flags(bold, italic)
    }
}
