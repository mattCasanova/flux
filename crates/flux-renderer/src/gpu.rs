//! wgpu device, queue, and surface setup.
//!
//! This module handles GPU initialization. Everything here is
//! pub(crate) — no wgpu types leak outside flux-renderer.

use anyhow::Result;
use std::sync::Arc;

/// Holds all wgpu state — device, queue, surface, and config.
pub(crate) struct GpuContext {
    pub device: wgpu::Device,
    pub queue: wgpu::Queue,
    pub surface: wgpu::Surface<'static>,
    pub surface_config: wgpu::SurfaceConfiguration,
}

impl GpuContext {
    /// Initialize wgpu with a winit window.
    pub fn new(window: Arc<winit::window::Window>) -> Result<Self> {
        let size = window.inner_size();
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let surface = instance.create_surface(window)?;
        let adapter = Self::request_adapter(&instance, &surface)?;
        let (device, queue) = Self::request_device(&adapter)?;
        let surface_config = Self::configure_surface(&surface, &adapter, &device, size);

        Ok(Self {
            device,
            queue,
            surface,
            surface_config,
        })
    }

    /// Resize the surface when the window size changes.
    pub fn resize(&mut self, width: u32, height: u32) {
        if width > 0 && height > 0 {
            self.surface_config.width = width;
            self.surface_config.height = height;
            self.surface.configure(&self.device, &self.surface_config);
        }
    }

    /// Get the surface texture format.
    pub fn format(&self) -> wgpu::TextureFormat {
        self.surface_config.format
    }

    fn request_adapter(
        instance: &wgpu::Instance,
        surface: &wgpu::Surface,
    ) -> Result<wgpu::Adapter> {
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(surface),
            force_fallback_adapter: false,
        }))?;

        log::info!("GPU adapter: {}", adapter.get_info().name);
        log::info!(
            "Available surface formats: {:?}",
            surface.get_capabilities(&adapter).formats
        );

        Ok(adapter)
    }

    fn request_device(adapter: &wgpu::Adapter) -> Result<(wgpu::Device, wgpu::Queue)> {
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("Flux GPU Device"),
                ..Default::default()
            },
        ))?;

        Ok((device, queue))
    }

    /// Use non-sRGB format — our colors are already in sRGB space (hex from theme file).
    /// Using an sRGB format would double-gamma them, making colors appear washed out.
    fn configure_surface(
        surface: &wgpu::Surface,
        adapter: &wgpu::Adapter,
        device: &wgpu::Device,
        size: winit::dpi::PhysicalSize<u32>,
    ) -> wgpu::SurfaceConfiguration {
        let surface_caps = surface.get_capabilities(adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .find(|f| !f.is_srgb())
            .copied()
            .unwrap_or(surface_caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width,
            height: size.height,
            present_mode: wgpu::PresentMode::Fifo,
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(device, &config);

        log::info!("Selected surface format: {:?}", surface_format);

        config
    }
}
