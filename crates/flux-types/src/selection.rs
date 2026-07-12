//! Grid-cell selection for copy-to-clipboard.
//!
//! A `Selection` is an anchored range: `anchor` is where the user
//! pressed mouse-down, `head` is where they're currently dragging to
//! (or released). Either may come before or after the other in reading
//! order — `sorted()` normalizes when iterating or extracting.
//!
//! `mode` determines how cells are grouped:
//! - `Character`: cell-by-cell linear range (wraps across rows)
//! - `Word`: like Character, but anchor/head snap to word boundaries
//! - `Line`: whole rows from anchor row to head row
//! - `Block`: rectangular — both col and row bounded

use crate::TerminalGrid;

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct CellPos {
    pub col: usize,
    pub row: usize,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SelectionMode {
    Character,
    Word,
    Line,
    Block,
}

#[derive(Copy, Clone, Debug)]
pub struct Selection {
    pub anchor: CellPos,
    pub head: CellPos,
    pub mode: SelectionMode,
}

impl Selection {
    pub fn new(pos: CellPos, mode: SelectionMode) -> Self {
        Self {
            anchor: pos,
            head: pos,
            mode,
        }
    }

    /// Extend the selection to a new head position (mouse drag or
    /// Shift+click).
    pub fn extend_to(&mut self, pos: CellPos) {
        self.head = pos;
    }

    /// Normalize anchor/head so the first result precedes the second in
    /// reading order.
    pub fn sorted(&self) -> (CellPos, CellPos) {
        let (a, b) = (self.anchor, self.head);
        if (a.row, a.col) <= (b.row, b.col) {
            (a, b)
        } else {
            (b, a)
        }
    }

    /// True if the selection covers no horizontal or vertical extent.
    /// A single-cell selection still copies that cell, so "empty" here
    /// only means "never extended" — the click-without-drag case.
    pub fn is_degenerate(&self) -> bool {
        self.anchor == self.head && matches!(self.mode, SelectionMode::Character)
    }

    /// Snap anchor and head to word boundaries (whitespace-delimited).
    /// Used by Word mode on double-click and while dragging.
    pub fn snap_to_words(&mut self, grid: &TerminalGrid) {
        if self.mode != SelectionMode::Word {
            return;
        }
        let (start, end) = self.sorted();
        let new_start = word_start(grid, start);
        let new_end = word_end(grid, end);
        self.anchor = new_start;
        self.head = new_end;
    }

    /// Extract the text content of the selection from a grid. Rows are
    /// joined with `\n`; trailing spaces on each row are trimmed
    /// (standard terminal copy behavior).
    pub fn text(&self, grid: &TerminalGrid) -> String {
        let mut out = String::new();
        if grid.cols == 0 || grid.rows == 0 {
            return out;
        }
        match self.mode {
            // Word is snapped to boundaries by the mouse handler, so
            // extraction is identical to Character.
            SelectionMode::Character | SelectionMode::Word => self.text_character(grid, &mut out),
            SelectionMode::Line => self.text_lines(grid, &mut out),
            SelectionMode::Block => self.text_block(grid, &mut out),
        }
        out
    }

    fn text_character(&self, grid: &TerminalGrid, out: &mut String) {
        let (start, end) = self.clamped(grid);
        for row in start.row..=end.row {
            let col_start = if row == start.row { start.col } else { 0 };
            let col_end = if row == end.row {
                end.col + 1
            } else {
                grid.cols
            };
            let mut line = String::new();
            for col in col_start..col_end.min(grid.cols) {
                line.push(printable(grid.get(row, col).character));
            }
            out.push_str(line.trim_end());
            if row < end.row {
                out.push('\n');
            }
        }
    }

    fn text_lines(&self, grid: &TerminalGrid, out: &mut String) {
        let (start, end) = self.clamped(grid);
        for row in start.row..=end.row {
            let mut line = String::new();
            for col in 0..grid.cols {
                line.push(printable(grid.get(row, col).character));
            }
            out.push_str(line.trim_end());
            if row < end.row {
                out.push('\n');
            }
        }
    }

    fn text_block(&self, grid: &TerminalGrid, out: &mut String) {
        let (start, end) = self.clamped(grid);
        let col_start = start.col.min(end.col);
        let col_end = start.col.max(end.col);
        for row in start.row..=end.row {
            let mut line = String::new();
            for col in col_start..=col_end.min(grid.cols - 1) {
                line.push(printable(grid.get(row, col).character));
            }
            out.push_str(line.trim_end());
            if row < end.row {
                out.push('\n');
            }
        }
    }

    /// Sorted endpoints clamped inside the grid so extraction can't
    /// index out of bounds even if the viewport shrank after the
    /// selection was made.
    fn clamped(&self, grid: &TerminalGrid) -> (CellPos, CellPos) {
        let (mut start, mut end) = self.sorted();
        let max_row = grid.rows - 1;
        let max_col = grid.cols - 1;
        start.row = start.row.min(max_row);
        start.col = start.col.min(max_col);
        end.row = end.row.min(max_row);
        end.col = end.col.min(max_col);
        (start, end)
    }

    /// Iterate all cells in the selection (for highlight rendering).
    /// `cols` is the grid width so linear modes wrap rows correctly.
    pub fn cells(&self, cols: usize) -> impl Iterator<Item = CellPos> + '_ {
        let (start, end) = self.sorted();
        let mode = self.mode;
        let (col_lo, col_hi) = (start.col.min(end.col), start.col.max(end.col));
        (start.row..=end.row).flat_map(move |row| {
            let (from, to) = match mode {
                SelectionMode::Character | SelectionMode::Word => {
                    let from = if row == start.row { start.col } else { 0 };
                    let to = if row == end.row { end.col + 1 } else { cols };
                    (from, to.min(cols))
                }
                SelectionMode::Line => (0, cols),
                SelectionMode::Block => (col_lo, (col_hi + 1).min(cols)),
            };
            (from..to).map(move |col| CellPos { col, row })
        })
    }
}

/// Treat NUL (uninitialized cells) as space for copy purposes.
fn printable(c: char) -> char {
    if c == '\0' { ' ' } else { c }
}

fn is_word_char(c: char) -> bool {
    !c.is_whitespace() && c != '\0'
}

fn word_start(grid: &TerminalGrid, pos: CellPos) -> CellPos {
    let row = pos.row.min(grid.rows - 1);
    let mut col = pos.col.min(grid.cols - 1);
    if !is_word_char(grid.get(row, col).character) {
        return CellPos { col, row };
    }
    while col > 0 && is_word_char(grid.get(row, col - 1).character) {
        col -= 1;
    }
    CellPos { col, row }
}

fn word_end(grid: &TerminalGrid, pos: CellPos) -> CellPos {
    let row = pos.row.min(grid.rows - 1);
    let mut col = pos.col.min(grid.cols - 1);
    if !is_word_char(grid.get(row, col).character) {
        return CellPos { col, row };
    }
    while col + 1 < grid.cols && is_word_char(grid.get(row, col + 1).character) {
        col += 1;
    }
    CellPos { col, row }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CellData;

    /// Build a grid from string rows (padded with spaces to the widest).
    fn grid_from(rows: &[&str]) -> TerminalGrid {
        let cols = rows.iter().map(|r| r.chars().count()).max().unwrap_or(1);
        let mut grid = TerminalGrid::new(cols, rows.len());
        for (r, row) in rows.iter().enumerate() {
            for (c, ch) in row.chars().enumerate() {
                grid.set(
                    r,
                    c,
                    CellData {
                        character: ch,
                        ..CellData::default()
                    },
                );
            }
        }
        grid
    }

    fn sel(anchor: (usize, usize), head: (usize, usize), mode: SelectionMode) -> Selection {
        Selection {
            anchor: CellPos {
                col: anchor.0,
                row: anchor.1,
            },
            head: CellPos {
                col: head.0,
                row: head.1,
            },
            mode,
        }
    }

    #[test]
    fn character_selection_single_row() {
        let grid = grid_from(&["hello world"]);
        let s = sel((6, 0), (10, 0), SelectionMode::Character);
        assert_eq!(s.text(&grid), "world");
    }

    #[test]
    fn character_selection_reversed_drag() {
        let grid = grid_from(&["hello world"]);
        // Dragged right-to-left: head before anchor.
        let s = sel((10, 0), (6, 0), SelectionMode::Character);
        assert_eq!(s.text(&grid), "world");
    }

    #[test]
    fn character_selection_multi_row_trims_trailing_spaces() {
        let grid = grid_from(&["first   ", "second  "]);
        let s = sel((0, 0), (5, 1), SelectionMode::Character);
        assert_eq!(s.text(&grid), "first\nsecond");
    }

    #[test]
    fn line_selection_takes_whole_rows() {
        let grid = grid_from(&["alpha beta", "gamma"]);
        let s = sel((7, 0), (2, 1), SelectionMode::Line);
        assert_eq!(s.text(&grid), "alpha beta\ngamma");
    }

    #[test]
    fn block_selection_is_rectangular() {
        let grid = grid_from(&["abcdef", "ghijkl", "mnopqr"]);
        let s = sel((1, 0), (3, 2), SelectionMode::Block);
        assert_eq!(s.text(&grid), "bcd\nhij\nnop");
    }

    #[test]
    fn word_snap_expands_to_boundaries() {
        let grid = grid_from(&["hello brave world"]);
        let mut s = sel((8, 0), (8, 0), SelectionMode::Word);
        s.snap_to_words(&grid);
        assert_eq!(s.text(&grid), "brave");
    }

    #[test]
    fn word_snap_on_whitespace_stays_put() {
        let grid = grid_from(&["hello world"]);
        let mut s = sel((5, 0), (5, 0), SelectionMode::Word);
        s.snap_to_words(&grid);
        assert_eq!(s.text(&grid), "");
    }

    #[test]
    fn out_of_bounds_endpoints_clamp() {
        let grid = grid_from(&["short"]);
        let s = sel((0, 0), (100, 5), SelectionMode::Character);
        assert_eq!(s.text(&grid), "short");
    }

    #[test]
    fn cells_iterator_wraps_rows() {
        let s = sel((2, 0), (1, 1), SelectionMode::Character);
        let cells: Vec<(usize, usize)> = s.cells(4).map(|p| (p.col, p.row)).collect();
        assert_eq!(cells, vec![(2, 0), (3, 0), (0, 1), (1, 1)]);
    }

    #[test]
    fn nul_cells_copy_as_spaces_then_trim() {
        let mut grid = TerminalGrid::new(4, 1);
        grid.set(
            0,
            0,
            CellData {
                character: 'a',
                ..CellData::default()
            },
        );
        // An explicit NUL (as alacritty leaves in untouched cells) must
        // not leak into the clipboard.
        grid.set(
            0,
            1,
            CellData {
                character: '\0',
                ..CellData::default()
            },
        );
        let s = sel((0, 0), (3, 0), SelectionMode::Character);
        assert_eq!(s.text(&grid), "a");
    }
}
