//! Flux — a modern, GPU-accelerated terminal with command blocks.
//!
//! "Where we're going, we don't need Electron."

use std::sync::Arc;
use anyhow::Result;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};
use flux_types::Color;

/// Application state — owns the window and renderer.
struct App {
    window: Option<Arc<Window>>,
    renderer: Option<flux_renderer::Renderer>,
}

impl App {
    fn new() -> Self {
        Self {
            window: None,
            renderer: None,
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_none() {
            let window_attrs = Window::default_attributes()
                .with_title("Flux — 1.21 gigawatts")
                .with_inner_size(winit::dpi::LogicalSize::new(1200, 800));

            match event_loop.create_window(window_attrs) {
                Ok(window) => {
                    let window = Arc::new(window);
                    match flux_renderer::Renderer::new(
                        window.clone(),
                        "Menlo",  // font family
                        14.0,     // font size
                    ) {
                        Ok(mut renderer) => {
                            log::info!("Renderer initialized");
                            let metrics = renderer.cell_metrics();
                            log::info!(
                                "Cell metrics: {:.1}x{:.1}",
                                metrics.width,
                                metrics.height,
                            );

                            // Render some test text
                            let fg = Color::from_hex("#c0caf5").unwrap(); // Tokyo Night foreground
                            let bg = Color::new(0.0, 0.0, 0.0, 0.0);    // transparent bg
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

    let shell = flux_shell::detect_shell();
    log::info!("Detected shell: {} ({})", shell.name(), shell.binary().display());

    println!("Flux v0.1.0 — 1.21 gigawatts");

    let event_loop = EventLoop::new()?;
    let mut app = App::new();
    event_loop.run_app(&mut app)?;

    Ok(())
}
