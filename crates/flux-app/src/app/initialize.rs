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

use super::{App, INPUT_CHROME_ROWS};

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
        // Reserve `INPUT_CHROME_ROWS` rows at the bottom for the divider + input editor.
        let metrics = renderer.cell_metrics();
        let inner_size = window.inner_size();
        let usable_w = (inner_size.width as f32 - pad_x * 2.0).max(0.0);
        let usable_h = (inner_size.height as f32 - pad_y * 2.0).max(0.0);
        let cols = (usable_w / metrics.width) as usize;
        let total_rows = (usable_h / metrics.height) as usize;
        let rows = total_rows.saturating_sub(INPUT_CHROME_ROWS);
        log::info!(
            "Grid: {}x{} (padding {}x{}, chrome {} rows)",
            cols,
            rows,
            pad_x,
            pad_y,
            INPUT_CHROME_ROWS
        );

        // Create terminal state
        let terminal = TerminalState::new(cols.max(1), rows.max(1));

        // Spawn the PTY with matching dimensions
        let shell = flux_shell::detect_shell();
        let proxy = self.proxy.clone();
        let wake = Box::new(move || {
            let _ = proxy.send_event(());
        });
        let pty = PtyManager::spawn(
            shell.binary().to_str().unwrap_or("/bin/zsh"),
            cols.max(1) as u16,
            rows.max(1) as u16,
            wake,
        )?;

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

    fn create_renderer(
        &self,
        window: &Arc<Window>,
    ) -> anyhow::Result<flux_renderer::Renderer> {
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
