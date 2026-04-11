//! Application state and event handling.

use std::sync::Arc;

use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowId};

use flux_terminal::pty::{PtyEvent, PtyManager};
use flux_types::Color;

use crate::config::FluxConfig;

/// Application state — owns the window, renderer, and PTY.
pub struct App {
    config: FluxConfig,
    window: Option<Arc<Window>>,
    renderer: Option<flux_renderer::Renderer>,
    pty: Option<PtyManager>,
    /// Accumulated PTY output for display (temporary — replaced by terminal grid).
    output_text: String,
}

impl App {
    pub fn new(config: FluxConfig) -> Self {
        Self {
            config,
            window: None,
            renderer: None,
            pty: None,
            output_text: String::new(),
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
                self.process_pty_output();
                self.handle_redraw();
            }

            WindowEvent::KeyboardInput { event, is_synthetic: false, .. } => {
                self.handle_keyboard(event);
            }

            _ => {}
        }
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

        log::info!("Renderer initialized");
        self.renderer = Some(renderer);

        // Spawn the PTY
        let shell = flux_shell::detect_shell();
        let pty = PtyManager::spawn(
            shell.binary().to_str().unwrap_or("/bin/zsh"),
            80,
            24,
        )?;
        log::info!("PTY spawned");
        self.pty = Some(pty);

        self.window = Some(window);

        // Request first frame
        self.request_redraw();

        Ok(())
    }

    fn scaled_font_size(&self, scale_factor: f32) -> f32 {
        self.config.font.size * scale_factor
    }

    fn is_bold(&self) -> bool {
        self.config.font.weight.to_lowercase() == "bold"
    }

    fn create_renderer(&self, window: &Arc<Window>) -> anyhow::Result<flux_renderer::Renderer> {
        let scale_factor = window.scale_factor() as f32;
        let font_size_px = self.scaled_font_size(scale_factor);

        log::info!(
            "Scale factor: {}, font size: {}pt -> {}px",
            scale_factor,
            self.config.font.size,
            font_size_px,
        );

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

        let metrics = renderer.cell_metrics();
        log::info!(
            "Font: {} {}pt ({}), cell: {:.1}x{:.1}",
            self.config.font.family,
            self.config.font.size,
            self.config.font.weight,
            metrics.width,
            metrics.height,
        );

        Ok(renderer)
    }

    /// Check for new PTY output and update the display.
    fn process_pty_output(&mut self) {
        let Some(pty) = &self.pty else { return };

        let mut got_output = false;
        for event in pty.read_events() {
            match event {
                PtyEvent::Output(bytes) => {
                    // For now, just accumulate as text (temporary — replaced by alacritty_terminal)
                    let text = String::from_utf8_lossy(&bytes);
                    self.output_text.push_str(&text);
                    got_output = true;
                }
                PtyEvent::Exited => {
                    log::info!("Shell exited");
                }
            }
        }

        if got_output {
            self.update_display();
        }
    }

    /// Update the rendered text from accumulated output.
    fn update_display(&mut self) {
        let renderer = self.renderer.as_mut().expect("renderer not initialized");
        let fg = Color::from_hex(&self.config.theme.foreground).unwrap_or(Color::default());
        let bg = Color::new(0.0, 0.0, 0.0, 0.0);

        // Show the last chunk of output (temporary — will be replaced by terminal grid)
        let display_text = if self.output_text.len() > 200 {
            &self.output_text[self.output_text.len() - 200..]
        } else {
            &self.output_text
        };

        // Strip control characters for raw display (temporary)
        let clean: String = display_text
            .chars()
            .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
            .collect();

        renderer.set_text(&clean, 20.0, 40.0, fg, bg);
    }

    fn handle_keyboard(&mut self, event: winit::event::KeyEvent) {
        use winit::event::ElementState;
        use winit::keyboard::Key;

        if event.state != ElementState::Pressed {
            return;
        }

        let Some(pty) = &mut self.pty else { return };

        // For now, forward all text input directly to PTY
        // (temporary — will go through input editor later)
        match &event.logical_key {
            Key::Named(winit::keyboard::NamedKey::Enter) => {
                let _ = pty.write(b"\r");
            }
            Key::Named(winit::keyboard::NamedKey::Backspace) => {
                let _ = pty.write(b"\x7f");
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
        let renderer = self.renderer.as_mut().expect("renderer not initialized");
        renderer.resize(width, height);
        self.request_redraw();
    }

    fn handle_redraw(&mut self) {
        let renderer = self.renderer.as_mut().expect("renderer not initialized");
        if let Err(e) = renderer.render() {
            log::error!("Render error: {}", e);
        }
    }

    fn handle_scale_change(&mut self, scale_factor: f32) {
        log::info!("Scale factor changed to {}", scale_factor);

        let font_size_px = self.config.font.size * scale_factor;
        let bold = self.is_bold();
        let font_family = self.config.font.family.clone();
        let line_height = self.config.font.line_height;

        let renderer = self.renderer.as_mut().expect("renderer not initialized");
        if let Err(e) = renderer.rebuild_font(&font_family, font_size_px, line_height, bold) {
            log::error!("Failed to rebuild font: {}", e);
            return;
        }

        self.update_display();

        let window = self.window.as_ref().expect("window not initialized");
        let size = window.inner_size();
        let renderer = self.renderer.as_mut().expect("renderer not initialized");
        renderer.resize(size.width, size.height);
        self.request_redraw();
    }

    fn request_redraw(&self) {
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}
