//! Instance buffer management for the renderer.
//!
//! Owns the rule that "output + input + (future) selection + popup
//! instances get concatenated into one GPU buffer and drawn in one
//! call". Also owns the `color_matches` helper used by `set_grid`
//! to decide whether a cell's bg rect needs to be emitted at all.
//!
//! The `rebuild_combined_buffer` method lives here as an `impl Renderer`
//! block — same pattern every sibling module uses (see Phase -1 D1).

use crate::cell_renderer::CellInstance;
use crate::renderer::Renderer;
use flux_types::Color;

/// Starting capacity for the instance buffer. Enough for a 200×60 grid
/// without reallocating. Grows on demand in `rebuild_combined_buffer`.
pub(crate) const INITIAL_MAX_CELLS: usize = 200 * 60;

/// Approximate color equality — used to skip painting cell backgrounds that
/// match the window clear color, since those are already filled by the
/// render pass' clear step.
pub(crate) fn color_matches(a: Color, b: Color) -> bool {
    const EPS: f32 = 1.0 / 512.0;
    (a.r - b.r).abs() < EPS
        && (a.g - b.g).abs() < EPS
        && (a.b - b.b).abs() < EPS
        && (a.a - b.a).abs() < EPS
}

impl Renderer {
    /// Rebuild the combined instance buffer from the persistent
    /// output / input / popup vecs.
    ///
    /// Paint order: **output → input → popup**. This is the draw
    /// order inside a single `draw(0..4, 0..instance_count)` call,
    /// which means later slices render on top of earlier ones.
    /// Popups (autocomplete from F7, search overlay from F14) must
    /// be visible over the input line and the terminal grid, so
    /// they sit at the end. R4 wires the popup slice through the
    /// buffer layout so F7/F14 only have to populate
    /// `popup_instances` — no further plumbing required.
    ///
    /// Grows the GPU buffer if the combined size exceeds current
    /// capacity.
    pub(crate) fn rebuild_combined_buffer(&mut self) {
        let total = self.output_instances.len()
            + self.input_instances.len()
            + self.popup_instances.len();
        if total == 0 {
            self.instance_count = 0;
            return;
        }

        if total > self.instance_capacity {
            self.instance_capacity = total * 2;
            self.instance_buffer =
                crate::gpu_resources::create_instance_buffer(&self.gpu.device, self.instance_capacity);
            log::info!("Grew instance buffer to {} cells", self.instance_capacity);
        }

        let mut offset: u64 = 0;
        if !self.output_instances.is_empty() {
            self.gpu.queue.write_buffer(
                &self.instance_buffer,
                offset,
                bytemuck::cast_slice(&self.output_instances),
            );
            offset += (self.output_instances.len() * std::mem::size_of::<CellInstance>()) as u64;
        }

        if !self.input_instances.is_empty() {
            self.gpu.queue.write_buffer(
                &self.instance_buffer,
                offset,
                bytemuck::cast_slice(&self.input_instances),
            );
            offset += (self.input_instances.len() * std::mem::size_of::<CellInstance>()) as u64;
        }

        if !self.popup_instances.is_empty() {
            self.gpu.queue.write_buffer(
                &self.instance_buffer,
                offset,
                bytemuck::cast_slice(&self.popup_instances),
            );
        }

        self.instance_count = total as u32;
    }
}
