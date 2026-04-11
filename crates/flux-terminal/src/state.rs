//! Wraps alacritty_terminal::Term with a clean interface.
//!
//! Feeds PTY bytes through the vte parser into Term, then converts
//! the grid to a RenderGrid for the renderer.

use std::sync::mpsc;

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::Config as TermConfig;
use alacritty_terminal::term::Term;
use alacritty_terminal::vte;
use flux_types::{CellData, CellFlags, Color, RenderGrid};

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
    event_rx: mpsc::Receiver<TermEvent>,
    cols: usize,
    rows: usize,
}

impl TerminalState {
    /// Create a new terminal state with the given dimensions.
    pub fn new(cols: usize, rows: usize) -> Self {
        let (tx, rx) = mpsc::channel();
        let event_proxy = EventProxy { tx };

        let config = TermConfig::default();
        let dims = TermDimensions { cols, rows };
        let term = Term::new(config, &dims, event_proxy);
        let parser = vte::ansi::Processor::new();

        Self {
            term,
            parser,
            event_rx: rx,
            cols,
            rows,
        }
    }

    /// Feed raw PTY output bytes into the terminal parser.
    pub fn process_bytes(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }

    /// Drain any events from alacritty_terminal (PtyWrite, Bell, Title).
    pub fn drain_events(&self) -> Vec<TermEvent> {
        let mut events = Vec::new();
        while let Ok(event) = self.event_rx.try_recv() {
            events.push(event);
        }
        events
    }

    /// Convert the current terminal grid to a RenderGrid for rendering.
    pub fn render_grid(&self, fg_default: Color, bg_default: Color) -> RenderGrid {
        let content = self.term.renderable_content();
        let mut grid = RenderGrid::new(self.cols, self.rows);

        // Set cursor position
        let cursor_point = content.cursor.point;
        let cursor_col = cursor_point.column.0;
        let cursor_row = cursor_point.line.0 as usize;
        if cursor_col < self.cols && cursor_row < self.rows {
            grid.cursor = Some((cursor_col, cursor_row));
        }

        for cell in content.display_iter {
            let col = cell.point.column.0;
            let row = cell.point.line.0 as usize;

            if col >= self.cols || row >= self.rows {
                continue;
            }

            let fg = self.convert_color(cell.fg, &fg_default);
            let bg = self.convert_color(cell.bg, &bg_default);

            let mut flags = CellFlags::empty();
            if cell.flags.contains(alacritty_terminal::term::cell::Flags::BOLD) {
                flags |= CellFlags::BOLD;
            }
            if cell.flags.contains(alacritty_terminal::term::cell::Flags::ITALIC) {
                flags |= CellFlags::ITALIC;
            }
            if cell.flags.contains(alacritty_terminal::term::cell::Flags::UNDERLINE) {
                flags |= CellFlags::UNDERLINE;
            }
            if cell.flags.contains(alacritty_terminal::term::cell::Flags::HIDDEN) {
                flags |= CellFlags::HIDDEN;
            }
            if cell.flags.contains(alacritty_terminal::term::cell::Flags::DIM_BOLD) {
                flags |= CellFlags::DIM;
            }
            if cell.flags.contains(alacritty_terminal::term::cell::Flags::WIDE_CHAR) {
                flags |= CellFlags::WIDE_CHAR;
            }

            grid.set(row, col, CellData {
                character: cell.c,
                fg,
                bg,
                flags,
            });
        }

        grid
    }

    /// Resize the terminal grid.
    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.cols = cols;
        self.rows = rows;
        let dims = TermDimensions { cols, rows };
        self.term.resize(dims);
    }

    /// Convert an alacritty color to our Color type.
    fn convert_color(&self, color: alacritty_terminal::vte::ansi::Color, default: &Color) -> Color {
        match color {
            alacritty_terminal::vte::ansi::Color::Named(named) => {
                self.named_color(named)
            }
            alacritty_terminal::vte::ansi::Color::Spec(rgb) => {
                Color::from_rgb(rgb.r, rgb.g, rgb.b)
            }
            alacritty_terminal::vte::ansi::Color::Indexed(idx) => {
                self.indexed_color(idx)
            }
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
