//! GPU rendering for the Flux terminal emulator.
//!
//! ALL wgpu code lives in this crate. Nothing outside flux-renderer
//! imports wgpu. Other crates interact through [flux_types] data
//! structures only.

mod gpu;
mod atlas;
mod pipeline;
mod cell_renderer;
mod ui_renderer;

use std::sync::Arc;
use anyhow::Result;
use flux_types::Color;

/// The renderer — owns all GPU state and renders frames.
pub struct Renderer {
    gpu: gpu::GpuContext,
    clear_color: Color,
}

impl Renderer {
    /// Create a new renderer attached to a winit window.
    pub fn new(window: Arc<winit::window::Window>) -> Result<Self> {
        let gpu = gpu::GpuContext::new(window)?;
        Ok(Self {
            gpu,
            clear_color: Color::from_hex("#24283b").unwrap(), // Tokyo Night Storm bg
        })
    }

    /// Handle window resize.
    pub fn resize(&mut self, width: u32, height: u32) {
        self.gpu.resize(width, height);
    }

    /// Set the clear color (background).
    pub fn set_clear_color(&mut self, color: Color) {
        self.clear_color = color;
    }

    /// Render a frame. For now, just clears to the background color.
    pub fn render(&mut self) -> Result<()> {
        let output = match self.gpu.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture)
            | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => texture,
            wgpu::CurrentSurfaceTexture::Timeout
            | wgpu::CurrentSurfaceTexture::Occluded => return Ok(()), // skip this frame
            other => return Err(anyhow::anyhow!("Surface texture error: {:?}", other)),
        };
        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());

        let mut encoder = self.gpu.device.create_command_encoder(
            &wgpu::CommandEncoderDescriptor {
                label: Some("Flux Render Encoder"),
            },
        );

        // Clear the screen to our background color
        {
            let _render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Clear Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: self.clear_color.r as f64,
                            g: self.clear_color.g as f64,
                            b: self.clear_color.b as f64,
                            a: self.clear_color.a as f64,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                    depth_slice: None,
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            // render pass ends when dropped
        }

        self.gpu.queue.submit(std::iter::once(encoder.finish()));
        output.present();

        Ok(())
    }
}
