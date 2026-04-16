//! Autocomplete popup rendering.
//!
//! Draws a floating list of candidates above the input bar cursor,
//! using the `popup_instances` vec. Selected row gets a highlight bg.

use crate::core::CellInstance;
use crate::renderer::Renderer;
use flux_types::Color;

/// Candidate kind for popup rendering — determines the text color.
#[derive(Copy, Clone, Debug)]
pub enum PopupKind {
    Directory,
    File,
    Symlink,
    Other,
}

impl Renderer {
    /// Render the autocomplete popup above the cursor.
    ///
    /// `candidates` is the visible list of `(name, kind)` pairs.
    /// `selected` is the highlighted index within that list.
    /// `anchor_col` is the column in the input bar (including prefix).
    /// `anchor_row_y` is the pixel Y of the cursor's editor row.
    pub fn set_autocomplete_popup(
        &mut self,
        candidates: &[(String, PopupKind)],
        selected: usize,
        anchor_col: usize,
        anchor_row_y: f32,
    ) {
        let cell_w = self.atlas.cell_width;
        let cell_h = self.atlas.cell_height;
        let pad_x = self.padding_x;
        let baseline = self.atlas.baseline_offset;
        let style = self.default_style;

        let mut instances = std::mem::take(&mut self.popup_instances);
        instances.clear();

        if candidates.is_empty() {
            self.popup_instances = instances;
            self.rebuild_combined_buffer();
            return;
        }

        // Popup width: widest candidate + 2 cols padding, clamped 16..40.
        let width_cols = candidates
            .iter()
            .map(|(name, _)| name.chars().count() + 2)
            .max()
            .unwrap_or(20)
            .clamp(16, 40);

        let popup_row_count = candidates.len();
        let popup_top_y = anchor_row_y - (popup_row_count as f32) * cell_h;
        let popup_left_x = pad_x + (anchor_col as f32) * cell_w;

        let bg_normal = Color::from_hex("#1f2335").unwrap_or_default();
        let bg_selected = Color::from_hex("#3b4261").unwrap_or_default();
        let fg_dir = Color::from_hex("#7aa2f7").unwrap_or_default();
        let fg_file = Color::from_hex("#c0caf5").unwrap_or_default();
        let fg_symlink = Color::from_hex("#bb9af7").unwrap_or_default();

        for (row_idx, (name, kind)) in candidates.iter().enumerate() {
            let y = popup_top_y + (row_idx as f32) * cell_h;
            let is_selected = row_idx == selected;
            let bg = if is_selected { bg_selected } else { bg_normal };

            // Full-row background.
            instances.push(CellInstance {
                position: [popup_left_x, y],
                size: [(width_cols as f32) * cell_w, cell_h],
                glyph_uv: [0.0, 0.0, 0.0, 0.0],
                fg_color: [bg.r, bg.g, bg.b, bg.a],
                bg_color: [bg.r, bg.g, bg.b, bg.a],
            });

            let fg = match kind {
                PopupKind::Directory => fg_dir,
                PopupKind::File => fg_file,
                PopupKind::Symlink => fg_symlink,
                PopupKind::Other => fg_file,
            };

            // Candidate name, 1-cell left padding.
            for (i, ch) in name.chars().enumerate() {
                if i >= width_cols - 2 {
                    break;
                }
                let x = popup_left_x + ((i + 1) as f32) * cell_w;
                if ch != ' ' {
                    self.render_glyph(ch, style, x, y, baseline, fg, bg, &mut instances);
                }
            }
        }

        self.popup_instances = instances;
        self.rebuild_combined_buffer();
    }

    /// Hide the autocomplete popup.
    pub fn hide_autocomplete_popup(&mut self) {
        if !self.popup_instances.is_empty() {
            self.popup_instances.clear();
            self.rebuild_combined_buffer();
        }
    }
}
