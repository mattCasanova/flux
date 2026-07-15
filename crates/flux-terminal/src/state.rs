//! Wraps alacritty_terminal::Term with a clean interface.
//!
//! Feeds PTY bytes through the vte parser into Term, then converts
//! the grid to a TerminalGrid for the renderer.

use std::sync::mpsc;

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::Config as TermConfig;
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte;
use flux_types::{CellData, CellFlags, Color, ResolvedTheme, TerminalGrid};

/// How a mouse gesture groups cells — mapped onto alacritty's
/// selection machinery (which anchors to CONTENT in absolute
/// scrollback coordinates, so selections survive scrolling and can
/// span far more than one screen).
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum SelectMode {
    /// Cell-by-cell (click + drag).
    Char,
    /// Word-snapped (double-click).
    Word,
    /// Whole lines (triple-click).
    Line,
    /// Rectangular (Alt+drag).
    Block,
}

impl SelectMode {
    fn to_alacritty(self) -> SelectionType {
        match self {
            SelectMode::Char => SelectionType::Simple,
            SelectMode::Word => SelectionType::Semantic,
            SelectMode::Line => SelectionType::Lines,
            SelectMode::Block => SelectionType::Block,
        }
    }
}

use crate::blocks::{BlockCapture, ShellPhase};

/// Events that alacritty_terminal sends back (bell, title change, etc.)
#[derive(Debug)]
pub enum TermEvent {
    /// Write these bytes back to the PTY (terminal query responses).
    PtyWrite(String),
    /// Terminal bell.
    Bell,
    /// Window title changed.
    Title(String),
}

/// Event listener that captures alacritty_terminal events via channel.
struct EventProxy {
    tx: mpsc::Sender<TermEvent>,
}

impl EventListener for EventProxy {
    fn send_event(&self, event: Event) {
        match event {
            Event::PtyWrite(text) => {
                let _ = self.tx.send(TermEvent::PtyWrite(text));
            }
            Event::Title(title) => {
                let _ = self.tx.send(TermEvent::Title(title));
            }
            Event::Bell => {
                let _ = self.tx.send(TermEvent::Bell);
            }
            _ => {}
        }
    }
}

/// Terminal dimensions for alacritty_terminal.
struct TermDimensions {
    cols: usize,
    rows: usize,
}

impl Dimensions for TermDimensions {
    fn total_lines(&self) -> usize {
        self.rows
    }

    fn screen_lines(&self) -> usize {
        self.rows
    }

    fn columns(&self) -> usize {
        self.cols
    }

    fn last_column(&self) -> alacritty_terminal::index::Column {
        alacritty_terminal::index::Column(self.cols.saturating_sub(1))
    }

    fn topmost_line(&self) -> alacritty_terminal::index::Line {
        alacritty_terminal::index::Line(0)
    }

    fn bottommost_line(&self) -> alacritty_terminal::index::Line {
        alacritty_terminal::index::Line(self.rows as i32 - 1)
    }
}

/// Wraps alacritty_terminal with a clean API.
pub struct TerminalState {
    term: Term<EventProxy>,
    parser: vte::ansi::Processor,
    /// Side-channel OSC interceptor. See `blocks.rs` for the full
    /// rationale — in short, alacritty's ansi layer drops OSC 7 and
    /// OSC 133 before they reach `Term`, so we run a second parser
    /// over the same byte stream to catch them.
    block_capture: BlockCapture,
    /// Stock vte parser driving `block_capture`. Independent state
    /// machine from `parser` — both see the exact same `&[u8]` but
    /// neither affects the other.
    block_parser: vte::Parser,
    /// Resolved color palette for named/indexed ANSI colors.
    theme: ResolvedTheme,
    event_rx: mpsc::Receiver<TermEvent>,
    cols: usize,
    rows: usize,
}

impl TerminalState {
    /// Create a new terminal state with the given dimensions,
    /// scrollback capacity in lines, and resolved color theme.
    pub fn new(cols: usize, rows: usize, scrollback_lines: usize, theme: ResolvedTheme) -> Self {
        let (tx, rx) = mpsc::channel();
        let event_proxy = EventProxy { tx };

        let config = TermConfig {
            scrolling_history: scrollback_lines,
            ..TermConfig::default()
        };
        let dims = TermDimensions { cols, rows };
        let term = Term::new(config, &dims, event_proxy);
        let parser = vte::ansi::Processor::new();

        Self {
            term,
            parser,
            theme,
            block_capture: BlockCapture::new(),
            block_parser: vte::Parser::new(),
            event_rx: rx,
            cols,
            rows,
        }
    }

    /// Feed raw PTY output bytes into the terminal parser.
    ///
    /// Two parsers run in parallel over the same byte slice:
    /// - `self.parser.advance(&mut self.term, bytes)` is the main
    ///   path — it drives alacritty's grid, cursor, and scrollback.
    ///   Unchanged from before R3.
    /// - `self.block_parser.advance(&mut self.block_capture, bytes)`
    ///   is the side path — a stock `vte::Parser` with its own
    ///   state machine, feeding a `Perform` impl that exists only
    ///   to intercept OSC 7 (cwd) and OSC 133 (prompt/exit). R3
    ///   lands this as a no-op foundation; F4/F8 add the real
    ///   handling.
    ///
    /// The two parsers are independent — feeding bytes to one does
    /// not affect the other's state. Running both is sub-microsecond
    /// per KB (see module docs for the perf rationale).
    pub fn process_bytes(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
        self.block_parser.advance(&mut self.block_capture, bytes);
    }

    pub fn cols(&self) -> usize {
        self.cols
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    /// Scroll the display by `lines` (positive = up into history,
    /// negative = down towards the live tail). Alacritty clamps at
    /// both ends, so overshooting is a no-op.
    pub fn scroll_lines(&mut self, lines: i32) {
        self.term.scroll_display(Scroll::Delta(lines));
    }

    pub fn scroll_page_up(&mut self) {
        self.term.scroll_display(Scroll::PageUp);
    }

    pub fn scroll_page_down(&mut self) {
        self.term.scroll_display(Scroll::PageDown);
    }

    pub fn scroll_to_bottom(&mut self) {
        self.term.scroll_display(Scroll::Bottom);
    }

    /// Current history offset in lines. 0 = tailing live output;
    /// positive = the user has scrolled that many lines into history.
    /// When new output arrives while scrolled up, alacritty grows the
    /// offset internally so the viewport doesn't jump — no gate needed
    /// on our side.
    pub fn display_offset(&self) -> usize {
        self.term.grid().display_offset()
    }

    /// Convert a viewport cell to alacritty grid coordinates (absolute
    /// within the visible+history window): grid line = viewport row −
    /// display offset.
    fn viewport_to_point(&self, col: usize, row: usize) -> Point {
        let line = Line(row as i32 - self.display_offset() as i32);
        Point::new(line, Column(col.min(self.cols.saturating_sub(1))))
    }

    /// Begin a selection at a viewport cell. `right_side` picks the
    /// half of the cell the pointer landed in (char-precise edges).
    pub fn start_selection(&mut self, mode: SelectMode, col: usize, row: usize, right_side: bool) {
        let point = self.viewport_to_point(col, row);
        let side = if right_side { Side::Right } else { Side::Left };
        self.term.selection = Some(Selection::new(mode.to_alacritty(), point, side));
    }

    /// Extend the active selection to a viewport cell (drag /
    /// Shift+click).
    pub fn update_selection(&mut self, col: usize, row: usize, right_side: bool) {
        let point = self.viewport_to_point(col, row);
        let side = if right_side { Side::Right } else { Side::Left };
        if let Some(selection) = &mut self.term.selection {
            selection.update(point, side);
        }
    }

    pub fn clear_terminal_selection(&mut self) {
        self.term.selection = None;
    }

    pub fn has_selection(&self) -> bool {
        self.term.selection.is_some()
    }

    /// The selected text, across scrollback if the selection spans it.
    /// None when there's no selection or it's empty (a click that
    /// never dragged).
    pub fn selection_text(&self) -> Option<String> {
        self.term.selection_to_string().filter(|s| !s.is_empty())
    }

    /// Drain any events from alacritty_terminal (PtyWrite, Bell, Title).
    pub fn drain_events(&self) -> Vec<TermEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.event_rx.try_recv() {
            events.push(event);
        }
        events
    }

    /// Snapshot the current terminal grid for rendering.
    pub fn grid_snapshot(&self) -> TerminalGrid {
        let content = self.term.renderable_content();
        let mut grid = TerminalGrid::new(self.cols, self.rows);
        // Alacritty's display_iter yields points in GRID coordinates,
        // where scrolled-into-history rows have NEGATIVE line numbers
        // (line 0 is the top of the live screen, -1 the line above it).
        // Viewport row = grid line + display_offset. Getting this wrong
        // renders a scrolled view as blank — regression-tested below.
        let display_offset = content.display_offset as i32;
        grid.display_offset = content.display_offset;

        // Set cursor position (scrolled up, the cursor converts to a row
        // at/below the viewport bottom and is culled by the bounds check).
        let cursor_point = content.cursor.point;
        let cursor_col = cursor_point.column.0;
        let cursor_row = cursor_point.line.0 + display_offset;
        if cursor_col < self.cols && (0..self.rows as i32).contains(&cursor_row) {
            grid.cursor = Some((cursor_col, cursor_row as usize));
        }

        // Selection range in grid coordinates — alacritty resolves the
        // content-anchored selection against the current viewport.
        let selection_range = content.selection;

        for cell in content.display_iter {
            let col = cell.point.column.0;
            let row_i = cell.point.line.0 + display_offset;
            if row_i < 0 {
                continue;
            }
            let row = row_i as usize;

            if col >= self.cols || row >= self.rows {
                continue;
            }

            let selected = selection_range
                .map(|range| range.contains(cell.point))
                .unwrap_or(false);

            let fg = self.convert_color(cell.fg);
            let bg = self.convert_color(cell.bg);

            let mut flags = CellFlags::empty();
            if selected {
                flags |= CellFlags::SELECTION;
            }
            if cell
                .flags
                .contains(alacritty_terminal::term::cell::Flags::BOLD)
            {
                flags |= CellFlags::BOLD;
            }
            if cell
                .flags
                .contains(alacritty_terminal::term::cell::Flags::ITALIC)
            {
                flags |= CellFlags::ITALIC;
            }
            if cell
                .flags
                .contains(alacritty_terminal::term::cell::Flags::UNDERLINE)
            {
                flags |= CellFlags::UNDERLINE;
            }
            if cell
                .flags
                .contains(alacritty_terminal::term::cell::Flags::HIDDEN)
            {
                flags |= CellFlags::HIDDEN;
            }
            if cell
                .flags
                .contains(alacritty_terminal::term::cell::Flags::DIM_BOLD)
            {
                flags |= CellFlags::DIM;
            }
            if cell
                .flags
                .contains(alacritty_terminal::term::cell::Flags::WIDE_CHAR)
            {
                flags |= CellFlags::WIDE_CHAR;
            }

            grid.set(
                row,
                col,
                CellData {
                    character: cell.c,
                    fg,
                    bg,
                    flags,
                },
            );
        }

        grid
    }

    /// The shell's current working directory, if known via OSC 7.
    /// Returns `None` until the shell emits its first OSC 7 sequence.
    pub fn cwd(&self) -> Option<&std::path::Path> {
        self.block_capture.cwd()
    }

    /// Exit code of the last finished command, if known via OSC 133;D.
    pub fn last_exit_code(&self) -> Option<i32> {
        self.block_capture.last_exit_code()
    }

    /// True when the program on the other end of the PTY is using the
    /// alternate screen buffer — vim, less, man, htop, tmux all set this
    /// bit. It's the single most reliable signal that the user is in a
    /// full-screen program that owns the keyboard.
    pub fn is_alt_screen(&self) -> bool {
        self.term.mode().contains(TermMode::ALT_SCREEN)
    }

    /// True when the child program has enabled bracketed paste mode — the
    /// terminal should wrap pasted text in `\x1b[200~` / `\x1b[201~` so the
    /// program can distinguish it from typed input. Most shells and vim
    /// enable this by default.
    pub fn is_bracketed_paste(&self) -> bool {
        self.term.mode().contains(TermMode::BRACKETED_PASTE)
    }

    /// True when the child program has requested xterm mouse reporting
    /// (vim with `mouse=a`, htop, …). Local mouse selection defers to
    /// the program in that case.
    pub fn wants_mouse_reporting(&self) -> bool {
        self.term.mode().intersects(TermMode::MOUSE_MODE)
    }

    /// True when wheel events over the alt screen should be translated
    /// to arrow keys (DECSET 1007 — on by default in alacritty's mode,
    /// cleared by programs that want raw wheel control).
    pub fn alternate_scroll(&self) -> bool {
        self.term.mode().contains(TermMode::ALTERNATE_SCROLL)
    }

    /// True when the program requested the SGR mouse encoding
    /// (DECSET 1006) — modern programs all do; the legacy `\x1b[M`
    /// byte encoding is the fallback.
    pub fn sgr_mouse(&self) -> bool {
        self.term.mode().contains(TermMode::SGR_MOUSE)
    }

    /// True when the program wants drag events while a button is held
    /// (DECSET 1002 button-event or 1003 any-event tracking).
    pub fn reports_mouse_drag(&self) -> bool {
        self.term
            .mode()
            .intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION)
    }

    /// True when the program requested hover motion with no button held
    /// (DECSET 1003 any-event tracking only — Claude Code enables this).
    pub fn reports_mouse_motion(&self) -> bool {
        self.term.mode().contains(TermMode::MOUSE_MOTION)
    }

    /// True when the application cursor-keys mode is active (DECCKM) —
    /// arrow keys must then be encoded as `\x1bOA`-style sequences.
    pub fn app_cursor_keys(&self) -> bool {
        self.term.mode().contains(TermMode::APP_CURSOR)
    }

    /// True while a command is running (between OSC 133;C and 133;D).
    /// Keyboard routing sends keys straight to the PTY during this
    /// window so interactive programs that never touch the alt screen
    /// (sudo prompts, REPLs) receive keystrokes directly.
    pub fn is_executing(&self) -> bool {
        self.block_capture.integration_active()
            && self.block_capture.shell_phase() == ShellPhase::Executing
    }

    /// Resize the terminal grid.
    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.cols = cols;
        self.rows = rows;
        let dims = TermDimensions { cols, rows };
        self.term.resize(dims);
    }

    /// Convert an alacritty color to our Color type.
    fn convert_color(&self, color: alacritty_terminal::vte::ansi::Color) -> Color {
        match color {
            alacritty_terminal::vte::ansi::Color::Named(named) => self.named_color(named),
            alacritty_terminal::vte::ansi::Color::Spec(rgb) => Color::from_rgb(rgb.r, rgb.g, rgb.b),
            alacritty_terminal::vte::ansi::Color::Indexed(idx) => self.indexed_color(idx),
        }
    }

    /// Map named ANSI colors through the resolved theme.
    fn named_color(&self, color: alacritty_terminal::vte::ansi::NamedColor) -> Color {
        use alacritty_terminal::vte::ansi::NamedColor::*;
        let t = &self.theme;
        match color {
            Black => t.ansi(0),
            Red => t.ansi(1),
            Green => t.ansi(2),
            Yellow => t.ansi(3),
            Blue => t.ansi(4),
            Magenta => t.ansi(5),
            Cyan => t.ansi(6),
            White => t.ansi(7),
            BrightBlack => t.ansi(8),
            BrightRed => t.ansi(9),
            BrightGreen => t.ansi(10),
            BrightYellow => t.ansi(11),
            BrightBlue => t.ansi(12),
            BrightMagenta => t.ansi(13),
            BrightCyan => t.ansi(14),
            BrightWhite => t.ansi(15),
            Foreground => t.foreground,
            Background => t.background,
            Cursor => t.cursor,
            _ => t.foreground,
        }
    }

    /// Map 256-color index to RGB.
    fn indexed_color(&self, idx: u8) -> Color {
        match idx {
            0..=15 => {
                // Standard colors — same as named
                use alacritty_terminal::vte::ansi::NamedColor;
                let named = match idx {
                    0 => NamedColor::Black,
                    1 => NamedColor::Red,
                    2 => NamedColor::Green,
                    3 => NamedColor::Yellow,
                    4 => NamedColor::Blue,
                    5 => NamedColor::Magenta,
                    6 => NamedColor::Cyan,
                    7 => NamedColor::White,
                    8 => NamedColor::BrightBlack,
                    9 => NamedColor::BrightRed,
                    10 => NamedColor::BrightGreen,
                    11 => NamedColor::BrightYellow,
                    12 => NamedColor::BrightBlue,
                    13 => NamedColor::BrightMagenta,
                    14 => NamedColor::BrightCyan,
                    15 => NamedColor::BrightWhite,
                    _ => unreachable!(),
                };
                self.named_color(named)
            }
            16..=231 => {
                // 216-color cube
                let idx = idx - 16;
                let r = (idx / 36) * 51;
                let g = ((idx % 36) / 6) * 51;
                let b = (idx % 6) * 51;
                Color::from_rgb(r, g, b)
            }
            232..=255 => {
                // Grayscale ramp
                let gray = 8 + (idx - 232) * 10;
                Color::from_rgb(gray, gray, gray)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// R3 smoke test — prove that `process_bytes` feeds BOTH parsers
    /// (main alacritty + side BlockCapture) without panicking, and
    /// that constructing a TerminalState works with the side parser
    /// wired in. No assertion on BlockCapture state — the side
    /// parser is a no-op in R3; F4 adds a test that verifies OSC 7
    /// actually updates `cwd`.
    #[test]
    fn block_capture_runs_alongside_main_parser() {
        let mut state = TerminalState::new(80, 24, 1000, ResolvedTheme::default());
        state.process_bytes(b"hello world\n");
        state.process_bytes(b"\x1b[31mred\x1b[0m\n");
        // Feed an OSC 7 sequence — the side parser should accept it
        // without panicking even though nothing reads the state yet.
        state.process_bytes(b"\x1b]7;file://localhost/tmp\x07");
    }

    #[test]
    fn scrollback_holds_history_and_offset_tracks_scrolling() {
        let mut state = TerminalState::new(80, 24, 1000, ResolvedTheme::default());
        // Push well past one screen of output.
        for i in 0..100 {
            state.process_bytes(format!("line {}\r\n", i).as_bytes());
        }
        assert_eq!(state.display_offset(), 0, "tailing by default");

        state.scroll_lines(10);
        assert_eq!(state.display_offset(), 10);

        // Overshoot clamps rather than panics.
        state.scroll_lines(100_000);
        let clamped = state.display_offset();
        assert!(clamped >= 10, "offset should clamp at history top");

        // New output while scrolled up must NOT move the viewport:
        // alacritty grows the offset to keep the same lines on screen.
        state.process_bytes(b"new tail line\r\n");
        assert_eq!(state.display_offset(), clamped + 1);

        state.scroll_to_bottom();
        assert_eq!(state.display_offset(), 0);

        // The snapshot carries the offset for downstream consumers.
        let grid = state.grid_snapshot();
        assert_eq!(grid.display_offset, 0);
    }

    /// Regression test for the "scrolled view renders black" bug:
    /// display_iter points are grid coords where history rows are
    /// NEGATIVE lines; the snapshot must convert to viewport rows.
    #[test]
    fn scrolled_snapshot_shows_history_content() {
        let mut state = TerminalState::new(80, 24, 1000, ResolvedTheme::default());
        for i in 0..30 {
            state.process_bytes(format!("line {}\r\n", i).as_bytes());
        }

        // Tailing: 31 logical rows, viewport shows rows 7.. => top = "line 7".
        let grid = state.grid_snapshot();
        let top: String = (0..7).map(|c| grid.get(0, c).character).collect();
        assert_eq!(top.trim_end(), "line 7");

        // Scroll 7 up: top of the viewport must show "line 0" — before
        // the coordinate fix this row came back blank.
        state.scroll_lines(7);
        let grid = state.grid_snapshot();
        let top: String = (0..7).map(|c| grid.get(0, c).character).collect();
        assert_eq!(top.trim_end(), "line 0");
        // The live cursor is below the scrolled viewport — hidden.
        assert_eq!(grid.cursor, None);
    }

    #[test]
    fn executing_phase_gates_on_osc_133() {
        let mut state = TerminalState::new(80, 24, 100, ResolvedTheme::default());
        assert!(!state.is_executing(), "no integration yet");
        state.process_bytes(b"\x1b]133;A\x07");
        assert!(!state.is_executing(), "at prompt");
        state.process_bytes(b"\x1b]133;C\x07");
        assert!(state.is_executing(), "command running");
        state.process_bytes(b"\x1b]133;D;0\x07");
        assert!(!state.is_executing(), "command finished");
    }

    /// The dogfood finding that forced content-anchored selection: a
    /// drag that autoscrolls must yield MORE text than one screen, and
    /// the selection must survive scrolling instead of being cleared.
    #[test]
    fn selection_survives_scrolling_and_spans_scrollback() {
        let mut state = TerminalState::new(80, 24, 1000, ResolvedTheme::default());
        for i in 0..60 {
            state.process_bytes(format!("line {}\r\n", i).as_bytes());
        }

        // The dogfood gesture: scroll up into history to the top of
        // the output first...
        state.scroll_lines(20);
        // ...anchor at the top visible row...
        state.start_selection(SelectMode::Char, 0, 0, false);
        // ...drag to the bottom edge...
        state.update_selection(79, 23, true);
        // ...and drag-autoscroll back toward the tail, re-pinning the
        // head to the bottom edge each step. The anchor stays glued to
        // its content up in history, so the selection grows.
        state.scroll_lines(-10);
        state.update_selection(79, 23, true);

        let text = state.selection_text().expect("selection has text");
        let lines: Vec<&str> = text.lines().collect();
        assert!(
            lines.len() > 24,
            "selection spans more than one screen: got {} lines",
            lines.len()
        );

        // Still selected after further scrolling — content-anchored.
        state.scroll_lines(5);
        assert!(state.selection_text().is_some());

        // And the snapshot carries SELECTION flags for visible cells.
        let grid = state.grid_snapshot();
        let any_selected = (0..grid.rows)
            .any(|r| (0..grid.cols).any(|c| grid.get(r, c).flags.contains(CellFlags::SELECTION)));
        assert!(any_selected, "highlight flags present in the viewport");
    }

    #[test]
    fn empty_click_selection_yields_no_text() {
        let mut state = TerminalState::new(80, 24, 100, ResolvedTheme::default());
        state.process_bytes(b"hello\r\n");
        state.start_selection(SelectMode::Char, 2, 0, false);
        // No drag — degenerate selection copies nothing.
        assert_eq!(state.selection_text(), None);
    }

    #[test]
    fn word_selection_snaps_to_boundaries() {
        let mut state = TerminalState::new(80, 24, 100, ResolvedTheme::default());
        state.process_bytes(b"alpha bravo charlie\r\n");
        // Double-click semantics: land mid-"bravo".
        state.start_selection(SelectMode::Word, 8, 0, false);
        assert_eq!(state.selection_text().as_deref(), Some("bravo"));
    }

    #[test]
    fn zero_scrollback_keeps_offset_pinned() {
        let mut state = TerminalState::new(80, 24, 0, ResolvedTheme::default());
        for i in 0..50 {
            state.process_bytes(format!("line {}\r\n", i).as_bytes());
        }
        state.scroll_lines(10);
        assert_eq!(state.display_offset(), 0, "no history to scroll into");
    }
}
