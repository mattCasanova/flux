//! Application state and event handling.

use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowId};

use flux_terminal::pty::{PtyEvent, PtyManager};
use flux_terminal::state::{TermEvent, TerminalState};
use flux_types::Color;

use crate::config::FluxConfig;

/// Application state — owns the window, renderer, PTY, and terminal state.
pub struct App {
    config: FluxConfig,
    proxy: winit::event_loop::EventLoopProxy<()>,
    window: Option<Arc<Window>>,
    renderer: Option<flux_renderer::Renderer>,
    pty: Option<PtyManager>,
    terminal: Option<TerminalState>,
}

impl App {
    pub fn new(config: FluxConfig, proxy: winit::event_loop::EventLoopProxy<()>) -> Self {
        Self {
            config,
            proxy,
            window: None,
            renderer: None,
            pty: None,
            terminal: None,
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        if let Err(e) = self.initialize(event_loop) {
            log::error!("Failed to initialize: {}", e);
            event_loop.exit();
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => {
                event_loop.exit();
            }

            WindowEvent::Resized(size) => {
                self.handle_resize(size.width, size.height);
            }

            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.handle_scale_change(scale_factor as f32);
            }

            WindowEvent::RedrawRequested => {
                self.handle_redraw();
            }

            WindowEvent::KeyboardInput {
                event,
                is_synthetic: false,
                ..
            } => {
                self.handle_keyboard(event);
            }

            _ => {}
        }
    }

    fn user_event(&mut self, _event_loop: &ActiveEventLoop, _event: ()) {
        // PTY output arrived — process and redraw
        self.process_pty_output();
        self.request_redraw();
    }
}

// --- Private helpers ---

impl App {
    fn initialize(&mut self, event_loop: &ActiveEventLoop) -> anyhow::Result<()> {
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

        // Calculate grid dimensions from window size, padding, and cell metrics
        let metrics = renderer.cell_metrics();
        let inner_size = window.inner_size();
        let usable_w = (inner_size.width as f32 - pad_x * 2.0).max(0.0);
        let usable_h = (inner_size.height as f32 - pad_y * 2.0).max(0.0);
        let cols = (usable_w / metrics.width) as usize;
        let rows = (usable_h / metrics.height) as usize;
        log::info!("Grid: {}x{} (padding {}x{})", cols, rows, pad_x, pad_y);

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
            self.is_bold(),
        )?;

        if let Some(bg) = Color::from_hex(&self.config.theme.background) {
            renderer.set_clear_color(bg);
        }

        Ok(renderer)
    }

    fn is_bold(&self) -> bool {
        self.config.font.weight.to_lowercase() == "bold"
    }

    /// Process pending PTY output through alacritty_terminal.
    fn process_pty_output(&mut self) {
        let Some(pty) = &self.pty else { return };
        let Some(terminal) = &mut self.terminal else { return };

        let mut dirty = false;

        for event in pty.read_events() {
            match event {
                PtyEvent::Output(bytes) => {
                    terminal.process_bytes(&bytes);
                    dirty = true;
                }
                PtyEvent::Exited => {
                    log::info!("Shell exited");
                }
            }
        }

        // Handle events from alacritty_terminal (PtyWrite responses)
        if dirty {
            for event in terminal.drain_events() {
                match event {
                    TermEvent::PtyWrite(text) => {
                        if let Some(pty) = &mut self.pty {
                            let _ = pty.write(text.as_bytes());
                        }
                    }
                    TermEvent::Title(title) => {
                        if let Some(window) = &self.window {
                            window.set_title(&title);
                        }
                    }
                    TermEvent::Bell => {
                        log::debug!("Bell");
                    }
                }
            }

            self.update_display();
        }
    }

    /// Render the terminal grid.
    fn update_display(&mut self) {
        let Some(terminal) = &self.terminal else { return };
        let Some(renderer) = &mut self.renderer else { return };

        let fg = Color::from_hex(&self.config.theme.foreground).unwrap_or(Color::default());
        let bg = Color::from_hex(&self.config.theme.background)
            .unwrap_or(Color::new(0.0, 0.0, 0.0, 1.0));

        let grid = terminal.render_grid(fg, bg);
        renderer.set_grid(&grid);
    }

    fn handle_keyboard(&mut self, event: winit::event::KeyEvent) {
        use winit::event::ElementState;
        use winit::keyboard::{Key, NamedKey};
        use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;

        if event.state != ElementState::Pressed {
            return;
        }

        let Some(pty) = &mut self.pty else { return };

        // Special keys that map to escape sequences
        let bytes: Option<Vec<u8>> = match &event.logical_key {
            Key::Named(NamedKey::Enter) => Some(b"\r".to_vec()),
            Key::Named(NamedKey::Backspace) => Some(b"\x7f".to_vec()),
            Key::Named(NamedKey::ArrowUp) => Some(b"\x1b[A".to_vec()),
            Key::Named(NamedKey::ArrowDown) => Some(b"\x1b[B".to_vec()),
            Key::Named(NamedKey::ArrowRight) => Some(b"\x1b[C".to_vec()),
            Key::Named(NamedKey::ArrowLeft) => Some(b"\x1b[D".to_vec()),
            Key::Named(NamedKey::Home) => Some(b"\x1b[H".to_vec()),
            Key::Named(NamedKey::End) => Some(b"\x1b[F".to_vec()),
            Key::Named(NamedKey::PageUp) => Some(b"\x1b[5~".to_vec()),
            Key::Named(NamedKey::PageDown) => Some(b"\x1b[6~".to_vec()),
            Key::Named(NamedKey::Delete) => Some(b"\x1b[3~".to_vec()),
            Key::Named(NamedKey::Tab) => Some(b"\t".to_vec()),
            Key::Named(NamedKey::Escape) => Some(b"\x1b".to_vec()),
            _ => None,
        };

        if let Some(bytes) = bytes {
            let _ = pty.write(&bytes);
        } else {
            // Regular text input — use text_with_all_modifiers() which includes
            // Ctrl effects (Ctrl+C -> \x03, Ctrl+D -> \x04, etc.).
            // text_with_all_modifiers is the right field for terminals; the plain
            // `event.text` is for text editors and excludes Ctrl effects.
            if let Some(text) = event.text_with_all_modifiers() {
                let _ = pty.write(text.as_bytes());
            }
        }

        self.request_redraw();
    }

    fn handle_resize(&mut self, width: u32, height: u32) {
        // Reconfigure surface + resize grid + render — all in the same event.
        // Presenting a frame before returning from the resize handler prevents
        // the compositor from stretching a stale frame.

        // 1. Resize surface + get metrics
        let renderer = self.renderer.as_mut().expect("renderer not initialized");
        renderer.resize(width, height);
        let cell_w = renderer.cell_metrics().width;
        let cell_h = renderer.cell_metrics().height;

        // 2. Resize terminal grid + PTY (account for padding)
        let scale_factor = self.window.as_ref().map(|w| w.scale_factor() as f32).unwrap_or(1.0);
        let pad_x = self.config.window.padding_horizontal * scale_factor;
        let pad_y = self.config.window.padding_vertical * scale_factor;
        let usable_w = (width as f32 - pad_x * 2.0).max(0.0);
        let usable_h = (height as f32 - pad_y * 2.0).max(0.0);
        let cols = (usable_w / cell_w) as usize;
        let rows = (usable_h / cell_h) as usize;

        if cols > 0 && rows > 0 {
            if let Some(terminal) = &mut self.terminal {
                terminal.resize(cols, rows);
            }
            if let Some(pty) = &mut self.pty {
                let _ = pty.resize(cols as u16, rows as u16);
            }
        }

        // 3. Update display + render immediately (no RedrawRequested wait)
        self.update_display();
        let renderer = self.renderer.as_mut().expect("renderer not initialized");
        if let Err(e) = renderer.render() {
            log::error!("Resize render error: {}", e);
        }
    }

    fn handle_redraw(&mut self) {
        let Some(renderer) = &mut self.renderer else { return };
        if let Err(e) = renderer.render() {
            log::error!("Render error: {}", e);
        }
    }

    fn handle_scale_change(&mut self, scale_factor: f32) {
        log::info!("Scale factor changed to {}", scale_factor);

        let font_size_px = self.config.font.size * scale_factor;
        let font_family = self.config.font.family.clone();
        let line_height = self.config.font.line_height;
        let bold = self.is_bold();

        let Some(renderer) = &mut self.renderer else { return };
        if let Err(e) = renderer.rebuild_font(&font_family, font_size_px, line_height, bold) {
            log::error!("Failed to rebuild font: {}", e);
            return;
        }

        // Recalculate grid after font change
        if let Some(window) = &self.window {
            let size = window.inner_size();
            renderer.resize(size.width, size.height);
        }

        self.update_display();
        self.request_redraw();
    }

    fn request_redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}
