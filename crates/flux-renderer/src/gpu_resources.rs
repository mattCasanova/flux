//! One-time GPU resource creation helpers.
//!
//! These functions run during `Renderer::new` (and once per atlas rebuild
//! on DPI change). They never touch `self` — pure `fn`s that take a
//! `wgpu::Device` and return a resource. Isolating them here keeps
//! `renderer.rs` focused on lifecycle rather than GPU initialization.

use wgpu::util::DeviceExt;

use crate::cell_renderer::{CellInstance, QUAD_VERTICES};
use crate::pipeline::Uniforms;

pub(crate) fn create_quad_buffer(device: &wgpu::Device) -> wgpu::Buffer {
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Quad Vertex Buffer"),
        contents: bytemuck::cast_slice(QUAD_VERTICES),
        usage: wgpu::BufferUsages::VERTEX,
    })
}

pub(crate) fn create_uniform_buffer(
    device: &wgpu::Device,
    config: &wgpu::SurfaceConfiguration,
) -> wgpu::Buffer {
    let uniforms = Uniforms::ortho(config.width as f32, config.height as f32);
    device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Uniform Buffer"),
        contents: bytemuck::cast_slice(&[uniforms]),
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    })
}

pub(crate) fn create_instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    let size = (capacity * std::mem::size_of::<CellInstance>()) as u64;
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Cell Instance Buffer"),
        size,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}

pub(crate) fn create_sampler(device: &wgpu::Device) -> wgpu::Sampler {
    device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("Atlas Sampler"),
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        ..Default::default()
    })
}
