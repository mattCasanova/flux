//! Flux — a modern, GPU-accelerated terminal with command blocks.
//!
//! "Where we're going, we don't need Electron."

mod config;

use std::sync::Arc;
use anyhow::Result;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};
use flux_types::Color;

use config::FluxConfig;

/// Application state — owns the window and renderer.
struct App {
    config: FluxConfig,
    window: Option<Arc<Window>>,
    renderer: Option<flux_renderer::Renderer>,
}

impl App {
    fn new(config: FluxConfig) -> Self {
        Self {
            config,
            window: None,
            renderer: None,
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            let window_attrs = Window::default_attributes()
                .with_title(&self.config.window.title)
                .with_inner_size(winit::dpi::LogicalSize::new(
                    self.config.window.width,
                    self.config.window.height,
                ));

            match event_loop.create_window(window_attrs) {
                Ok(window) => {
                    let window = Arc::new(window);
                    let bold = self.config.font.weight.to_lowercase() == "bold";
                    // On macOS: scale_factor is 2.0 on Retina, 1.0 on standard displays.
                    // Font size in config is in points (like iTerm2/VS Code).
                    // cosmic-text takes pixels, so multiply by scale factor.
                    let scale_factor = window.scale_factor() as f32;
                    let scaled_font_size = self.config.font.size * scale_factor;
                    log::info!("Scale factor: {}, font size: {}pt -> {}px", scale_factor, self.config.font.size, scaled_font_size);
                    match flux_renderer::Renderer::new(
                        window.clone(),
                        &self.config.font.family,
                        scaled_font_size,
                        self.config.font.line_height,
                        bold,
                    ) {
                        Ok(mut renderer) => {
                            log::info!("Renderer initialized");
                            let metrics = renderer.cell_metrics();
                            log::info!(
                                "Font: {} {}pt ({}), cell: {:.1}x{:.1}",
                                self.config.font.family,
                                self.config.font.size,
                                self.config.font.weight,
                                metrics.width,
                                metrics.height,
                            );

                            // Set background color from config
                            if let Some(bg) = Color::from_hex(&self.config.theme.background) {
                                renderer.set_clear_color(bg);
                            }

                            // Render some test text
                            let fg = Color::from_hex(&self.config.theme.foreground)
                                .unwrap_or(Color::default());
                            let bg = Color::new(0.0, 0.0, 0.0, 0.0);
                            renderer.set_text(
                                "Great Scott! Flux is rendering text.",
                                20.0,
                                40.0,
                                fg,
                                bg,
                            );

                            self.renderer = Some(renderer);
                        }
                        Err(e) => {
                            log::error!("Failed to create renderer: {}", e);
                            event_loop.exit();
                            return;
                        }
                    }
                    self.window = Some(window);
                }
                Err(e) => {
                    log::error!("Failed to create window: {}", e);
                    event_loop.exit();
                }
            }
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
            WindowEvent::Resized(physical_size) => {
                if let Some(renderer) = &mut self.renderer {
                    renderer.resize(physical_size.width, physical_size.height);
                }
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
            WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
                let scale_factor = scale_factor as f32;
                log::info!("Scale factor changed to {}", scale_factor);

                if let Some(renderer) = &mut self.renderer {
                    // Rebuild atlas at new scale
                    let bold = self.config.font.weight.to_lowercase() == "bold";
                    let scaled_font_size = self.config.font.size * scale_factor;
                    if let Err(e) = renderer.rebuild_font(
                        &self.config.font.family,
                        scaled_font_size,
                        self.config.font.line_height,
                        bold,
                    ) {
                        log::error!("Failed to rebuild font: {}", e);
                    }

                    // Re-render test text
                    let fg = Color::from_hex(&self.config.theme.foreground)
                        .unwrap_or(Color::default());
                    let bg = Color::new(0.0, 0.0, 0.0, 0.0);
                    renderer.set_text(
                        "Great Scott! Flux is rendering text.",
                        20.0,
                        40.0,
                        fg,
                        bg,
                    );

                    // Resize surface
                    if let Some(window) = &self.window {
                        let size = window.inner_size();
                        renderer.resize(size.width, size.height);
                        window.request_redraw();
                    }
                }
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

fn main() -> Result<()> {
    env_logger::init();

    // Load configuration
    let config = FluxConfig::load()?;
    log::info!("Font: {} {}pt {}", config.font.family, config.font.size, config.font.weight);

    let shell = flux_shell::detect_shell();
    log::info!("Detected shell: {} ({})", shell.name(), shell.binary().display());

    println!("Flux v0.1.0 — 1.21 gigawatts");

    let event_loop = EventLoop::new()?;
    let mut app = App::new(config);
    event_loop.run_app(&mut app)?;

    Ok(())
}
