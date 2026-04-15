//! Atlas texture allocation and clearing — wgpu-specific GPU resource
//! setup that isolates the atlas size constant and the texture format
//! from the rasterization logic.

pub(crate) const ATLAS_SIZE: u32 = 1024;

pub(crate) fn create_atlas_texture(device: &wgpu::Device) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Glyph Atlas"),
        size: wgpu::Extent3d {
            width: ATLAS_SIZE,
            height: ATLAS_SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    })
}

pub(crate) fn clear_atlas_texture(queue: &wgpu::Queue, texture: &wgpu::Texture) {
    let zeros = vec![0u8; (ATLAS_SIZE * ATLAS_SIZE) as usize];
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &zeros,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(ATLAS_SIZE),
            rows_per_image: None,
        },
        wgpu::Extent3d {
            width: ATLAS_SIZE,
            height: ATLAS_SIZE,
            depth_or_array_layers: 1,
        },
    );
}
