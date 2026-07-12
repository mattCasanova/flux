//! Low-level GPU plumbing — device, pipeline, buffers, frame submission.
//!
//! Everything here is created once at startup and rarely touched during
//! feature work. Higher-level rendering logic (set_grid, set_input_line)
//! lives in sibling modules at the crate root.

mod buffer;
mod gpu;
mod pipeline;
mod render_pass;
mod resources;
mod types;

pub(crate) use buffer::{INITIAL_MAX_CELLS, color_matches};
pub(crate) use gpu::GpuContext;
pub(crate) use pipeline::{
    Uniforms, create_bind_group, create_bind_group_layout, create_cell_pipeline,
};
pub(crate) use resources::{
    create_instance_buffer, create_quad_buffer, create_sampler, create_uniform_buffer,
};
pub(crate) use types::CellInstance;
