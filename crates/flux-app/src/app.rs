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
        let renderer = self.create_renderer(&window)?;

        // Calculate grid dimensions from window size and cell metrics
        let metrics = renderer.cell_metrics();
        let inner_size = window.inner_size();
        let cols = (inner_size.width as f32 / metrics.width) as usize;
        let rows = (inner_size.height as f32 / metrics.height) as usize;
        log::info!("Grid: {}x{}", cols, rows);

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
        use winit::keyboard::Key;

        if event.state != ElementState::Pressed {
            return;
        }

        let Some(pty) = &mut self.pty else { return };

        match &event.logical_key {
            Key::Named(winit::keyboard::NamedKey::Enter) => {
                let _ = pty.write(b"\r");
            }
            Key::Named(winit::keyboard::NamedKey::Backspace) => {
                let _ = pty.write(b"\x7f");
            }
            Key::Named(winit::keyboard::NamedKey::ArrowUp) => {
                let _ = pty.write(b"\x1b[A");
            }
            Key::Named(winit::keyboard::NamedKey::ArrowDown) => {
                let _ = pty.write(b"\x1b[B");
            }
            Key::Named(winit::keyboard::NamedKey::ArrowRight) => {
                let _ = pty.write(b"\x1b[C");
            }
            Key::Named(winit::keyboard::NamedKey::ArrowLeft) => {
                let _ = pty.write(b"\x1b[D");
            }
            Key::Named(winit::keyboard::NamedKey::Tab) => {
                let _ = pty.write(b"\t");
            }
            Key::Named(winit::keyboard::NamedKey::Escape) => {
                let _ = pty.write(b"\x1b");
            }
            _ => {
                if let Some(text) = &event.text {
                    let _ = pty.write(text.as_bytes());
                }
            }
        }

        self.request_redraw();
    }

    fn handle_resize(&mut self, width: u32, height: u32) {
        let Some(renderer) = &mut self.renderer else { return };
        renderer.resize(width, height);

        // Recalculate grid dimensions and resize PTY + terminal
        let metrics = renderer.cell_metrics();
        let cols = (width as f32 / metrics.width) as usize;
        let rows = (height as f32 / metrics.height) as usize;

        if cols > 0 && rows > 0 {
            if let Some(terminal) = &mut self.terminal {
                terminal.resize(cols, rows);
            }
            if let Some(pty) = &mut self.pty {
                let _ = pty.resize(cols as u16, rows as u16);
            }
        }

        self.request_redraw();
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
