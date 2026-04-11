//! Application state and event handling.

use std::sync::Arc;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowId};

use flux_types::Color;

use crate::config::FluxConfig;

/// Application state — owns the window and renderer.
pub struct App {
    config: FluxConfig,
    window: Option<Arc<Window>>,
    renderer: Option<flux_renderer::Renderer>,
}

impl App {
    pub fn new(config: FluxConfig) -> Self {
        Self {
            config,
            window: None,
            renderer: None,
        }
    }

    /// Compute the font pixel size from config + display scale factor.
    fn scaled_font_size(&self, scale_factor: f32) -> f32 {
        self.config.font.size * scale_factor
    }

    /// Whether the config specifies bold weight.
    fn is_bold(&self) -> bool {
        self.config.font.weight.to_lowercase() == "bold"
    }

    /// Create the renderer for the given window.
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

        // Apply theme colors
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

    /// Render test text. Temporary — will be replaced by terminal grid.
    fn render_test_text(&mut self) {
        let fg = Color::from_hex(&self.config.theme.foreground).unwrap_or(Color::default());
        let bg = Color::new(0.0, 0.0, 0.0, 0.0);

        if let Some(renderer) = &mut self.renderer {
            renderer.set_text("Great Scott! Flux is rendering text.", 20.0, 40.0, fg, bg);
        }
    }

    /// Handle a display scale factor change (moving between monitors).
    fn handle_scale_change(&mut self, scale_factor: f32) {
        log::info!("Scale factor changed to {}", scale_factor);

        let font_size_px = self.config.font.size * scale_factor;
        let bold = self.is_bold();
        let font_family = self.config.font.family.clone();
        let line_height = self.config.font.line_height;
        let fg = Color::from_hex(&self.config.theme.foreground).unwrap_or(Color::default());
        let bg = Color::new(0.0, 0.0, 0.0, 0.0);

        if let Some(renderer) = &mut self.renderer {
            if let Err(e) = renderer.rebuild_font(&font_family, font_size_px, line_height, bold) {
                log::error!("Failed to rebuild font: {}", e);
                return;
            }
            renderer.set_text("Great Scott! Flux is rendering text.", 20.0, 40.0, fg, bg);
        }

        if let Some(window) = &self.window {
            let size = window.inner_size();
            if let Some(renderer) = &mut self.renderer {
                renderer.resize(size.width, size.height);
            }
            window.request_redraw();
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let window_attrs = Window::default_attributes()
            .with_title(&self.config.window.title)
            .with_inner_size(winit::dpi::LogicalSize::new(
                self.config.window.width,
                self.config.window.height,
            ));

        let window = match event_loop.create_window(window_attrs) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                log::error!("Failed to create window: {}", e);
                event_loop.exit();
                return;
            }
        };

        match self.create_renderer(&window) {
            Ok(renderer) => {
                log::info!("Renderer initialized");
                self.renderer = Some(renderer);
                self.render_test_text();
            }
            Err(e) => {
                log::error!("Failed to create renderer: {}", e);
                event_loop.exit();
                return;
            }
        }

        self.window = Some(window);
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
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize(size.width, size.height);
                }
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                self.handle_scale_change(scale_factor as f32);
            }
            WindowEvent::RedrawRequested => {
                if let Some(renderer) = &mut self.renderer {
                    if let Err(e) = renderer.render() {
                        log::error!("Render error: {}", e);
                    }
                }
            }
            _ => {}
        }
    }
}
