//! Wraps alacritty_terminal::Term with a clean interface.
//!
//! Feeds PTY bytes through the vte parser into Term, then converts
//! the grid to a TerminalGrid for the renderer.

use std::sync::mpsc;

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::term::Config as TermConfig;
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte;
use flux_types::{CellData, CellFlags, Color, TerminalGrid};

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
    event_rx: mpsc::Receiver<TermEvent>,
    cols: usize,
    rows: usize,
}

impl TerminalState {
    /// Create a new terminal state with the given dimensions and
    /// scrollback capacity in lines.
    pub fn new(cols: usize, rows: usize, scrollback_lines: usize) -> Self {
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

    /// Drain any events from alacritty_terminal (PtyWrite, Bell, Title).
    pub fn drain_events(&self) -> Vec<TermEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.event_rx.try_recv() {
            events.push(event);
        }
        events
    }

    /// Snapshot the current terminal grid for rendering.
    pub fn grid_snapshot(&self, fg_default: Color, bg_default: Color) -> TerminalGrid {
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

            let fg = self.convert_color(cell.fg, &fg_default);
            let bg = self.convert_color(cell.bg, &bg_default);

            let mut flags = CellFlags::empty();
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
    fn convert_color(
        &self,
        color: alacritty_terminal::vte::ansi::Color,
        _default: &Color,
    ) -> Color {
        match color {
            alacritty_terminal::vte::ansi::Color::Named(named) => self.named_color(named),
            alacritty_terminal::vte::ansi::Color::Spec(rgb) => Color::from_rgb(rgb.r, rgb.g, rgb.b),
            alacritty_terminal::vte::ansi::Color::Indexed(idx) => self.indexed_color(idx),
        }
    }

    /// Map named ANSI colors to Tokyo Night Storm palette.
    /// TODO: Read these from the theme file.
    fn named_color(&self, color: alacritty_terminal::vte::ansi::NamedColor) -> Color {
        use alacritty_terminal::vte::ansi::NamedColor::*;
        match color {
            Black => Color::from_hex("#414868").unwrap(),
            Red => Color::from_hex("#f7768e").unwrap(),
            Green => Color::from_hex("#73daca").unwrap(),
            Yellow => Color::from_hex("#e0af68").unwrap(),
            Blue => Color::from_hex("#7aa2f7").unwrap(),
            Magenta => Color::from_hex("#bb9af7").unwrap(),
            Cyan => Color::from_hex("#7dcfff").unwrap(),
            White => Color::from_hex("#c0caf5").unwrap(),
            BrightBlack => Color::from_hex("#6a7099").unwrap(),
            BrightRed => Color::from_hex("#ff9999").unwrap(),
            BrightGreen => Color::from_hex("#b9e986").unwrap(),
            BrightYellow => Color::from_hex("#f4e070").unwrap(),
            BrightBlue => Color::from_hex("#9cc1ff").unwrap(),
            BrightMagenta => Color::from_hex("#d6b4ff").unwrap(),
            BrightCyan => Color::from_hex("#a3e6ff").unwrap(),
            BrightWhite => Color::from_hex("#e0e6ff").unwrap(),
            Foreground => Color::from_hex("#c0caf5").unwrap(),
            Background => Color::from_hex("#24283b").unwrap(),
            Cursor => Color::from_hex("#c0caf5").unwrap(),
            _ => Color::from_hex("#c0caf5").unwrap(),
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
        let mut state = TerminalState::new(80, 24, 1000);
        state.process_bytes(b"hello world\n");
        state.process_bytes(b"\x1b[31mred\x1b[0m\n");
        // Feed an OSC 7 sequence — the side parser should accept it
        // without panicking even though nothing reads the state yet.
        state.process_bytes(b"\x1b]7;file://localhost/tmp\x07");
    }

    #[test]
    fn scrollback_holds_history_and_offset_tracks_scrolling() {
        let mut state = TerminalState::new(80, 24, 1000);
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
        let grid = state.grid_snapshot(Color::default(), Color::default());
        assert_eq!(grid.display_offset, 0);
    }

    /// Regression test for the "scrolled view renders black" bug:
    /// display_iter points are grid coords where history rows are
    /// NEGATIVE lines; the snapshot must convert to viewport rows.
    #[test]
    fn scrolled_snapshot_shows_history_content() {
        let mut state = TerminalState::new(80, 24, 1000);
        for i in 0..30 {
            state.process_bytes(format!("line {}\r\n", i).as_bytes());
        }

        // Tailing: 31 logical rows, viewport shows rows 7.. => top = "line 7".
        let grid = state.grid_snapshot(Color::default(), Color::default());
        let top: String = (0..7).map(|c| grid.get(0, c).character).collect();
        assert_eq!(top.trim_end(), "line 7");

        // Scroll 7 up: top of the viewport must show "line 0" — before
        // the coordinate fix this row came back blank.
        state.scroll_lines(7);
        let grid = state.grid_snapshot(Color::default(), Color::default());
        let top: String = (0..7).map(|c| grid.get(0, c).character).collect();
        assert_eq!(top.trim_end(), "line 0");
        // The live cursor is below the scrolled viewport — hidden.
        assert_eq!(grid.cursor, None);
    }

    #[test]
    fn executing_phase_gates_on_osc_133() {
        let mut state = TerminalState::new(80, 24, 100);
        assert!(!state.is_executing(), "no integration yet");
        state.process_bytes(b"\x1b]133;A\x07");
        assert!(!state.is_executing(), "at prompt");
        state.process_bytes(b"\x1b]133;C\x07");
        assert!(state.is_executing(), "command running");
        state.process_bytes(b"\x1b]133;D;0\x07");
        assert!(!state.is_executing(), "command finished");
    }

    #[test]
    fn zero_scrollback_keeps_offset_pinned() {
        let mut state = TerminalState::new(80, 24, 0);
        for i in 0..50 {
            state.process_bytes(format!("line {}\r\n", i).as_bytes());
        }
        state.scroll_lines(10);
        assert_eq!(state.display_offset(), 0, "no history to scroll into");
    }
}
