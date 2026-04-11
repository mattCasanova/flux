//! Flux — a modern, GPU-accelerated terminal with command blocks.
//!
//! "Where we're going, we don't need Electron."

use std::sync::Arc;
use anyhow::Result;
use winit::application::ApplicationHandler;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

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
        // Create the window on first resume
        if self.window.is_none() {
            let window_attrs = Window::default_attributes()
                .with_title("Flux — 1.21 gigawatts")
                .with_inner_size(winit::dpi::LogicalSize::new(1200, 800));

            match event_loop.create_window(window_attrs) {
                Ok(window) => {
                    let window = Arc::new(window);
                    match flux_renderer::Renderer::new(window.clone()) {
                        Ok(renderer) => {
                            log::info!("Renderer initialized");
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
            }
            WindowEvent::RedrawRequested => {
                if let Some(renderer) = &mut self.renderer {
                    if let Err(e) = renderer.render() {
                        log::error!("Render error: {}", e);
                    }
                }
                // Request another frame
                if let Some(window) = &self.window {
                    window.request_redraw();
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
