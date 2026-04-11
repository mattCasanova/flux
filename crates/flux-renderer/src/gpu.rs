//! wgpu device, queue, and surface setup.
//!
//! This module handles GPU initialization. Everything here is
//! pub(crate) — no wgpu types leak outside flux-renderer.

// TODO: Phase 1, Step 1
// - Create wgpu::Instance
// - Request adapter + device + queue
// - Create surface from winit window
// - Configure surface format
