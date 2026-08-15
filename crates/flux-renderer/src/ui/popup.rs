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

    /// Render the search bar (F14) in the top-right corner of the
    /// content area: `🔍 query▏  n/N`. Reuses `popup_instances` — the
    /// autocomplete popup and the search bar never show together.
    pub fn set_search_bar(&mut self, query: &str, position: Option<usize>, total: usize) {
        let cell_w = self.atlas.cell_width;
        let cell_h = self.atlas.cell_height;
        let baseline = self.atlas.baseline_offset;
        let style = self.default_style;
        let window_w = self.gpu.surface_config.width as f32;
        let top_y = self.content_top + self.padding_y * 0.5;

        let mut instances = std::mem::take(&mut self.popup_instances);
        instances.clear();

        let status = if total == 0 {
            if query.is_empty() {
                String::new()
            } else {
                "no matches".to_string()
            }
        } else {
            match position {
                Some(p) => format!("{p}/{total}"),
                None => format!("{total}"),
            }
        };
        // ` ⌕ query▏  status ` — the caret is a thin cell after the query.
        let text = format!(" ⌕ {query}▏  {status} ");
        let width_cols = text.chars().count().max(24);
        let left_x = (window_w - self.padding_x - width_cols as f32 * cell_w).max(0.0);

        let bg = Color::from_hex("#1f2335").unwrap_or_default();
        let fg = Color::from_hex("#c0caf5").unwrap_or_default();
        let accent = Color::from_hex("#e0af68").unwrap_or_default();
        let dim = Color::from_hex("#6a7099").unwrap_or_default();

        instances.push(CellInstance {
            position: [left_x, top_y],
            size: [width_cols as f32 * cell_w, cell_h],
            glyph_uv: [0.0, 0.0, 0.0, 0.0],
            fg_color: [bg.r, bg.g, bg.b, bg.a],
            bg_color: [bg.r, bg.g, bg.b, bg.a],
        });
        // Accent stripe on the left edge.
        instances.push(CellInstance {
            position: [left_x, top_y],
            size: [2.0, cell_h],
            glyph_uv: [0.0, 0.0, 0.0, 0.0],
            fg_color: [accent.r, accent.g, accent.b, accent.a],
            bg_color: [accent.r, accent.g, accent.b, accent.a],
        });

        let query_len = query.chars().count();
        for (i, ch) in text.chars().enumerate() {
            if ch == ' ' {
                continue;
            }
            // Positions: 1 = magnifier, 3..3+len = query, then caret, then status.
            let color = if i == 1 {
                accent
            } else if i > 3 + query_len {
                dim
            } else {
                fg
            };
            let x = left_x + i as f32 * cell_w;
            self.render_glyph(ch, style, x, top_y, baseline, color, bg, &mut instances);
        }

        self.popup_instances = instances;
        self.rebuild_combined_buffer();
    }

    /// Show a one-line notice centered near the top of the content area
    /// (close confirmations and the like). Own instance list, so it can
    /// coexist with the search bar / autocomplete.
    pub fn set_notice(&mut self, text: &str) {
        let cell_w = self.atlas.cell_width;
        let cell_h = self.atlas.cell_height;
        let baseline = self.atlas.baseline_offset;
        let style = self.default_style;
        let window_w = self.gpu.surface_config.width as f32;
        let top_y = self.content_top + self.padding_y * 0.5;

        let mut instances = std::mem::take(&mut self.notice_instances);
        instances.clear();

        let text = format!(" {text} ");
        let width = text.chars().count() as f32 * cell_w;
        let left_x = ((window_w - width) * 0.5).max(0.0);
        let bg = Color::from_hex("#3b2f2f").unwrap_or_default();
        let fg = Color::from_hex("#f7768e").unwrap_or_default();
        instances.push(CellInstance {
            position: [left_x, top_y],
            size: [width, cell_h],
            glyph_uv: [0.0, 0.0, 0.0, 0.0],
            fg_color: [bg.r, bg.g, bg.b, bg.a],
            bg_color: [bg.r, bg.g, bg.b, bg.a],
        });
        for (i, ch) in text.chars().enumerate() {
            if ch == ' ' {
                continue;
            }
            let x = left_x + i as f32 * cell_w;
            self.render_glyph(ch, style, x, top_y, baseline, fg, bg, &mut instances);
        }
        self.notice_instances = instances;
        self.rebuild_combined_buffer();
    }

    pub fn hide_notice(&mut self) {
        if !self.notice_instances.is_empty() {
            self.notice_instances.clear();
            self.rebuild_combined_buffer();
        }
    }

    /// Hide the autocomplete popup.
    pub fn hide_autocomplete_popup(&mut self) {
        if !self.popup_instances.is_empty() {
            self.popup_instances.clear();
            self.rebuild_combined_buffer();
        }
    }
}
