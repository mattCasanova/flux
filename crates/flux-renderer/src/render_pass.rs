//! Frame rendering — `render()` and its private helpers.
//!
//! Owns the per-frame work that doesn't change with content: acquire a
//! surface texture, begin a render pass, clear, draw all instances in
//! one call, submit. Nothing here rebuilds the instance buffer — that
//! happens in `set_grid` / `set_input_line` via `rebuild_combined_buffer`
//! in `buffer.rs`.

use anyhow::Result;

use crate::renderer::Renderer;

impl Renderer {
    /// Render a frame.
    pub fn render(&mut self) -> Result<()> {
        let output = self.acquire_surface_texture()?;
        let Some(output) = output else { return Ok(()) };

        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Flux Render Encoder"),
            });

        self.render_pass(&mut encoder, &view);

        self.gpu.queue.submit(std::iter::once(encoder.finish()));
        output.present();
        Ok(())
    }

    /// Acquire the next surface texture. Returns None if the frame should be skipped.
    fn acquire_surface_texture(&mut self) -> Result<Option<wgpu::SurfaceTexture>> {
        match self.gpu.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(texture)
            | wgpu::CurrentSurfaceTexture::Suboptimal(texture) => Ok(Some(texture)),
            wgpu::CurrentSurfaceTexture::Timeout | wgpu::CurrentSurfaceTexture::Occluded => {
                Ok(None)
            }
            other => Err(anyhow::anyhow!("Surface texture error: {:?}", other)),
        }
    }

    /// Execute the render pass — clear screen and draw glyphs.
    fn render_pass(&self, encoder: &mut wgpu::CommandEncoder, view: &wgpu::TextureView) {
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Main Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(self.wgpu_clear_color()),
                    store: wgpu::StoreOp::Store,
                },
                depth_slice: None,
            })],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });

        self.draw_glyphs(&mut pass);
    }

    /// Draw all glyph instances in a single draw call.
    fn draw_glyphs<'a>(&'a self, pass: &mut wgpu::RenderPass<'a>) {
        if self.instance_count == 0 {
            return;
        }

        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, &self.bind_group, &[]);
        pass.set_vertex_buffer(0, self.quad_vertex_buffer.slice(..));
        pass.set_vertex_buffer(1, self.instance_buffer.slice(..));
        pass.draw(0..4, 0..self.instance_count);
    }

    /// Convert our Color to wgpu::Color. Uses the effective clear so
    /// raw-mode alt-screen programs fill the whole window with their bg,
    /// not just the cells they happen to cover.
    fn wgpu_clear_color(&self) -> wgpu::Color {
        wgpu::Color {
            r: self.effective_clear_color.r as f64,
            g: self.effective_clear_color.g as f64,
            b: self.effective_clear_color.b as f64,
            a: self.effective_clear_color.a as f64,
        }
    }
}
